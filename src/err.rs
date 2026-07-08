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

//! GIF error types and string lookup — reimplementation of gif_err.c.

use crate::c_types::{
    D_GIF_ERR_CLOSE_FAILED, D_GIF_ERR_DATA_TOO_BIG, D_GIF_ERR_EOF_TOO_SOON, D_GIF_ERR_IMAGE_DEFECT,
    D_GIF_ERR_NOT_ENOUGH_MEM, D_GIF_ERR_NOT_GIF_FILE, D_GIF_ERR_NOT_READABLE,
    D_GIF_ERR_NO_COLOR_MAP, D_GIF_ERR_NO_IMAG_DSCR, D_GIF_ERR_NO_SCRN_DSCR, D_GIF_ERR_OPEN_FAILED,
    D_GIF_ERR_READ_FAILED, D_GIF_ERR_WRONG_RECORD, E_GIF_ERR_CLOSE_FAILED, E_GIF_ERR_DATA_TOO_BIG,
    E_GIF_ERR_DISK_IS_FULL, E_GIF_ERR_HAS_IMAG_DSCR, E_GIF_ERR_HAS_SCRN_DSCR,
    E_GIF_ERR_NOT_ENOUGH_MEM, E_GIF_ERR_NOT_WRITEABLE, E_GIF_ERR_NO_COLOR_MAP,
    E_GIF_ERR_OPEN_FAILED, E_GIF_ERR_WRITE_FAILED,
};
use core::ffi::c_int;
use core::fmt;

// ---------------------------------------------------------------------------
//  GifError enum
// ---------------------------------------------------------------------------

/// Typed GIF error — maps 1:1 to the `D_GIF_ERR_*` / `E_GIF_ERR_*` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GifError {
    // Decoder errors (D_GIF_ERR_*)
    DOpenFailed = D_GIF_ERR_OPEN_FAILED,
    DReadFailed = D_GIF_ERR_READ_FAILED,
    DNotGifFile = D_GIF_ERR_NOT_GIF_FILE,
    DNoScreenDesc = D_GIF_ERR_NO_SCRN_DSCR,
    DNoImageDesc = D_GIF_ERR_NO_IMAG_DSCR,
    DNoColorMap = D_GIF_ERR_NO_COLOR_MAP,
    DWrongRecord = D_GIF_ERR_WRONG_RECORD,
    DDataTooBig = D_GIF_ERR_DATA_TOO_BIG,
    DNotEnoughMem = D_GIF_ERR_NOT_ENOUGH_MEM,
    DCloseFailed = D_GIF_ERR_CLOSE_FAILED,
    DNotReadable = D_GIF_ERR_NOT_READABLE,
    DImageDefect = D_GIF_ERR_IMAGE_DEFECT,
    DEofTooSoon = D_GIF_ERR_EOF_TOO_SOON,
    // Encoder errors (E_GIF_ERR_*)
    EOpenFailed = E_GIF_ERR_OPEN_FAILED,
    EWriteFailed = E_GIF_ERR_WRITE_FAILED,
    EHasScreenDesc = E_GIF_ERR_HAS_SCRN_DSCR,
    EHasImageDesc = E_GIF_ERR_HAS_IMAG_DSCR,
    ENoColorMap = E_GIF_ERR_NO_COLOR_MAP,
    EDataTooBig = E_GIF_ERR_DATA_TOO_BIG,
    ENotEnoughMem = E_GIF_ERR_NOT_ENOUGH_MEM,
    EDiskIsFull = E_GIF_ERR_DISK_IS_FULL,
    ECloseFailed = E_GIF_ERR_CLOSE_FAILED,
    ENotWriteable = E_GIF_ERR_NOT_WRITEABLE,
}

impl GifError {
    /// Convert a `D_GIF_ERR_*` / `E_GIF_ERR_*` integer constant to a
    /// `GifError` variant. Returns `None` for unrecognized codes.
    pub fn from_error_code(code: c_int) -> Option<Self> {
        match code {
            D_GIF_ERR_OPEN_FAILED => Some(GifError::DOpenFailed),
            D_GIF_ERR_READ_FAILED => Some(GifError::DReadFailed),
            D_GIF_ERR_NOT_GIF_FILE => Some(GifError::DNotGifFile),
            D_GIF_ERR_NO_SCRN_DSCR => Some(GifError::DNoScreenDesc),
            D_GIF_ERR_NO_IMAG_DSCR => Some(GifError::DNoImageDesc),
            D_GIF_ERR_NO_COLOR_MAP => Some(GifError::DNoColorMap),
            D_GIF_ERR_WRONG_RECORD => Some(GifError::DWrongRecord),
            D_GIF_ERR_DATA_TOO_BIG => Some(GifError::DDataTooBig),
            D_GIF_ERR_NOT_ENOUGH_MEM => Some(GifError::DNotEnoughMem),
            D_GIF_ERR_CLOSE_FAILED => Some(GifError::DCloseFailed),
            D_GIF_ERR_NOT_READABLE => Some(GifError::DNotReadable),
            D_GIF_ERR_IMAGE_DEFECT => Some(GifError::DImageDefect),
            D_GIF_ERR_EOF_TOO_SOON => Some(GifError::DEofTooSoon),
            E_GIF_ERR_OPEN_FAILED => Some(GifError::EOpenFailed),
            E_GIF_ERR_WRITE_FAILED => Some(GifError::EWriteFailed),
            E_GIF_ERR_HAS_SCRN_DSCR => Some(GifError::EHasScreenDesc),
            E_GIF_ERR_HAS_IMAG_DSCR => Some(GifError::EHasImageDesc),
            E_GIF_ERR_NO_COLOR_MAP => Some(GifError::ENoColorMap),
            E_GIF_ERR_DATA_TOO_BIG => Some(GifError::EDataTooBig),
            E_GIF_ERR_NOT_ENOUGH_MEM => Some(GifError::ENotEnoughMem),
            E_GIF_ERR_DISK_IS_FULL => Some(GifError::EDiskIsFull),
            E_GIF_ERR_CLOSE_FAILED => Some(GifError::ECloseFailed),
            E_GIF_ERR_NOT_WRITEABLE => Some(GifError::ENotWriteable),
            _ => None,
        }
    }

    /// NUL-terminated error description matching the C `GifErrorString` output.
    pub fn to_error_string(self) -> &'static core::ffi::CStr {
        match self {
            GifError::DOpenFailed => c"Failed to open given file",
            GifError::DReadFailed => c"Failed to read from given file",
            GifError::DNotGifFile => c"Data is not in GIF format",
            GifError::DNoScreenDesc => c"No screen descriptor detected",
            GifError::DNoImageDesc => c"No Image Descriptor detected",
            GifError::DNoColorMap => c"Neither global nor local color map",
            GifError::DWrongRecord => c"Wrong record type detected",
            GifError::DDataTooBig => c"Number of pixels bigger than width * height",
            GifError::DNotEnoughMem => c"Failed to allocate required memory",
            GifError::DCloseFailed => c"Failed to close given file",
            GifError::DNotReadable => c"Given file was not opened for read",
            GifError::DImageDefect => c"Image is defective, decoding aborted",
            GifError::DEofTooSoon => c"Image EOF detected before image complete",
            GifError::EOpenFailed => c"Failed to open given file",
            GifError::EWriteFailed => c"Failed to write to given file",
            GifError::EHasScreenDesc => c"Screen descriptor has already been set",
            GifError::EHasImageDesc => c"Image descriptor is still active",
            GifError::ENoColorMap => c"Neither global nor local color map",
            GifError::EDataTooBig => c"Number of pixels bigger than width * height",
            GifError::ENotEnoughMem => c"Failed to allocate required memory",
            GifError::EDiskIsFull => c"Write failed (disk full?)",
            GifError::ECloseFailed => c"Failed to close given file",
            GifError::ENotWriteable => c"Given file was not opened for write",
        }
    }
}

impl fmt::Display for GifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // to_str() is infallible here — all literals are valid UTF-8.
        f.write_str(self.to_error_string().to_str().unwrap())
    }
}
