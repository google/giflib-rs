#![forbid(unsafe_code)]
// Copyright (c) 1997-2025 Eric S. Raymond
// Copyright (c) 2026 Google LLC
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! Shared private state for the GIF encoder and decoder.
//!
//! `GifFilePrivateType` is the Rust equivalent of the C `GifFilePrivateType`
//! struct from `gif_lib_private.h`. It is shared between encoder and decoder,
//! stored in `GifFileType.Private`.
//!
//! The struct is split into sub-structs to enable split-borrowing:
//! - `IoState`: file handles and callbacks (enum — read XOR write)
//! - `LzwState`: LZW codec state, buffers, and hash table

use crate::c_types::{GifByteType, GifFileType, GifPrefixType, ReadCallback, WriteCallback};
use crate::encoder_lzw::GifHashTableType;
use crate::private_c_types::LZ_MAX_CODE;
use core::ffi::c_int;
use std::io::{Read, Write};

// ---------------------------------------------------------------------------
//  I/O State
// ---------------------------------------------------------------------------

/// File I/O state — a GIF handle is either reading or writing, never both.
#[derive(Debug)]
pub(crate) enum IoState {
    FileRead { file: std::fs::File },
    CallbackRead { read_fn: ReadCallback },
    FileWrite { file: std::fs::File },
    CallbackWrite { write_fn: WriteCallback },
}

impl IoState {
    /// Returns `true` if this handle was opened for reading.
    pub(crate) fn is_readable(&self) -> bool {
        matches!(self, IoState::FileRead { .. } | IoState::CallbackRead { .. })
    }

    /// Returns `true` if this handle was opened for writing.
    pub(crate) fn is_writable(&self) -> bool {
        matches!(self, IoState::FileWrite { .. } | IoState::CallbackWrite { .. })
    }

    /// Read from the I/O source into `buf`. Returns number of bytes read.
    ///
    /// For callback-based I/O, `gif_ptr` is passed through to the C callback.
    /// For file-based I/O, loops to handle partial reads (matching `fread` behavior).
    pub(crate) fn read(&mut self, gif_ptr: *mut GifFileType, buf: &mut [u8]) -> usize {
        match self {
            IoState::CallbackRead { read_fn } => read_fn.read(gif_ptr, buf) as usize,
            IoState::FileRead { file, .. } => {
                let mut total = 0usize;
                while total < buf.len() {
                    match file.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => total += n,
                        Err(_) => break,
                    }
                }
                total
            }
            _ => 0, // write handle — caller error
        }
    }

    /// Write `buf` to the I/O sink. Returns number of bytes written.
    ///
    /// For callback-based I/O, `gif_ptr` is passed through to the C callback.
    /// For file-based I/O, loops to handle partial writes (matching `fwrite` behavior).
    pub(crate) fn write(&mut self, gif_ptr: *mut GifFileType, buf: &[u8]) -> usize {
        match self {
            IoState::CallbackWrite { write_fn } => write_fn.write(gif_ptr, buf) as usize,
            IoState::FileWrite { file, .. } => {
                let mut total = 0usize;
                while total < buf.len() {
                    match file.write(&buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => total += n,
                        Err(_) => break,
                    }
                }
                total
            }
            _ => 0, // read handle — caller error
        }
    }
}

// ---------------------------------------------------------------------------
//  LZW Codec State
// ---------------------------------------------------------------------------

/// LZW codec state shared between encoder and decoder.
///
/// Includes the bit-packing shift register, code tables (prefix/suffix/stack),
/// the block I/O buffer (`buf`), and the encoder's hash table.
#[derive(Debug)]
pub(crate) struct LzwState {
    pub(crate) bits_per_pixel: c_int,
    pub(crate) clear_code: c_int,
    pub(crate) eof_code: c_int,
    pub(crate) running_code: c_int,
    pub(crate) running_bits: c_int,
    pub(crate) max_code1: c_int,
    pub(crate) last_code: c_int,
    pub(crate) crnt_code: c_int,
    pub(crate) stack_ptr: c_int,
    pub(crate) crnt_shift_state: c_int,
    pub(crate) crnt_shift_dword: u64,
    /// Pascal-string block buffer: buf[0] = remaining count, data at buf[1..].
    pub(crate) buf: [GifByteType; 256],
    /// Boxed to avoid placing ~4 KB on the call stack during construction.
    pub(crate) stack: Box<[GifByteType; LZ_MAX_CODE + 1]>,
    /// Boxed to avoid placing ~4 KB on the call stack during construction.
    pub(crate) suffix: Box<[GifByteType; LZ_MAX_CODE + 1]>,
    /// Boxed to avoid placing ~8 KB on the call stack during construction.
    pub(crate) prefix: Box<[GifPrefixType; LZ_MAX_CODE + 1]>,
    pub(crate) hash_table: Option<Box<GifHashTableType>>,
}

impl Default for LzwState {
    fn default() -> Self {
        // Large arrays are boxed to keep the stack frame small (~300 bytes
        // instead of ~16 KB). This is critical for ARM targets where the
        // DecodeSafeStackTest exercises decoding on a 16 KB thread stack.
        Self {
            bits_per_pixel: 0,
            clear_code: 0,
            eof_code: 0,
            running_code: 0,
            running_bits: 0,
            max_code1: 0,
            last_code: 0,
            crnt_code: 0,
            stack_ptr: 0,
            crnt_shift_state: 0,
            crnt_shift_dword: 0,
            buf: [0; 256],
            stack: Box::new([0; LZ_MAX_CODE + 1]),
            suffix: Box::new([0; LZ_MAX_CODE + 1]),
            prefix: Box::new([0; LZ_MAX_CODE + 1]),
            hash_table: None,
        }
    }
}

// ---------------------------------------------------------------------------
//  GifFilePrivateType
// ---------------------------------------------------------------------------

/// Internal GIF private state. Opaque to callers (stored in `GifFileType.Private`).
/// Shared between encoder and decoder, matching the C code's single struct.
#[derive(Debug)]
pub struct GifFilePrivateType {
    pub(crate) pixel_count: usize,
    pub(crate) gif89: bool,
    /// Encoder-only: has the screen descriptor been written?
    pub(crate) screen_desc_written: bool,
    /// Encoder-only: are we currently inside an image?
    pub(crate) in_image: bool,
    pub(crate) io: IoState,
    pub(crate) lzw: LzwState,
}

impl GifFilePrivateType {
    /// Create a new `GifFilePrivateType` with the given I/O state.
    pub(crate) fn new(io: IoState) -> Self {
        Self {
            pixel_count: 0,
            gif89: false,
            screen_desc_written: false,
            in_image: false,
            io,
            lzw: LzwState::default(),
        }
    }
}
