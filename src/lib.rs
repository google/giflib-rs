// Crate root for the giflib Rust reimplementation.
// Bug-for-bug compatible drop-in replacement for the C libgif decoder.
//
// SPDX-License-Identifier: MIT

#[path = "../c_types_gen.rs"]
#[allow(
    dead_code,
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    clippy::all,
    improper_ctypes
)]
pub(crate) mod c_types_gen;

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
