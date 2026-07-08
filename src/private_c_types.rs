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

/// Constants from gif_lib_private.h — not exposed in the public C header.
use core::ffi::c_int;

// ---- Record markers (byte-level) ----
pub const EXTENSION_INTRODUCER: u8 = 0x21;
pub const DESCRIPTOR_INTRODUCER: u8 = 0x2C;
pub const TERMINATOR_INTRODUCER: u8 = 0x3B;

// ---- LZW constants ----
pub const LZ_MAX_CODE: usize = 4095;
pub const LZ_BITS: c_int = 12;

pub const FLUSH_OUTPUT: c_int = 4096;
pub const FIRST_CODE: c_int = 4097;
pub const NO_SUCH_CODE: c_int = 4098;
