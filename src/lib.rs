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

//! Giflib Rust reimplementation.
//! Bug-for-bug compatible drop-in replacement for the C libgif decoder.

#[allow(
    dead_code,
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    clippy::all,
    improper_ctypes
)]
pub(crate) mod c_types_gen {
  include!(concat!(env!("OUT_DIR"), "/c_types_gen.rs"));
}

pub(crate) mod c_types;

pub(crate) mod decoder;
pub(crate) mod decoder_lzw;
pub(crate) mod encoder;
pub(crate) mod encoder_lzw;
pub(crate) mod err;
pub(crate) mod ffi;
pub(crate) mod font;
pub(crate) mod helpers;
pub(crate) mod private;
pub(crate) mod private_c_types;
pub(crate) mod quantize;

// Re-export public C-API symbols so the c_library_from_rust_signatures_test
// can find them at the crate root.
pub use c_types::*;
pub use ffi::*;
