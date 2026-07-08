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

//! LZW decompression internals — port of the decompression half of dgif_lib.c.
//!
//! These functions operate on `GifFileType` / `GifFilePrivateType` state
//! and are called by the higher-level decoder functions.

use crate::c_types::{GifByteType, GifFileType, GifPixelType, GifPrefixType};

use crate::err::GifError;
use crate::private_c_types::{LZ_BITS, LZ_MAX_CODE, NO_SUCH_CODE};
use core::ffi::c_int;

/// Matches DGifSetupDecompress in C.
pub(crate) fn dgif_setup_decompress(gif: &mut GifFileType) -> Result<(), GifError> {
    // GOOGLE MODIFICATION: initialize to invalid in case read fails
    let mut cs_buf = [0u8; 1];
    gif.read_exact(&mut cs_buf)?;
    let code_size = cs_buf[0];
    let bits_per_pixel = code_size as c_int;

    // Severely malformed GIF
    if bits_per_pixel > 8 {
        return Err(GifError::DReadFailed);
    }

    gif.Private.lzw.buf[0] = 0; // Input buffer empty
    gif.Private.lzw.bits_per_pixel = bits_per_pixel;
    gif.Private.lzw.clear_code = 1 << bits_per_pixel;
    gif.Private.lzw.eof_code = gif.Private.lzw.clear_code + 1;
    gif.Private.lzw.running_code = gif.Private.lzw.eof_code + 1;
    gif.Private.lzw.running_bits = bits_per_pixel + 1;
    gif.Private.lzw.max_code1 = 1 << gif.Private.lzw.running_bits;
    gif.Private.lzw.stack_ptr = 0;
    gif.Private.lzw.last_code = NO_SUCH_CODE;
    gif.Private.lzw.crnt_shift_state = 0;
    gif.Private.lzw.crnt_shift_dword = 0;

    gif.Private.lzw.prefix.fill(NO_SUCH_CODE as GifPrefixType);

    Ok(())
}

/// Matches DGifDecompressLine in C.
pub(crate) fn dgif_decompress_line(
    gif: &mut GifFileType,
    line: &mut [GifPixelType],
) -> Result<(), GifError> {
    let line_len = line.len();
    let mut i: usize = 0;
    let mut stack_ptr = gif.Private.lzw.stack_ptr;
    let eof_code = gif.Private.lzw.eof_code;
    let clear_code = gif.Private.lzw.clear_code;
    let mut last_code = gif.Private.lzw.last_code;

    if stack_ptr > LZ_MAX_CODE as c_int {
        return Err(GifError::DImageDefect);
    }

    // Pop any leftover pixels from the stack
    while stack_ptr != 0 && i < line_len {
        stack_ptr -= 1;
        line[i] = gif.Private.lzw.stack[stack_ptr as usize];
        i += 1;
    }

    while i < line_len {
        let mut crnt_code: c_int = 0;
        dgif_decompress_input(gif, &mut crnt_code)?;

        if crnt_code == eof_code {
            // GOOGLE MODIFICATION: zero remaining pixels on early EOF
            line[i..].fill(0);
            return Err(GifError::DEofTooSoon);
        } else if crnt_code == clear_code {
            // Reset the table
            gif.Private.lzw.prefix.fill(NO_SUCH_CODE as GifPrefixType);
            gif.Private.lzw.running_code = gif.Private.lzw.eof_code + 1;
            gif.Private.lzw.running_bits = gif.Private.lzw.bits_per_pixel + 1;
            gif.Private.lzw.max_code1 = 1 << gif.Private.lzw.running_bits;
            last_code = NO_SUCH_CODE;
            gif.Private.lzw.last_code = NO_SUCH_CODE;
        } else {
            let crnt_prefix: c_int;
            if crnt_code < clear_code {
                // Literal pixel
                line[i] = crnt_code as GifPixelType;
                i += 1;
            } else {
                // Code chain to trace
                if gif.Private.lzw.prefix[crnt_code as usize] == NO_SUCH_CODE as GifPrefixType {
                    crnt_prefix = last_code;
                    let rc_idx = gif.Private.lzw.running_code - 2;
                    let trace_from = if crnt_code == gif.Private.lzw.running_code - 2 {
                        last_code
                    } else {
                        crnt_code
                    };
                    let pc = dgif_get_prefix_char(&*gif.Private.lzw.prefix, trace_from, clear_code);
                    gif.Private.lzw.suffix[rc_idx as usize] = pc as GifByteType;
                    gif.Private.lzw.stack[stack_ptr as usize] = pc as GifByteType;
                    stack_ptr += 1;
                } else {
                    crnt_prefix = crnt_code;
                }

                // Trace the chain
                let mut cp = crnt_prefix;
                while (stack_ptr as usize) < LZ_MAX_CODE
                    && cp > clear_code
                    && cp <= LZ_MAX_CODE as c_int
                {
                    gif.Private.lzw.stack[stack_ptr as usize] = gif.Private.lzw.suffix[cp as usize];
                    stack_ptr += 1;
                    cp = gif.Private.lzw.prefix[cp as usize] as c_int;
                }
                if stack_ptr as usize >= LZ_MAX_CODE || cp > LZ_MAX_CODE as c_int {
                    return Err(GifError::DImageDefect);
                }
                // Push the final pixel
                gif.Private.lzw.stack[stack_ptr as usize] = cp as GifByteType;
                stack_ptr += 1;

                // Pop stack to output
                while stack_ptr != 0 && i < line_len {
                    stack_ptr -= 1;
                    line[i] = gif.Private.lzw.stack[stack_ptr as usize];
                    i += 1;
                }
            }

            // Update the prefix/suffix table
            let rc_idx = gif.Private.lzw.running_code - 2;
            if last_code != NO_SUCH_CODE
                && rc_idx >= 0
                && (rc_idx as usize) < LZ_MAX_CODE + 1
                && gif.Private.lzw.prefix[rc_idx as usize] == NO_SUCH_CODE as GifPrefixType
            {
                gif.Private.lzw.prefix[rc_idx as usize] = last_code as GifPrefixType;

                let trace_from = if crnt_code == gif.Private.lzw.running_code - 2 {
                    last_code
                } else {
                    crnt_code
                };
                gif.Private.lzw.suffix[rc_idx as usize] =
                    dgif_get_prefix_char(&*gif.Private.lzw.prefix, trace_from, clear_code)
                        as GifByteType;
            }
            last_code = crnt_code;
        }
    }

    gif.Private.lzw.last_code = last_code;
    gif.Private.lzw.stack_ptr = stack_ptr;
    Ok(())
}

/// Trace prefix chain to find the root pixel. Matches DGifGetPrefixChar in C.
#[inline(always)]
fn dgif_get_prefix_char(prefix: &[GifPrefixType], mut code: c_int, clear_code: c_int) -> c_int {
    let mut i = 0;
    while code > clear_code && i <= LZ_MAX_CODE {
        if code > LZ_MAX_CODE as c_int {
            return NO_SUCH_CODE;
        }
        code = prefix[code as usize] as c_int;
        i += 1;
    }
    code
}

/// Decompress one LZW code from the bit stream. Matches DGifDecompressInput in C.
///
/// Marked `#[inline(always)]` — in the C version this is a `static` function
/// that the compiler inlines into `DGifDecompressLine`. Without inlining, the
/// `&mut GifFileType` parameter creates an optimization barrier: the compiler
/// must assume the callee could modify any field, preventing it from keeping
/// array base pointers in registers across the call.
#[inline(always)]
pub(crate) fn dgif_decompress_input(
    gif: &mut GifFileType,
    code: &mut c_int,
) -> Result<(), GifError> {
    let running_bits = gif.Private.lzw.running_bits;

    if running_bits > LZ_BITS {
        return Err(GifError::DImageDefect);
    }

    while gif.Private.lzw.crnt_shift_state < running_bits {
        let mut next_byte: GifByteType = 0;
        dgif_buffered_input(gif, &mut next_byte)?;

        gif.Private.lzw.crnt_shift_dword |=
            (next_byte as u64) << gif.Private.lzw.crnt_shift_state as u64;
        gif.Private.lzw.crnt_shift_state += 8;
    }

    // Compute mask directly instead of table lookup: avoids bounds check.
    // running_bits is guaranteed <= LZ_BITS (12) by the check above.
    let code_mask = (1u64 << running_bits as u64) - 1;
    *code = (gif.Private.lzw.crnt_shift_dword & code_mask) as c_int;
    gif.Private.lzw.crnt_shift_dword >>= running_bits as u64;
    gif.Private.lzw.crnt_shift_state -= running_bits;

    // Increment running code and potentially increase bit width.
    // Exact match of the C pre-increment-then-compare pattern.
    if gif.Private.lzw.running_code < LZ_MAX_CODE as c_int + 2 {
        gif.Private.lzw.running_code += 1;
        if gif.Private.lzw.running_code > gif.Private.lzw.max_code1 && running_bits < LZ_BITS {
            gif.Private.lzw.max_code1 <<= 1;
            gif.Private.lzw.running_bits += 1;
        }
    }

    Ok(())
}

/// Read one buffered byte from the input stream. Matches DGifBufferedInput in C.
///
/// The block buffer uses Pascal-string layout: `buf[0]` = remaining count,
/// data lives at `buf[1..]`.  After reading a new block, `buf[1]` initially
/// holds the *first data byte* that gets returned, but is then overwritten
/// with `2` — the index of the *next* byte to read.  On subsequent calls
/// `buf[1]` acts as a read-position cursor into the block.
///
/// This dual-use of `buf[1]` mirrors the C implementation.
///
/// Marked `#[inline(always)]` because in the C version this is a `static`
/// function that the compiler inlines into `DGifDecompressInput`. The fast
/// path (buffer not empty) is a simple array read + index bump that should
/// compile to a handful of instructions when inlined.
#[inline(always)]
fn dgif_buffered_input(gif: &mut GifFileType, next_byte: &mut GifByteType) -> Result<(), GifError> {
    /// Index within `buf` that stores the next-read position after initial read.
    const BUF_READ_POS: usize = 1;

    if gif.Private.lzw.buf[0] == 0 {
        // Cold path: buffer empty, need to refill from I/O.
        std::hint::cold_path();

        // Need to read a new data block
        let mut len_buf = [0u8; 1];
        gif.read_exact(&mut len_buf)?;
        gif.Private.lzw.buf[0] = len_buf[0];

        if gif.Private.lzw.buf[0] == 0 {
            return Err(GifError::DImageDefect);
        }

        let block_len = gif.Private.lzw.buf[0] as usize;
        let gif_ptr = gif as *mut GifFileType;
        if gif.Private.io.read(gif_ptr, &mut gif.Private.lzw.buf[1..1 + block_len]) != block_len {
            return Err(GifError::DReadFailed);
        }
        *next_byte = gif.Private.lzw.buf[BUF_READ_POS];
        gif.Private.lzw.buf[BUF_READ_POS] = 2; // Next read position
        gif.Private.lzw.buf[0] -= 1;
    } else {
        *next_byte = gif.Private.lzw.buf[gif.Private.lzw.buf[BUF_READ_POS] as usize];
        gif.Private.lzw.buf[BUF_READ_POS] = gif.Private.lzw.buf[BUF_READ_POS].wrapping_add(1);
        gif.Private.lzw.buf[0] -= 1;
    }

    Ok(())
}
