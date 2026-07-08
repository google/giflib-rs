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

//! LZW compression internals + GIF hash table — port of egif_lib.c + gif_hash.c.
//!
//! These functions operate on `GifFileType` / `GifFilePrivateType` state
//! and are called by the higher-level encoder functions.

use crate::c_types::{GifFileType, GifPixelType};
use crate::err::GifError;
use crate::private_c_types::{FIRST_CODE, FLUSH_OUTPUT, LZ_MAX_CODE};
use core::ffi::c_int;

// ---------------------------------------------------------------------------
//  Hash table constants (from gif_hash.h)
// ---------------------------------------------------------------------------

pub(crate) const HT_SIZE: usize = 8192; // 12bits = 4096 or twice as big!
const HT_KEY_MASK: u32 = 0x1FFF; // 13bits keys

// The 32 bits of the long are divided into two parts for the key & code:
// 1. The code is 12 bits as our compression algorithm is limited to 12bits
// 2. The key is 12 bits Prefix code + 8 bit new char or 20 bits.
// The key is the upper 20 bits.  The code is the lower 12.
fn ht_get_key(l: u32) -> u32 {
    l >> 12
}
fn ht_get_code(l: u32) -> i32 {
    (l & 0x0FFF) as i32
}
fn ht_put_key(l: u32) -> u32 {
    l << 12
}
fn ht_put_code(l: i32) -> u32 {
    (l as u32) & 0x0FFF
}

/// Generate a hash key from the given unique key.
/// The key is 20 bits: upper 12 = prefix code, lower 8 = new char.
fn key_item(item: u32) -> u32 {
    ((item >> 12) ^ item) & HT_KEY_MASK
}

// ---------------------------------------------------------------------------
//  GifHashTableType
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct GifHashTableType {
    pub h_table: [u32; HT_SIZE],
}

impl Default for GifHashTableType {
    fn default() -> Self {
        let mut ht = Self { h_table: [0; HT_SIZE] };
        ht.clear();
        ht
    }
}

impl GifHashTableType {
    /// Clear the hash table to an empty state.
    /// Matches _ClearHashTable in C.
    pub fn clear(&mut self) {
        self.h_table.fill(0xFFFFFFFF);
    }

    /// Insert a new item into the hash table.
    /// The data is assumed to be new (not already present).
    /// Matches _InsertHashTable in C.
    pub fn insert(&mut self, key: u32, code: i32) {
        let mut h_key = key_item(key) as usize;

        while ht_get_key(self.h_table[h_key]) != 0xFFFFF {
            h_key = (h_key + 1) & (HT_KEY_MASK as usize);
        }
        self.h_table[h_key] = ht_put_key(key) | ht_put_code(code);
    }

    /// Test if the given key exists and return its code.
    /// Returns `Some(code)` if found, `None` if not.
    /// Matches _ExistsHashTable in C (returns -1 for not found).
    pub fn exists(&self, key: u32) -> Option<i32> {
        let mut h_key = key_item(key) as usize;

        loop {
            let ht_key = ht_get_key(self.h_table[h_key]);
            if ht_key == 0xFFFFF {
                return None;
            }
            if key == ht_key {
                return Some(ht_get_code(self.h_table[h_key]));
            }
            h_key = (h_key + 1) & (HT_KEY_MASK as usize);
        }
    }
}

// ---------------------------------------------------------------------------
//  Masks given codes to BitsPerPixel
// ---------------------------------------------------------------------------

static CODE_MASK: [GifPixelType; 9] = [0x00, 0x01, 0x03, 0x07, 0x0f, 0x1f, 0x3f, 0x7f, 0xff];

// ---------------------------------------------------------------------------
//  LZW compression setup
// ---------------------------------------------------------------------------

/// Matches EGifSetupCompress in C.
pub(crate) fn egif_setup_compress(gif: &mut GifFileType) -> Result<(), GifError> {
    // Test and see what color map to use, and from it # bits per pixel.
    let bits_per_pixel = if let Some(ref cm) = gif.Image.ColorMap {
        cm.BitsPerPixel
    } else if let Some(ref cm) = gif.SColorMap {
        cm.BitsPerPixel
    } else {
        return Err(GifError::ENoColorMap);
    };

    let bits_per_pixel = if bits_per_pixel < 2 { 2 } else { bits_per_pixel };

    // Write the code size to file.
    let buf = [bits_per_pixel as u8];
    gif.write_exact(&buf)?;

    gif.Private.lzw.buf[0] = 0; // Nothing was output yet.
    gif.Private.lzw.bits_per_pixel = bits_per_pixel;
    gif.Private.lzw.clear_code = 1 << bits_per_pixel;
    gif.Private.lzw.eof_code = gif.Private.lzw.clear_code + 1;
    gif.Private.lzw.running_code = gif.Private.lzw.eof_code + 1;
    gif.Private.lzw.running_bits = bits_per_pixel + 1; // Number of bits per code.
    gif.Private.lzw.max_code1 = 1 << gif.Private.lzw.running_bits; // Max. code + 1.
    gif.Private.lzw.crnt_code = FIRST_CODE; // Signal that this is first one!
    gif.Private.lzw.crnt_shift_state = 0;
    gif.Private.lzw.crnt_shift_dword = 0;

    // Allocate hash table on first use (decoder handles never need it).
    let ht = gif.Private.lzw.hash_table.get_or_insert_default();
    ht.clear();

    if egif_compress_output(gif, gif.Private.lzw.clear_code).is_err() {
        return Err(GifError::EDiskIsFull);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
//  LZW compression line
// ---------------------------------------------------------------------------

/// Matches EGifCompressLine in C.
pub(crate) fn egif_compress_line(
    gif: &mut GifFileType,
    line: &[GifPixelType],
) -> Result<(), GifError> {
    let line_len = line.len();
    let mut i = 0usize;

    let mut crnt_code: c_int = if gif.Private.lzw.crnt_code == FIRST_CODE {
        // It's first time!
        let c = line[i] as c_int;
        i += 1;
        c
    } else {
        gif.Private.lzw.crnt_code // Get last code in compression.
    };

    while i < line_len {
        // Decode LineLen items.
        let pixel = line[i]; // Get next pixel from stream.
        i += 1;

        // Form a new unique key to search hash table for the code
        // combines CrntCode as Prefix string with Pixel as postfix char.
        let new_key: u32 = ((crnt_code as u32) << 8) + pixel as u32;
        if let Some(new_code) = gif.Private.lzw.hash_table.as_ref().unwrap().exists(new_key) {
            // This Key is already there, or the string is old one,
            // so simple take new code as our CrntCode:
            crnt_code = new_code;
        } else {
            // Put it in hash table, output the prefix code, and
            // make our CrntCode equal to Pixel.
            egif_compress_output(gif, crnt_code)?;
            crnt_code = pixel as c_int;

            // If however the HashTable is full, we send a clear
            // first and clear the hash table.
            if gif.Private.lzw.running_code >= LZ_MAX_CODE as c_int {
                // Time to do some clearance:
                let clear_code = gif.Private.lzw.clear_code;
                egif_compress_output(gif, clear_code)?;
                gif.Private.lzw.running_code = gif.Private.lzw.eof_code + 1;
                gif.Private.lzw.running_bits = gif.Private.lzw.bits_per_pixel + 1;
                gif.Private.lzw.max_code1 = 1 << gif.Private.lzw.running_bits;
                gif.Private.lzw.hash_table.as_mut().unwrap().clear();
            } else {
                // Put this unique key with its relative Code in hash table:
                let rc = gif.Private.lzw.running_code;
                gif.Private.lzw.hash_table.as_mut().unwrap().insert(new_key, rc);
                gif.Private.lzw.running_code += 1;
            }
        }
    }

    // Preserve the current state of the compression algorithm:
    gif.Private.lzw.crnt_code = crnt_code;

    if gif.Private.pixel_count == 0 {
        // We are done - output last Code and flush output buffers:
        egif_compress_output(gif, crnt_code)?;
        let eof_code = gif.Private.lzw.eof_code;
        egif_compress_output(gif, eof_code)?;
        egif_compress_output(gif, FLUSH_OUTPUT)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
//  LZW compression output (bit packing)
// ---------------------------------------------------------------------------

/// Matches EGifCompressOutput in C.
fn egif_compress_output(gif: &mut GifFileType, code: c_int) -> Result<(), GifError> {
    let mut retval = Ok(());

    if code == FLUSH_OUTPUT {
        while gif.Private.lzw.crnt_shift_state > 0 {
            // Get Rid of what is left in DWord, and flush it.
            let byte = (gif.Private.lzw.crnt_shift_dword & 0xff) as c_int;
            if egif_buffered_output(gif, byte).is_err() {
                retval = Err(GifError::EDiskIsFull);
            }
            gif.Private.lzw.crnt_shift_dword >>= 8;
            gif.Private.lzw.crnt_shift_state -= 8;
        }
        gif.Private.lzw.crnt_shift_state = 0; // For next time.
        if egif_buffered_output(gif, FLUSH_OUTPUT).is_err() {
            retval = Err(GifError::EDiskIsFull);
        }
    } else {
        gif.Private.lzw.crnt_shift_dword |=
            (code as u64) << gif.Private.lzw.crnt_shift_state as u64;
        gif.Private.lzw.crnt_shift_state += gif.Private.lzw.running_bits;
        while gif.Private.lzw.crnt_shift_state >= 8 {
            // Dump out full bytes:
            let byte = (gif.Private.lzw.crnt_shift_dword & 0xff) as c_int;
            if egif_buffered_output(gif, byte).is_err() {
                retval = Err(GifError::EDiskIsFull);
            }
            gif.Private.lzw.crnt_shift_dword >>= 8;
            gif.Private.lzw.crnt_shift_state -= 8;
        }
    }

    // If code cannot fit into RunningBits bits, must raise its size. Note
    // however that codes above 4095 are used for special signaling.
    if gif.Private.lzw.running_code >= gif.Private.lzw.max_code1 && code <= 4095 {
        gif.Private.lzw.running_bits += 1;
        gif.Private.lzw.max_code1 = 1 << gif.Private.lzw.running_bits;
    }

    retval
}

// ---------------------------------------------------------------------------
//  Buffered output
// ---------------------------------------------------------------------------

/// Matches EGifBufferedOutput in C.
/// Buffers characters until 255 are ready, then outputs.
/// Pascal-string layout: buf[0] = count, data at buf[1..=255].
fn egif_buffered_output(gif: &mut GifFileType, c: c_int) -> Result<(), GifError> {
    let gif_ptr = gif as *mut GifFileType;

    if c == FLUSH_OUTPUT {
        // Flush everything out.
        let count = gif.Private.lzw.buf[0] as usize;
        if count != 0 {
            let written = gif.Private.io.write(gif_ptr, &gif.Private.lzw.buf[..count + 1]);
            if written != count + 1 {
                return Err(GifError::EWriteFailed);
            }
        }
        // Mark end of compressed data, by an empty block (see GIF doc):
        gif.Private.lzw.buf[0] = 0;
        let written = gif.Private.io.write(gif_ptr, &gif.Private.lzw.buf[..1]);
        if written != 1 {
            return Err(GifError::EWriteFailed);
        }
    } else {
        if gif.Private.lzw.buf[0] == 255 {
            // Dump out this buffer - it is full:
            let written = gif.Private.io.write(gif_ptr, &gif.Private.lzw.buf[..256]);
            if written != 256 {
                return Err(GifError::EWriteFailed);
            }
            gif.Private.lzw.buf[0] = 0;
        }
        gif.Private.lzw.buf[0] += 1;
        let idx = gif.Private.lzw.buf[0] as usize;
        gif.Private.lzw.buf[idx] = c as u8;
    }

    Ok(())
}

/// Get the code mask for pixel masking.
pub(crate) fn get_code_mask(bits_per_pixel: c_int) -> GifPixelType {
    CODE_MASK[bits_per_pixel as usize]
}
