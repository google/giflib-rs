#![allow(non_snake_case)]
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

//! FFI bridge — all #[no_mangle] extern "C" entry points live here.
//!
//! This is the ONLY module that interacts with GIF_FILE_TRACKER and
//! performs raw pointer manipulation across the FFI boundary.

use crate::c_types::{
    ColorMapObject, ExtensionBlock, GifByteType, GifColorType, GifFileType, GifImageDesc,
    GifPixelType, GifRecordType, GraphicsControlBlock, InputFunc, OutputFunc, ReadCallback,
    SavedImage, WriteCallback, D_GIF_ERR_CLOSE_FAILED, E_GIF_ERR_CLOSE_FAILED,
    E_GIF_ERR_NOT_WRITEABLE, E_GIF_SUCCEEDED, GIF87_STAMP, GIF_ERROR, GIF_OK,
};
use crate::decoder;
use crate::encoder;
use crate::err;
use crate::err::GifError;
use crate::font;
use crate::quantize;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr;
use safer_cffi::{CBufPtr, CStrRef, OwnedCBufPtr};

// ---------------------------------------------------------------------------
//  Open
// ---------------------------------------------------------------------------

/// DGifOpenFileName — open a GIF file by filename.
// SAFETY: Exports the canonical giflib C ABI symbol `DGifOpenFileName` (or `rs_DGifOpenFileName`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifOpenFileName(
    file_name: Option<CStrRef<'_>>,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    let Some(file_name) = file_name else {
        if let Some(error) = error {
            *error = GifError::DOpenFailed as c_int;
        };
        return None;
    };
    let c_str = file_name.to_c_str();
    let gif = decoder::dgif_open_file_name(c_str);
    match gif {
        Ok(gif) => Some(gif),
        Err(e) => {
            if let Some(error) = error {
                *error = e as c_int;
            }
            None
        }
    }
}

/// Helper to convert a raw file descriptor to a `std::fs::File`.
/// Takes ownership of the file descriptor.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `file_handle >= 0`, `file_handle` is a valid, open OS file descriptor, and exclusive
///   ownership of `file_handle` is transferred to this function (the caller must not close or
///   use `file_handle` after this call).
/// - Postcondition: If `file_handle >= 0`, ownership is wrapped in the returned `std::fs::File`
///   (or closed immediately if the platform cannot wrap it), whereas if `file_handle < 0`,
///   `None` is returned without touching any OS descriptor.
unsafe fn make_file(file_handle: c_int) -> Option<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        if file_handle >= 0 {
            // SAFETY:
            // - Operation: `std::fs::File::from_raw_fd(file_handle)`.
            // - Contract: `file_handle` must be an open, valid file descriptor whose exclusive
            //   ownership transfers to the new `File`.
            // - Evidence: `file_handle >= 0` is checked above, and by this function's `# Safety`
            //   precondition, a non-negative `file_handle` is a valid open descriptor whose
            //   exclusive ownership is transferred by the caller.
            Some(unsafe { std::fs::File::from_raw_fd(file_handle) })
        } else {
            None
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawHandle;
        // Check that the file handle is greater 0, because otherwise it will
        // abort the process. The C library fails gracefully via fdopen(); we
        // replicate that by checking here.
        if file_handle >= 0 {
            // SAFETY:
            // - Operation: `libc::get_osfhandle(file_handle)`.
            // - Contract: `file_handle` must be a valid C runtime file descriptor.
            // - Evidence: `file_handle >= 0` is checked above, and by this function's `# Safety`
            //   precondition, a non-negative `file_handle` is a valid open CRT descriptor.
            let handle = unsafe { libc::get_osfhandle(file_handle) };
            if handle == -1 {
                None
            } else {
                // SAFETY:
                // - Operation: `std::fs::File::from_raw_handle(handle as *mut c_void)`.
                // - Contract: `handle` must be a valid open Windows OS handle whose exclusive
                //   ownership transfers to the new `File`.
                // - Evidence: `get_osfhandle` succeeded (`handle != -1`), and by this function's
                //   `# Safety` precondition, exclusive ownership of the underlying descriptor/handle
                //   is transferred by the caller.
                Some(unsafe { std::fs::File::from_raw_handle(handle as *mut c_void) })
            }
        } else {
            None
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        // We cannot use file_handle on this platform to create a File,
        // so we close it to prevent leaking resources.
        if file_handle >= 0 {
            // SAFETY:
            // - Operation: `libc::close(file_handle)`.
            // - Contract: `file_handle` must be a valid open file descriptor owned by the caller.
            // - Evidence: `file_handle >= 0` is checked above, and by this function's `# Safety`
            //   precondition, exclusive ownership of `file_handle` was transferred to this call.
            unsafe {
                let _ = libc::close(file_handle);
            }
        }
        None
    }
}

/// DGifOpenFileHandle — open a GIF from a raw file descriptor.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `file_handle >= 0`, `file_handle` is a valid, open OS file descriptor, and exclusive
///   ownership of `file_handle` is transferred to `DGifOpenFileHandle` (the caller must not close
///   or access `file_handle` directly after this call).
/// - Postcondition: If `file_handle >= 0`, the descriptor is closed either when the returned
///   `GifFileType` is closed via `DGifCloseFile` (on success) or immediately before this function
///   returns `None` (if reading the GIF header fails).
// SAFETY: Exports the canonical giflib C ABI symbol `DGifOpenFileHandle` (or
// `rs_DGifOpenFileHandle` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifOpenFileHandle(
    file_handle: c_int,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    // SAFETY:
    // - Operation: `make_file(file_handle)`.
    // - Contract: If `file_handle >= 0`, it must be a valid open OS file descriptor whose
    //   exclusive ownership transfers to `make_file`.
    // - Evidence: Guaranteed directly by this function's `# Safety` precondition.
    let file = unsafe { make_file(file_handle) };

    let gif = decoder::dgif_open(file, None, ptr::null_mut());
    match gif {
        Ok(gif) => Some(gif),
        Err(e) => {
            if let Some(error) = error {
                *error = e as c_int;
            }
            None
        }
    }
}

/// DGifOpen — open a GIF with a user-supplied read callback.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `read_func` is `Some(func)`, `func` is a valid C ABI function pointer that remains valid
///   to invoke for the duration of this call and, if this call succeeds, until the returned
///   `GifFileType` is closed via `DGifCloseFile`.
/// - Whenever `func(gif, buf, len)` is invoked with a valid `GifFileType*`, `len >= 0`, and a
///   non-null pointer `buf` valid for writes of `len as usize` bytes within a single live
///   allocation, `func` writes at most `len as usize` bytes to `buf`, does not read uninitialized
///   bytes from `buf`, does not unwind across the FFI boundary, and does not invalidate `*gif`.
/// - `user_data` satisfies any validity and lifetime invariants required by `read_func` (via
///   `gif->UserData`) for the duration of this call and until `DGifCloseFile` is called.
// SAFETY: Exports the canonical giflib C ABI symbol `DGifOpen` (or `rs_DGifOpen` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifOpen(
    user_data: *mut c_void,
    read_func: InputFunc,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    if read_func.is_none() {
        if let Some(error) = error {
            *error = GifError::DReadFailed as c_int;
        }
        return None;
    }
    let callback = read_func.map(|f| {
        // SAFETY:
        // - Operation: `ReadCallback::new(f)`.
        // - Contract: `f` must be a valid C ABI function pointer for the lifetime of the
        //   `GifFileType` handle that writes at most `len` bytes into the provided buffer without
        //   unwinding or invalidating `*gif`.
        // - Evidence: Guaranteed by this function's `# Safety` precondition on `read_func` and
        //   `user_data`.
        unsafe { ReadCallback::new(f) }
    });
    let gif = decoder::dgif_open(None, callback, user_data);
    match gif {
        Ok(gif) => Some(gif),
        Err(e) => {
            if let Some(error) = error {
                *error = e as c_int;
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
//  Close
// ---------------------------------------------------------------------------

// SAFETY: Exports the canonical giflib C ABI symbol `DGifCloseFile` (or `rs_DGifCloseFile` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifCloseFile(
    gif_file: Option<Box<GifFileType>>,
    error: Option<&mut c_int>,
) -> c_int {
    match gif_file {
        Some(_) => GIF_OK, // Box<GifFileType> drops here, triggering Drop impl.
        None => {
            if let Some(error) = error {
                *error = D_GIF_ERR_CLOSE_FAILED;
            }
            GIF_ERROR
        }
    }
}

// ---------------------------------------------------------------------------
//  Decoder API
// ---------------------------------------------------------------------------

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetScreenDesc` (or
// `rs_DGifGetScreenDesc` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetScreenDesc(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_get_screen_desc(gif);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetRecordType` (or
// `rs_DGifGetRecordType` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetRecordType(
    gif_file: Option<&mut GifFileType>,
    gif_record_type: Option<&mut GifRecordType>,
) -> c_int {
    let Some(gif_record_type) = gif_record_type else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    match decoder::dgif_get_record_type(gif) {
        Ok(rt) => {
            *gif_record_type = rt;
            GIF_OK
        }
        Err(e) => {
            gif.Error = e as c_int;
            if e == GifError::DWrongRecord {
                *gif_record_type = GifRecordType::UNDEFINED_RECORD_TYPE;
            }
            GIF_ERROR
        }
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetImageHeader` (or
// `rs_DGifGetImageHeader` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetImageHeader(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_get_image_header(gif);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetImageDesc` (or `rs_DGifGetImageDesc`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetImageDesc(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_get_image_desc(gif);
    gif.result_to_status(r)
}

/// DGifGetLine — get the next line of image data from the GIF.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `gif_file` is `Some(gif)`, `line` is non-null, and `line_len >= 0`, let
///   `len = if line_len == 0 { gif.Image.Width.max(0) as usize } else { line_len as usize }`;
///   then `line` points to a buffer of at least `len` consecutive `GifPixelType` elements within
///   a single live allocation (with `len <= isize::MAX as usize`) that is valid for writes.
/// - For the duration of this call, the buffer pointed to by `line` does not alias `gif` (or any
///   memory reachable from `gif`) and is not accessed through any other pointer or thread.
// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetLine` (or `rs_DGifGetLine` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifGetLine(
    gif_file: Option<&mut GifFileType>,
    line: *mut GifPixelType,
    line_len: c_int,
) -> c_int {
    if line.is_null() || line_len < 0 {
        return GIF_ERROR;
    }
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let len = if line_len == 0 { gif.Image.Width.max(0) as usize } else { line_len as usize };
    // SAFETY:
    // - Operation: `core::slice::from_raw_parts_mut(line, len)`.
    // - Contract: `line` must be non-null, aligned for `GifPixelType` (`u8`, alignment 1), point
    //   to `len` consecutive elements within a single live allocation (`len <= isize::MAX as usize`)
    //   valid for reads and writes, and be unaliased for the duration of the slice's lifetime.
    // - Evidence: `!line.is_null()` and `line_len >= 0` are checked above; `len` is bounded by
    //   `c_int::MAX as usize <= isize::MAX as usize`; `u8` has alignment 1 and `dgif_get_line`
    //   only writes into `line_slice` without reading uninitialized bytes (cf. UCG#414); and by
    //   this function's `# Safety` precondition, `line` points to at least `len` writable bytes in
    //   a single live allocation disjoint from `gif` for the duration of this call.
    let line_slice = unsafe { core::slice::from_raw_parts_mut(line, len) };
    let r = decoder::dgif_get_line(gif, line_slice);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetPixel` (or `rs_DGifGetPixel` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetPixel(gif_file: Option<&mut GifFileType>, pixel: GifPixelType) -> c_int {
    // NOTE: The C API is a no-op by design (pass-by-value pixel).
    // We maintain bug-for-bug compatibility but the internal
    // decoder::dgif_get_pixel is fixed for direct Rust callers.
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let mut px = pixel;
    let r = decoder::dgif_get_pixel(gif, &mut px);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetExtension` (or `rs_DGifGetExtension`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetExtension(
    gif_file: Option<&mut GifFileType>,
    ext_code: Option<&mut c_int>,
    extension: Option<&mut *mut GifByteType>,
) -> c_int {
    let Some(ext_code) = ext_code else {
        return GIF_ERROR;
    };
    let Some(extension) = extension else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    match decoder::dgif_get_extension(gif, ext_code, extension) {
        Ok(()) => {
            // Re-derive the buf pointer to ensure correct provenance.
            if !extension.is_null() {
                *extension = gif.Private.lzw.buf.as_mut_ptr();
            }
            GIF_OK
        }
        Err(e) => {
            gif.Error = e as c_int;
            GIF_ERROR
        }
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetExtensionNext` (or
// `rs_DGifGetExtensionNext` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetExtensionNext(
    gif_file: Option<&mut GifFileType>,
    extension: Option<&mut *mut GifByteType>,
) -> c_int {
    let Some(extension) = extension else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    match decoder::dgif_get_extension_next(gif) {
        Ok(true) => {
            *extension = gif.Private.lzw.buf.as_mut_ptr();
            GIF_OK
        }
        Ok(false) => {
            *extension = ptr::null_mut();
            GIF_OK
        }
        Err(e) => {
            gif.Error = e as c_int;
            GIF_ERROR
        }
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetCode` (or `rs_DGifGetCode` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetCode(
    gif_file: Option<&mut GifFileType>,
    code_size: Option<&mut c_int>,
    code_block: Option<&mut *mut GifByteType>,
) -> c_int {
    let Some(code_size) = code_size else {
        return GIF_ERROR;
    };
    let Some(code_block) = code_block else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    match decoder::dgif_get_code(gif, code_size, code_block) {
        Ok(()) => {
            if !code_block.is_null() {
                *code_block = gif.Private.lzw.buf.as_mut_ptr();
            }
            GIF_OK
        }
        Err(e) => {
            gif.Error = e as c_int;
            GIF_ERROR
        }
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetCodeNext` (or `rs_DGifGetCodeNext` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetCodeNext(
    gif_file: Option<&mut GifFileType>,
    code_block: Option<&mut *mut GifByteType>,
) -> c_int {
    let Some(code_block) = code_block else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    match decoder::dgif_get_code_next(gif) {
        Ok(true) => {
            *code_block = gif.Private.lzw.buf.as_mut_ptr();
            GIF_OK
        }
        Ok(false) => {
            *code_block = ptr::null_mut();
            GIF_OK
        }
        Err(e) => {
            gif.Error = e as c_int;
            GIF_ERROR
        }
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetLZCodes` (or `rs_DGifGetLZCodes` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetLZCodes(
    gif_file: Option<&mut GifFileType>,
    code: Option<&mut c_int>,
) -> c_int {
    let Some(code) = code else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_get_lz_codes(gif, code);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifSlurp` (or `rs_DGifSlurp` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifSlurp(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_slurp(gif);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifGetGifVersion` (or
// `rs_DGifGetGifVersion` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifGetGifVersion(gif_file: Option<&mut GifFileType>) -> *const c_char {
    let Some(gif) = gif_file else {
        return GIF87_STAMP.as_ptr() as *const c_char;
    };
    decoder::dgif_get_gif_version(gif).as_ptr() as *const c_char
}

// ---------------------------------------------------------------------------
//  GCB helpers
// ---------------------------------------------------------------------------

/// DGifExtensionToGCB — convert a GIF extension to a Graphics Control Block.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `gcb` is `Some`, `gif_extension` is non-null, and `gif_extension_length == 4`, then
///   `gif_extension` points to 4 consecutive initialized `GifByteType` bytes within a single live
///   allocation that is valid for reads.
/// - For the duration of this call, the 4 bytes at `gif_extension` do not alias `gcb` and are not
///   mutated by any other pointer or thread.
// SAFETY: Exports the canonical giflib C ABI symbol `DGifExtensionToGCB` (or
// `rs_DGifExtensionToGCB` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifExtensionToGCB(
    gif_extension_length: usize,
    gif_extension: *const GifByteType,
    gcb: Option<&mut GraphicsControlBlock>,
) -> c_int {
    let Some(gcb) = gcb else {
        return GIF_ERROR;
    };
    if gif_extension.is_null() || gif_extension_length != 4 {
        return GIF_ERROR;
    }
    // SAFETY:
    // - Operation: `core::slice::from_raw_parts(gif_extension, 4)`.
    // - Contract: `gif_extension` must be non-null, 1-byte aligned, point to 4 consecutive
    //   initialized `GifByteType` elements within a single live allocation (`4 <= isize::MAX`),
    //   and not be mutated for the duration of the slice borrow.
    // - Evidence: `!gif_extension.is_null()` and `gif_extension_length == 4` are checked above,
    //   `align_of::<GifByteType>() == 1`, and by this function's `# Safety` precondition,
    //   `gif_extension` points to 4 initialized bytes in a single live allocation disjoint from
    //   `gcb` and unmutated for the duration of this call.
    let ext = unsafe { core::slice::from_raw_parts(gif_extension, 4) };
    match decoder::dgif_extension_to_gcb(ext, gcb) {
        Ok(()) => GIF_OK,
        Err(_) => GIF_ERROR,
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `DGifSavedExtensionToGCB` (or
// `rs_DGifSavedExtensionToGCB` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn DGifSavedExtensionToGCB(
    gif_file: Option<&mut GifFileType>,
    image_index: c_int,
    gcb: Option<&mut GraphicsControlBlock>,
) -> c_int {
    let Some(gcb) = gcb else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    match decoder::dgif_saved_extension_to_gcb(gif, image_index, gcb) {
        Ok(()) => GIF_OK,
        Err(_) => GIF_ERROR,
    }
}

// ---------------------------------------------------------------------------
//  Allocation API
// ---------------------------------------------------------------------------

/// GifMakeMapObject — create a new color map object.
///
/// Returns NULL unless `color_count` is a power of 2 from 2 to 512. The colors are zero if
/// `color_map` is null.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `color_map` is non-null and `color_count` is a power of 2 in `2..=512`, `color_map`
///   points to at least `color_count as usize` consecutive initialized `GifColorType` elements
///   within a single live allocation (`color_count as usize * size_of::<GifColorType>() <= isize::MAX as usize`).
/// - For the duration of this call, the memory referenced by `color_map` is not mutated by any
///   other pointer or thread. Alignment is satisfied because `align_of::<GifColorType>() == 1`.
// SAFETY: Exports the canonical giflib C ABI symbol `GifMakeMapObject` (or `rs_GifMakeMapObject`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifMakeMapObject(
    color_count: c_int,
    color_map: *const GifColorType,
) -> Option<Box<ColorMapObject>> {
    let mut object = ColorMapObject::new(color_count).ok()?;
    if !color_map.is_null() {
        let colors = object.colors_mut();
        // SAFETY:
        // - Operation: `core::slice::from_raw_parts(color_map, colors.len())`.
        // - Contract: `color_map` must be non-null, 1-byte aligned (`align_of::<GifColorType>() == 1`),
        //   point to `colors.len()` consecutive initialized `GifColorType` elements within a
        //   single live allocation (`colors.len() * 3 <= 1536 <= isize::MAX as usize`), and not be
        //   mutated during the call.
        // - Evidence: `!color_map.is_null()` is checked above; `colors.len()` equals
        //   `color_count as usize` (`2..=512`) as allocated by `ColorMapObject::new(color_count)`;
        //   and by this function's `# Safety` precondition, `color_map` points to at least
        //   `color_count as usize` initialized `GifColorType` elements in a single live allocation
        //   that is disjoint from the freshly allocated `colors` buffer and unmutated during this
        //   call.
        colors.copy_from_slice(unsafe { core::slice::from_raw_parts(color_map, colors.len()) });
    }
    Some(Box::new(object))
}

// SAFETY: Exports the canonical giflib C ABI symbol `GifFreeMapObject` (or `rs_GifFreeMapObject`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifFreeMapObject(object: Option<Box<ColorMapObject>>) {
    drop(object);
}

/// GifUnionColorMap — compute the union of two color map objects.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `color_in1` and `color_in2` are `Some` and `color_trans_in2` is non-null,
///   `color_trans_in2` points to a buffer valid for writes of at least
///   `color_in2.colors().len()` consecutive `GifPixelType` elements within a single live
///   allocation (`<= isize::MAX` bytes).
/// - For the duration of this call, the buffer at `color_trans_in2` does not alias `color_in1` or
///   `color_in2` (or their `Colors` arrays) and is not accessed through any other pointer or
///   thread.
// SAFETY: Exports the canonical giflib C ABI symbol `GifUnionColorMap` (or `rs_GifUnionColorMap`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifUnionColorMap(
    color_in1: Option<&ColorMapObject>,
    color_in2: Option<&ColorMapObject>,
    color_trans_in2: *mut GifPixelType,
) -> Option<Box<ColorMapObject>> {
    let color_in1 = color_in1?;
    let color_in2 = color_in2?;
    if color_trans_in2.is_null() {
        return None;
    }
    let trans_len = color_in2.colors().len();

    // SAFETY:
    // - Operation: `core::slice::from_raw_parts_mut(color_trans_in2, trans_len)`.
    // - Contract: `color_trans_in2` must be non-null, 1-byte aligned
    //   (`align_of::<GifPixelType>() == 1`), point to `trans_len` consecutive `GifPixelType`
    //   elements within a single live allocation (`trans_len <= isize::MAX as usize`) valid for
    //   reads and writes, and be unaliased for the duration of the slice's lifetime.
    // - Evidence: `!color_trans_in2.is_null()` is checked above; by `ColorMapObject::Colors`'s
    //   field safety invariant, `trans_len <= isize::MAX as usize`; `union_with` only writes into
    //   `trans` without reading uninitialized bytes (cf. UCG#414); and by this function's
    //   `# Safety` precondition, `color_trans_in2` points to at least `trans_len` writable bytes
    //   in a single live allocation disjoint from `color_in1` and `color_in2` for the duration of
    //   this call.
    let trans = unsafe { core::slice::from_raw_parts_mut(color_trans_in2, trans_len) };

    color_in1.union_with(color_in2, trans).map(Box::new)
}

// SAFETY: Exports the canonical giflib C ABI symbol `GifBitSize` (or `rs_GifBitSize` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifBitSize(n: c_int) -> c_int {
    ColorMapObject::bit_size(n)
}

// SAFETY: Exports the canonical giflib C ABI symbol `GifApplyTranslation` (or
// `rs_GifApplyTranslation` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifApplyTranslation(
    image: Option<&mut SavedImage>,
    translation: Option<&[GifPixelType; 256]>,
) {
    let Some(image) = image else {
        return;
    };
    let Some(translation) = translation else {
        return;
    };
    image.apply_translation(translation);
}

/// GifAddExtensionBlock — add a new extension block to a GIF file.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `extension_block_count` is `Some(count)` and `extension_blocks` is `Some(blocks)`:
///   - `*count >= 0`.
///   - Either `*blocks` is null with `*count == 0`, or `*blocks` points to a `LibcAlloc`-allocated
///     array of `*count` consecutive initialized `ExtensionBlock` elements (each satisfying
///     `ExtensionBlock`'s field safety invariant) within a single live allocation with length-only
///     capacity (`capacity == *count`).
/// - If `ext_data` is non-null and `len > 0` (`len <= c_int::MAX as c_uint`), `ext_data` points
///   to at least `len as usize` consecutive initialized `u8` bytes valid for reads within a single
///   live allocation (`len as usize <= isize::MAX as usize`), and is not mutated or aliased by
///   `extension_block_count` or `extension_blocks` for the duration of this call.
// SAFETY: Exports the canonical giflib C ABI symbol `GifAddExtensionBlock` (or
// `rs_GifAddExtensionBlock` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifAddExtensionBlock(
    extension_block_count: Option<&mut c_int>,
    extension_blocks: Option<&mut CBufPtr<ExtensionBlock>>,
    function: c_int,
    len: c_uint,
    ext_data: *mut u8,
) -> c_int {
    let Some(extension_block_count) = extension_block_count else {
        return GIF_ERROR;
    };
    let Some(extension_blocks) = extension_blocks else {
        return GIF_ERROR;
    };
    // `ByteCount` is a `c_int`: reject longer blocks before allocating anything for them.
    if c_int::try_from(len).is_err() {
        return GIF_ERROR;
    }
    let zero_buf;
    let data: &[u8] = if ext_data.is_null() || len == 0 {
        if len > 0 {
            // null ext_data but len > 0: create zeroed buffer to match C behavior.
            zero_buf = vec![0u8; len as usize];
            &zero_buf
        } else {
            &[]
        }
    } else {
        // SAFETY:
        // - Operation: `core::slice::from_raw_parts(ext_data, len as usize)`.
        // - Contract: `ext_data` must be non-null, 1-byte aligned, point to `len as usize`
        //   consecutive initialized `u8` bytes within a single live allocation
        //   (`len as usize <= isize::MAX as usize`), and not be mutated for the lifetime of `data`.
        // - Evidence: `!ext_data.is_null()`, `len > 0`, and `c_int::try_from(len).is_ok()` (so
        //   `len as usize <= c_int::MAX as usize <= isize::MAX as usize`) are checked above; and by
        //   this function's `# Safety` precondition, `ext_data` points to `len as usize`
        //   initialized bytes in a single live allocation that is not mutated before
        //   `ExtensionBlock::try_new` copies `data`.
        unsafe { core::slice::from_raw_parts(ext_data, len as usize) }
    };
    let Ok(block) = ExtensionBlock::try_new(function, data) else {
        return GIF_ERROR;
    };
    // SAFETY:
    // - Operation: `extension_blocks.as_vec_mut(extension_block_count)`.
    // - Contract: `*extension_block_count >= 0`, and either `*extension_blocks` is null with
    //   `*extension_block_count == 0`, or `*extension_blocks` points to a `LibcAlloc`-allocated
    //   array of `*extension_block_count` initialized `ExtensionBlock` elements with length-only
    //   capacity.
    // - Evidence: Guaranteed by this function's `# Safety` precondition on `extension_blocks` and
    //   `extension_block_count`.
    match unsafe { extension_blocks.as_vec_mut(extension_block_count) }.try_push_back(block) {
        Ok(()) => GIF_OK,
        Err(_) => GIF_ERROR,
    }
}

/// GifFreeExtensions — free the memory allocated for the extension blocks.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `extension_block_count` is `Some(count)` and `extension_blocks` is `Some(blocks)`:
///   - `*count >= 0`.
///   - Either `*blocks` is null with `*count == 0`, or `*blocks` points to a `LibcAlloc`-allocated
///     array of `*count` consecutive initialized `ExtensionBlock` elements (each satisfying
///     `ExtensionBlock`'s field safety invariant) within a single live allocation with length-only
///     capacity (`capacity == *count`), and exclusive ownership of those elements is available to
///     be dropped and deallocated by this call.
// SAFETY: Exports the canonical giflib C ABI symbol `GifFreeExtensions` (or
// `rs_GifFreeExtensions` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifFreeExtensions(
    extension_block_count: Option<&mut c_int>,
    extension_blocks: Option<&mut CBufPtr<ExtensionBlock>>,
) {
    let Some(extension_block_count) = extension_block_count else {
        return;
    };
    let Some(extension_blocks) = extension_blocks else {
        return;
    };
    // SAFETY:
    // - Operation: `extension_blocks.as_vec_mut(extension_block_count)`.
    // - Contract: `*extension_block_count >= 0`, and either `*extension_blocks` is null with
    //   `*extension_block_count == 0`, or `*extension_blocks` points to a `LibcAlloc`-allocated
    //   array of `*extension_block_count` initialized `ExtensionBlock` elements with length-only
    //   capacity.
    // - Evidence: Guaranteed by this function's `# Safety` precondition on `extension_blocks` and
    //   `extension_block_count`.
    unsafe { extension_blocks.as_vec_mut(extension_block_count) }.clear();
}

/// GifMakeSavedImage — append an image to `gif_file->SavedImages`, copying `copy_from` if
/// non-null.
///
/// # Safety
///
/// The caller must ensure that:
/// - `copy_from` is either null or properly aligned and points to an initialized `SavedImage`
///   within a live allocation whose fields satisfy `SavedImage`'s field safety invariants.
/// - `copy_from` may point to an element inside `gif_file->SavedImages`; no mutable reference
///   other than `gif_file` may alias `*copy_from` for the duration of this call.
// SAFETY: Exports the canonical giflib C ABI symbol `GifMakeSavedImage` (or
// `rs_GifMakeSavedImage` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifMakeSavedImage<'a>(
    gif_file: Option<&'a mut GifFileType>,
    copy_from: *const SavedImage,
) -> Option<&'a mut SavedImage> {
    let gif = gif_file?;

    // `copy_from` may point into `gif.SavedImages`, which `try_push_back` reallocates. It is a raw
    // pointer because a reference parameter would have to stay valid until the function returns.
    // SAFETY:
    // - Operation: `copy_from.as_ref()`.
    // - Contract: If `copy_from` is non-null, it must be properly aligned for `SavedImage`, point
    //   to an initialized valid `SavedImage` in a live allocation, and not be mutably aliased
    //   during the lifetime of the temporary `&SavedImage`.
    // - Evidence: By this function's `# Safety` precondition, `copy_from` is null or aligned and
    //   points to a valid `SavedImage`. Even if `copy_from` points into the heap buffer behind
    //   `gif.SavedImages` (a raw `CBufPtr`), `gif` is not accessed while the temporary
    //   `&SavedImage` is cloned, and the temporary reference is dropped at the end of the `match`
    //   before `gif.saved_images_vec().try_push_back(sp)` mutates or reallocates `gif.SavedImages`.
    let sp = match unsafe { copy_from.as_ref() } {
        Some(copy_from) => copy_from.clone(),
        None => SavedImage::new(GifImageDesc {
            Left: 0,
            Top: 0,
            Width: 0,
            Height: 0,
            Interlace: false,
            ColorMap: None,
        }),
    };

    gif.saved_images_vec().try_push_back(sp).ok()?;

    // Return reference to the newly added last element.
    gif.saved_images_mut().last_mut()
}

// SAFETY: Exports the canonical giflib C ABI symbol `GifFreeSavedImages` (or
// `rs_GifFreeSavedImages` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifFreeSavedImages(gif_file: Option<&mut GifFileType>) {
    if let Some(gif) = gif_file {
        gif.saved_images_vec().clear();
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `openbsd_reallocarray` (or
// `rs_openbsd_reallocarray` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn openbsd_reallocarray(
    _optr: *mut c_void,
    _nmemb: usize,
    _size: usize,
) -> *mut c_void {
    panic!("openbsd_reallocarray is not implemented; use standard allocation functions instead");
}

// ---------------------------------------------------------------------------
//  Error API
// ---------------------------------------------------------------------------

// SAFETY: Exports the canonical giflib C ABI symbol `GifErrorString` (or `rs_GifErrorString` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifErrorString(error_code: c_int) -> *const c_char {
    match err::GifError::from_error_code(error_code) {
        Some(e) => e.to_error_string().as_ptr(),
        None => ptr::null(),
    }
}

// ---------------------------------------------------------------------------
//  Encoder API
// ---------------------------------------------------------------------------

// SAFETY: Exports the canonical giflib C ABI symbol `EGifOpenFileName` (or `rs_EGifOpenFileName`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifOpenFileName(
    file_name: Option<CStrRef<'_>>,
    gif89: bool,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    let Some(file_name) = file_name else {
        if let Some(error) = error {
            *error = GifError::EOpenFailed as c_int;
        }
        return None;
    };
    let c_str = file_name.to_c_str();
    let gif = encoder::egif_open_file_name(c_str, gif89);
    match gif {
        Ok(gif) => Some(gif),
        Err(e) => {
            if let Some(error) = error {
                *error = e as c_int;
            }
            None
        }
    }
}

/// EGifOpenFileHandle — open a GIF for write from a raw file descriptor.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `file_handle >= 0`, `file_handle` is a valid, open OS file descriptor, and exclusive
///   ownership of `file_handle` is transferred to `EGifOpenFileHandle` (the caller must not close
///   or access `file_handle` directly after this call).
/// - Postcondition: If `file_handle >= 0`, the descriptor is closed either when the returned
///   `GifFileType` is closed via `EGifCloseFile` / `EGifSpew` (on success) or immediately before
///   this function returns `None` (if opening fails).
// SAFETY: Exports the canonical giflib C ABI symbol `EGifOpenFileHandle` (or
// `rs_EGifOpenFileHandle` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifOpenFileHandle(
    file_handle: c_int,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    // SAFETY:
    // - Operation: `make_file(file_handle)`.
    // - Contract: If `file_handle >= 0`, it must be a valid open OS file descriptor whose
    //   exclusive ownership transfers to `make_file`.
    // - Evidence: Guaranteed directly by this function's `# Safety` precondition.
    let file = unsafe { make_file(file_handle) };

    let gif = encoder::egif_open(file, None, ptr::null_mut());
    match gif {
        Ok(gif) => Some(gif),
        Err(e) => {
            if let Some(error) = error {
                *error = e as c_int;
            }
            None
        }
    }
}

/// EGifOpen — open a GIF with a user-supplied write callback.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `write_func` is `Some(func)`, `func` is a valid C ABI function pointer that remains valid
///   to invoke for the duration of this call and, if this call succeeds, until the returned
///   `GifFileType` is closed via `EGifCloseFile` or `EGifSpew`.
/// - Whenever `func(gif, buf, len)` is invoked with a valid `GifFileType*`, `len >= 0`, and a
///   non-null pointer `buf` valid for reads of `len as usize` initialized bytes within a single
///   live allocation, `func` reads at most `len as usize` bytes from `buf`, does not mutate
///   memory through `buf`, does not unwind across the FFI boundary, and does not invalidate
///   `*gif`.
/// - `user_data` satisfies any validity and lifetime invariants required by `write_func` (via
///   `gif->UserData`) for the duration of this call and until `EGifCloseFile` / `EGifSpew` is
///   called.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifOpen` (or `rs_EGifOpen` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifOpen(
    user_data: *mut c_void,
    write_func: OutputFunc,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    let callback = write_func.map(|f| {
        // SAFETY:
        // - Operation: `WriteCallback::new(f)`.
        // - Contract: `f` must be a valid C ABI function pointer for the lifetime of the
        //   `GifFileType` handle that reads at most `len` bytes from the provided buffer without
        //   unwinding or invalidating `*gif`.
        // - Evidence: Guaranteed by this function's `# Safety` precondition on `write_func` and
        //   `user_data`.
        unsafe { WriteCallback::new(f) }
    });
    let gif = encoder::egif_open(None, callback, user_data);
    match gif {
        Ok(gif) => Some(gif),
        Err(e) => {
            if let Some(error) = error {
                *error = e as c_int;
            }
            None
        }
    }
}

// SAFETY: Exports the canonical giflib C ABI symbol `EGifCloseFile` (or `rs_EGifCloseFile` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifCloseFile(
    gif_file: Option<Box<GifFileType>>,
    error: Option<&mut c_int>,
) -> c_int {
    let Some(mut gif) = gif_file else {
        // C leaves `*ErrorCode` unset here; reporting an error code is deliberate.
        if let Some(error) = error {
            *error = E_GIF_ERR_CLOSE_FAILED;
        }
        return GIF_ERROR;
    };

    // Write the terminator on the owned value; Drop cleans up at scope end.
    if let Err(e) = encoder::egif_close_file(&mut gif) {
        if let Some(error) = error {
            *error = e as c_int;
        }
        return GIF_ERROR;
    }

    GIF_OK
}

// SAFETY: Exports the canonical giflib C ABI symbol `EGifGetGifVersion` (or
// `rs_EGifGetGifVersion` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifGetGifVersion(gif_file: Option<&mut GifFileType>) -> *const c_char {
    let Some(gif) = gif_file else {
        return GIF87_STAMP.as_ptr() as *const c_char;
    };
    encoder::egif_get_gif_version(gif).as_ptr() as *const c_char
}

// SAFETY: Exports the canonical giflib C ABI symbol `EGifSetGifVersion` (or
// `rs_EGifSetGifVersion` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifSetGifVersion(gif_file: Option<&mut GifFileType>, gif89: bool) {
    let Some(gif) = gif_file else {
        return;
    };
    encoder::egif_set_gif_version(gif, gif89);
}

/// EGifPutScreenDesc — write the GIF header, logical screen descriptor and global color map.
///
/// # Safety
///
/// The caller must ensure that:
/// - `color_map` is either null or properly aligned and points to an initialized `ColorMapObject`
///   within a live allocation whose fields satisfy `ColorMapObject`'s field safety invariants.
/// - `color_map` may point to one of `gif_file`'s own maps (e.g. `gif_file->SColorMap` or
///   `gif_file->Image.ColorMap`); no mutable reference other than `gif_file` may alias
///   `*color_map` for the duration of this call.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutScreenDesc` (or
// `rs_EGifPutScreenDesc` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutScreenDesc(
    gif_file: Option<&mut GifFileType>,
    width: c_int,
    height: c_int,
    color_resolution: c_int,
    background: c_int,
    color_map: *const ColorMapObject,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    // C callers may pass `gif->SColorMap` back in, which `egif_put_screen_desc` would otherwise
    // replace. `color_map` is a raw pointer because a reference parameter must stay valid until
    // the function returns.
    let color_map = if gif.SColorMap.as_deref().is_some_and(|stored| ptr::eq(stored, color_map)) {
        encoder::Arg::Current
    } else {
        // SAFETY:
        // - Operation: `color_map.as_ref()`.
        // - Contract: If `color_map` is non-null, it must be properly aligned for
        //   `ColorMapObject`, point to an initialized valid `ColorMapObject` in a live allocation,
        //   and not be mutably aliased during the lifetime of the temporary `&ColorMapObject`.
        // - Evidence: By this function's `# Safety` precondition, `color_map` is null or aligned
        //   and points to a valid `ColorMapObject`. Even if `color_map` points to another map
        //   owned by `gif` (e.g. `gif.Image.ColorMap`), `gif` is not accessed while the temporary
        //   `&ColorMapObject` is cloned, and the temporary reference is dropped before
        //   `encoder::egif_put_screen_desc` mutates `gif`.
        encoder::Arg::New(unsafe { color_map.as_ref() }.cloned())
    };
    let r =
        encoder::egif_put_screen_desc(gif, width, height, color_resolution, background, color_map);
    gif.result_to_status(r)
}

/// EGifPutImageDesc — write an image descriptor and its local color map.
///
/// # Safety
///
/// The caller must ensure that:
/// - `color_map` is either null or properly aligned and points to a `ColorMapObject` within a
///   live allocation with initialized, valid `ColorCount`, `BitsPerPixel`, and `Colors` fields
///   satisfying `ColorMapObject::Colors`'s field safety invariant.
/// - `color_map` may point to one of `gif_file`'s own maps (e.g. `gif_file->Image.ColorMap` or
///   `gif_file->SColorMap`); no mutable reference other than `gif_file` may alias `*color_map`
///   for the duration of this call.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutImageDesc` (or `rs_EGifPutImageDesc`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutImageDesc(
    gif_file: Option<&mut GifFileType>,
    left: c_int,
    top: c_int,
    width: c_int,
    height: c_int,
    interlace: bool,
    color_map: *const ColorMapObject,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    // As in `EGifPutScreenDesc`: C callers may pass `gif->Image.ColorMap` back in, which
    // `egif_put_image_desc` would otherwise replace.
    let color_map =
        if gif.Image.ColorMap.as_deref().is_some_and(|stored| ptr::eq(stored, color_map)) {
            encoder::Arg::Current
        } else {
            // SAFETY:
            // - Operation: `color_map.as_ref()`.
            // - Contract: If `color_map` is non-null, it must be properly aligned for
            //   `ColorMapObject`, point to a live `ColorMapObject` whose accessed fields
            //   (`ColorCount`, `BitsPerPixel`, `Colors`) are initialized and valid, and not be
            //   mutably aliased during the lifetime of the temporary `&ColorMapObject`.
            // - Evidence: By this function's `# Safety` precondition, `color_map` is null or
            //   aligned and points to a `ColorMapObject` with valid `ColorCount`, `BitsPerPixel`,
            //   and `Colors`. Even if `color_map` is owned by `gif` (e.g. `gif.SColorMap`), `gif`
            //   is not accessed while the fields are copied, and the temporary reference is
            //   dropped before `encoder::egif_put_image_desc` mutates `gif`. (Unaccessed fields
            //   such as `SortFlag` are never read, cf. UCG#414.)
            encoder::Arg::New(unsafe { color_map.as_ref() }.map(|cm| ColorMapObject {
                ColorCount: cm.ColorCount,
                BitsPerPixel: cm.BitsPerPixel,
                SortFlag: false,
                Colors: OwnedCBufPtr::clone_from_slice(cm.colors()),
            }))
        };
    let r = encoder::egif_put_image_desc(gif, left, top, width, height, interlace, color_map);
    gif.result_to_status(r)
}

/// EGifPutLine — write a single row of pixel data to the GIF.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `gif_file` is `Some(gif)`, `line` is non-null, and `line_len > 0`, `line` points to at
///   least `line_len as usize` consecutive initialized `GifPixelType` elements within a single
///   live allocation (`line_len as usize <= isize::MAX as usize`) that is valid for both reads and
///   writes (because `EGifPutLine` masks pixel values in place).
/// - For the duration of this call, the buffer pointed to by `line` does not alias `gif` (or any
///   memory reachable from `gif`) and is not accessed through any other pointer or thread.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutLine` (or `rs_EGifPutLine` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutLine(
    gif_file: Option<&mut GifFileType>,
    line: *mut GifPixelType,
    line_len: c_int,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let len = if line_len <= 0 { 0 } else { line_len as usize };
    // When len is 0 or line is null, we pass an empty slice; egif_put_line defaults to Image.Width.
    let line_slice = if len == 0 || line.is_null() {
        &mut []
    } else {
        // SAFETY:
        // - Operation: `core::slice::from_raw_parts_mut(line, len)`.
        // - Contract: `line` must be non-null, 1-byte aligned, point to `len` consecutive
        //   initialized `GifPixelType` elements valid for reads and writes within a single live
        //   allocation (`len <= isize::MAX as usize`), and be unaliased for the slice's lifetime.
        // - Evidence: `!line.is_null()` and `line_len > 0` (so `len <= c_int::MAX as usize <= isize::MAX as usize`)
        //   are checked above, `align_of::<GifPixelType>() == 1`, and by this function's `# Safety`
        //   precondition, `line` points to `len` initialized readable and writable bytes in a
        //   single live allocation disjoint from `gif` for the duration of this call.
        unsafe { core::slice::from_raw_parts_mut(line, len) }
    };
    let r = encoder::egif_put_line(gif, line_slice);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutPixel` (or `rs_EGifPutPixel` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifPutPixel(gif_file: Option<&mut GifFileType>, pixel: GifPixelType) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = encoder::egif_put_pixel(gif, pixel);
    gif.result_to_status(r)
}

/// EGifPutComment — write a comment to the GIF.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutComment` (or `rs_EGifPutComment` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifPutComment(
    gif_file: Option<&mut GifFileType>,
    comment: Option<CStrRef<'_>>,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let Some(comment) = comment else {
        return GIF_ERROR;
    };
    let r = encoder::egif_put_comment(gif, comment.to_bytes());
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutExtensionLeader` (or
// `rs_EGifPutExtensionLeader` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifPutExtensionLeader(
    gif_file: Option<&mut GifFileType>,
    ext_code: c_int,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = encoder::egif_put_extension_leader(gif, ext_code);
    gif.result_to_status(r)
}

/// EGifPutExtensionBlock — write an extension block to the GIF.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `gif_file` is `Some(gif)`, `extension` is non-null, and `ext_len > 0`, `extension` points
///   to at least `ext_len as usize` consecutive initialized `u8` bytes valid for reads within a
///   single live allocation (`ext_len as usize <= isize::MAX as usize`).
/// - For the duration of this call, the bytes at `extension` do not alias `gif` and are not
///   mutated by any other pointer or thread.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutExtensionBlock` (or
// `rs_EGifPutExtensionBlock` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutExtensionBlock(
    gif_file: Option<&mut GifFileType>,
    ext_len: c_int,
    extension: *const c_void,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let ext_data = if extension.is_null() || ext_len <= 0 {
        &[]
    } else {
        // SAFETY:
        // - Operation: `core::slice::from_raw_parts(extension as *const u8, ext_len as usize)`.
        // - Contract: `extension` must be non-null, 1-byte aligned, point to `ext_len as usize`
        //   consecutive initialized `u8` bytes within a single live allocation
        //   (`ext_len as usize <= isize::MAX as usize`), and not be mutated for the slice's lifetime.
        // - Evidence: `!extension.is_null()` and `ext_len > 0` (so
        //   `ext_len as usize <= c_int::MAX as usize <= isize::MAX as usize`) are checked above,
        //   `align_of::<u8>() == 1`, and by this function's `# Safety` precondition, `extension`
        //   points to `ext_len as usize` initialized bytes in a single live allocation disjoint
        //   from `gif` and unmutated for the duration of this call.
        unsafe { core::slice::from_raw_parts(extension as *const u8, ext_len as usize) }
    };
    let r = encoder::egif_put_extension_block(gif, ext_data);
    gif.result_to_status(r)
}

// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutExtensionTrailer` (or
// `rs_EGifPutExtensionTrailer` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifPutExtensionTrailer(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = encoder::egif_put_extension_trailer(gif);
    gif.result_to_status(r)
}

/// EGifPutExtension — write an extension with a given extension code to the GIF.
///
/// # Safety
///
/// The caller must ensure that:
/// - If `gif_file` is `Some(gif)`, `extension` is non-null, and `ext_len > 0`, `extension` points
///   to at least `ext_len as usize` consecutive initialized `u8` bytes valid for reads within a
///   single live allocation (`ext_len as usize <= isize::MAX as usize`).
/// - For the duration of this call, the bytes at `extension` do not alias `gif` and are not
///   mutated by any other pointer or thread.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutExtension` (or `rs_EGifPutExtension`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutExtension(
    gif_file: Option<&mut GifFileType>,
    ext_code: c_int,
    ext_len: c_int,
    extension: *const c_void,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let ext_data = if extension.is_null() || ext_len <= 0 {
        &[]
    } else {
        // SAFETY:
        // - Operation: `core::slice::from_raw_parts(extension as *const u8, ext_len as usize)`.
        // - Contract: `extension` must be non-null, 1-byte aligned, point to `ext_len as usize`
        //   consecutive initialized `u8` bytes within a single live allocation
        //   (`ext_len as usize <= isize::MAX as usize`), and not be mutated for the slice's lifetime.
        // - Evidence: `!extension.is_null()` and `ext_len > 0` (so
        //   `ext_len as usize <= c_int::MAX as usize <= isize::MAX as usize`) are checked above,
        //   `align_of::<u8>() == 1`, and by this function's `# Safety` precondition, `extension`
        //   points to `ext_len as usize` initialized bytes in a single live allocation disjoint
        //   from `gif` and unmutated for the duration of this call.
        unsafe { core::slice::from_raw_parts(extension as *const u8, ext_len as usize) }
    };
    let r = encoder::egif_put_extension(gif, ext_code, ext_data);
    gif.result_to_status(r)
}

/// EGifPutCode
///
/// # Safety
///
/// The caller must ensure that:
/// - If `gif_file` is `Some(gif)` and `code_block` is non-null, `code_block` points to a valid
///   Pascal-style byte block within a single live allocation where `code_block[0]` is initialized
///   and all `(*code_block as usize) + 1` bytes `code_block[0..=(*code_block as usize)]` are
///   consecutive initialized `GifByteType` values valid for reads.
/// - For the duration of this call, the bytes at `code_block` do not alias `gif` and are not
///   mutated by any other pointer or thread.
///
/// The `code_size` parameter is unused and should be ignored. This behavior
/// matches the C library.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutCode` (or `rs_EGifPutCode` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutCode(
    gif_file: Option<&mut GifFileType>,
    _code_size: c_int, // Unused, matches C library.
    code_block: *const GifByteType,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    if code_block.is_null() {
        return GIF_ERROR;
    }
    // SAFETY:
    // - Operation: dereferencing `*code_block` to read the Pascal-block length byte.
    // - Contract: `code_block` must be non-null, 1-byte aligned, and point to an initialized
    //   `GifByteType` valid for reads.
    // - Evidence: `!code_block.is_null()` is checked above, `align_of::<GifByteType>() == 1`, and
    //   by this function's `# Safety` precondition, `code_block[0]` is an initialized byte in a
    //   live allocation disjoint from `gif`.
    let len = unsafe { *code_block } as usize + 1;
    // SAFETY:
    // - Operation: `core::slice::from_raw_parts(code_block, len)`.
    // - Contract: `code_block` must be non-null, 1-byte aligned, point to `len` (`1..=256`)
    //   consecutive initialized `GifByteType` elements within a single live allocation
    //   (`len <= 256 <= isize::MAX as usize`), and not be mutated for the slice's lifetime.
    // - Evidence: `!code_block.is_null()` is checked above, `len` is at most `u8::MAX as usize + 1 == 256`,
    //   and by this function's `# Safety` precondition, all `len` bytes of the Pascal-style block
    //   lie in a single live allocation disjoint from `gif` and unmutated during this call.
    let block = unsafe { core::slice::from_raw_parts(code_block, len) };
    let r = encoder::egif_put_code(gif, block);
    gif.result_to_status(r)
}

/// EGifPutCodeNext
///
/// # Safety
///
/// The caller must ensure that:
/// - If `gif_file` is `Some(gif)` and `code_block` is non-null, `code_block` points to a valid
///   Pascal-style byte block within a single live allocation where `code_block[0]` is initialized
///   and all `(*code_block as usize) + 1` bytes `code_block[0..=(*code_block as usize)]` are
///   consecutive initialized `GifByteType` values valid for reads.
/// - For the duration of this call, the bytes at `code_block` do not alias `gif` and are not
///   mutated by any other pointer or thread.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifPutCodeNext` (or `rs_EGifPutCodeNext` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifPutCodeNext(
    gif_file: Option<&mut GifFileType>,
    code_block: *const GifByteType,
) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let block = if code_block.is_null() {
        None
    } else {
        // SAFETY:
        // - Operation: dereferencing `*code_block` to read the Pascal-block length byte.
        // - Contract: `code_block` must be non-null, 1-byte aligned, and point to an initialized
        //   `GifByteType` valid for reads.
        // - Evidence: `!code_block.is_null()` is checked above, `align_of::<GifByteType>() == 1`,
        //   and by this function's `# Safety` precondition, `code_block[0]` is an initialized byte
        //   in a live allocation disjoint from `gif`.
        let len = unsafe { *code_block } as usize + 1;
        // SAFETY:
        // - Operation: `core::slice::from_raw_parts(code_block, len)`.
        // - Contract: `code_block` must be non-null, 1-byte aligned, point to `len` (`1..=256`)
        //   consecutive initialized `GifByteType` elements within a single live allocation
        //   (`len <= 256 <= isize::MAX as usize`), and not be mutated for the slice's lifetime.
        // - Evidence: `!code_block.is_null()` is checked above, `len` is at most
        //   `u8::MAX as usize + 1 == 256`, and by this function's `# Safety` precondition, all
        //   `len` bytes of the Pascal-style block lie in a single live allocation disjoint from
        //   `gif` and unmutated during this call.
        let block = unsafe { core::slice::from_raw_parts(code_block, len) };
        Some(block)
    };
    let r = encoder::egif_put_code_next_bytes(gif, block);
    gif.result_to_status(r)
}

/// EGifSpew — write an in-core representation of a GIF to the output.
///
/// This is a consume-and-close operation.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifSpew` (or `rs_EGifSpew` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifSpew(
    gif_file: Option<Box<GifFileType>>,
    error_code: Option<&mut c_int>,
) -> c_int {
    let Some(mut gif) = gif_file else {
        if let Some(error_code) = error_code {
            *error_code = E_GIF_ERR_NOT_WRITEABLE;
        }
        return GIF_ERROR;
    };

    let mut status = GIF_OK;
    let mut err = E_GIF_SUCCEEDED;

    // Write all data
    if let Err(e) = encoder::egif_spew(&mut gif) {
        status = GIF_ERROR;
        err = e as c_int;
    }

    // Close: write terminator
    if let Err(e) = encoder::egif_close_file(&mut gif) {
        status = GIF_ERROR;
        if err == E_GIF_SUCCEEDED {
            err = e as c_int;
        }
    }

    if let Some(error_code) = error_code {
        *error_code = if status == GIF_OK { E_GIF_SUCCEEDED } else { err };
    }
    status
}

// ---------------------------------------------------------------------------
//  GCB encoder helpers
// ---------------------------------------------------------------------------

/// EGifGCBToExtension — write a Graphics Control Block to an extension block.
// SAFETY: Exports the canonical giflib C ABI symbol `EGifGCBToExtension` (or
// `rs_EGifGCBToExtension` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib
// and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifGCBToExtension(
    gcb: Option<&GraphicsControlBlock>,
    gif_extension: Option<&mut [GifByteType; 4]>,
) -> usize {
    let Some(gcb) = gcb else {
        return 0;
    };
    let Some(ext) = gif_extension else {
        return 0;
    };
    encoder::egif_gcb_to_extension(gcb, ext);
    4
}

// SAFETY: Exports the canonical giflib C ABI symbol `EGifGCBToSavedExtension` (or
// `rs_EGifGCBToSavedExtension` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn EGifGCBToSavedExtension(
    gcb: Option<&GraphicsControlBlock>,
    gif_file: Option<&mut GifFileType>,
    image_index: c_int,
) -> c_int {
    let Some(gcb) = gcb else {
        return GIF_ERROR;
    };
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    match encoder::egif_gcb_to_saved_extension(gcb, gif, image_index) {
        Ok(()) => GIF_OK,
        Err(_) => GIF_ERROR,
    }
}

// ---------------------------------------------------------------------------
//  Font / drawing
// ---------------------------------------------------------------------------

/// Re-export the font table with its canonical C name.
// SAFETY: Exports the canonical giflib C ABI static `GifAsciiTable8x8`; `gif_guard_b` prevents
// linking alongside C giflib and `gif_signatures_test` verifies ABI type compatibility.
#[unsafe(no_mangle)]
pub static GifAsciiTable8x8: [[u8; font::GIF_FONT_WIDTH]; 128] = font::GIF_ASCII_TABLE_8X8;

// SAFETY: Exports the canonical giflib C ABI symbol `GifDrawText8x8` (or `rs_GifDrawText8x8` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifDrawText8x8(
    image: Option<&mut SavedImage>,
    x: c_int,
    y: c_int,
    legend: Option<CStrRef<'_>>,
    color: c_int,
) {
    let Some(image) = image else {
        return;
    };
    let Some(legend) = legend else {
        return;
    };
    let legend = legend.to_bytes();
    font::gif_draw_text8x8(image, x, y, legend, color);
}

/// Matches GifDrawBox in C — draws the outline of a rectangle.
// SAFETY: Exports the canonical giflib C ABI symbol `GifDrawBox` (or `rs_GifDrawBox` in
// `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and `gif_signatures_test`
// verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifDrawBox(
    image: Option<&mut SavedImage>,
    x: c_int,
    y: c_int,
    w: c_int,
    d: c_int,
    color: c_int,
) {
    let Some(image) = image else {
        return;
    };
    font::gif_draw_box(image, x, y, w, d, color);
}

/// Matches GifDrawRectangle in C — fills a rectangle.
// SAFETY: Exports the canonical giflib C ABI symbol `GifDrawRectangle` (or `rs_GifDrawRectangle`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifDrawRectangle(
    image: Option<&mut SavedImage>,
    x: c_int,
    y: c_int,
    w: c_int,
    d: c_int,
    color: c_int,
) {
    let Some(image) = image else {
        return;
    };
    font::gif_draw_rectangle(image, x, y, w, d, color);
}

// SAFETY: Exports the canonical giflib C ABI symbol `GifDrawBoxedText8x8` (or
// `rs_GifDrawBoxedText8x8` in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C
// giflib and `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn GifDrawBoxedText8x8(
    image: Option<&mut SavedImage>,
    x: c_int,
    y: c_int,
    legend: Option<CStrRef<'_>>,
    border: c_int,
    bg: c_int,
    fg: c_int,
) {
    let Some(image) = image else {
        return;
    };
    let Some(legend) = legend else {
        return;
    };
    let legend = legend.to_bytes();
    font::gif_draw_boxed_text8x8(image, x, y, legend, border, bg, fg);
}

// ---------------------------------------------------------------------------
//  Quantization
// ---------------------------------------------------------------------------

/// # Safety
///
/// The caller must ensure that:
/// - If `color_map_size` is `Some(cms)` with `1 <= *cms <= 256`, all five buffer pointers are
///   non-null, and `num_pixels = (width as usize).checked_mul(height as usize)` satisfies
///   `num_pixels <= isize::MAX as usize`:
///   - `red_input`, `green_input`, and `blue_input` each point to at least `num_pixels`
///     consecutive initialized `GifByteType` elements valid for reads within a single live
///     allocation.
///   - `output_buffer` points to at least `num_pixels` consecutive `GifByteType` elements valid
///     for writes within a single live allocation.
///   - `output_color_map` points to at least `*cms as usize` consecutive `GifColorType` elements
///     valid for writes within a single live allocation.
/// - For the duration of this call, `output_buffer`, `output_color_map`, and `color_map_size` do
///   not alias each other or any of `red_input`, `green_input`, or `blue_input`, and none of the
///   buffers are mutated by any other pointer or thread.
// SAFETY: Exports the canonical giflib C ABI symbol `GifQuantizeBuffer` (or `rs_GifQuantizeBuffer`
// in `gif_rs_prefixed`); `gif_guard_b` prevents linking alongside C giflib and
// `gif_signatures_test` verifies ABI signature compatibility.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifQuantizeBuffer(
    width: c_uint,
    height: c_uint,
    color_map_size: Option<&mut c_int>,
    red_input: *const GifByteType,
    green_input: *const GifByteType,
    blue_input: *const GifByteType,
    output_buffer: *mut GifByteType,
    output_color_map: *mut GifColorType,
) -> c_int {
    let Some(cms) = color_map_size else {
        return GIF_ERROR;
    };
    let Ok(cms_len) = usize::try_from(*cms) else {
        return GIF_ERROR;
    };
    let Some(num_pixels) = (width as usize).checked_mul(height as usize) else {
        return GIF_ERROR;
    };
    if !(1..=256).contains(&cms_len) || num_pixels > isize::MAX as usize {
        return GIF_ERROR;
    }

    if red_input.is_null()
        || green_input.is_null()
        || blue_input.is_null()
        || output_buffer.is_null()
        || output_color_map.is_null()
    {
        return GIF_ERROR;
    }

    // SAFETY:
    // - Operation: `core::slice::from_raw_parts(red_input, num_pixels)`.
    // - Contract: `red_input` must be non-null, 1-byte aligned, point to `num_pixels` consecutive
    //   initialized `GifByteType` elements in a single live allocation (`num_pixels <= isize::MAX as usize`),
    //   and not be mutated during the call.
    // - Evidence: `!red_input.is_null()` and `num_pixels <= isize::MAX as usize` are checked above,
    //   `align_of::<GifByteType>() == 1`, and the caller's `# Safety` precondition guarantees
    //   `red_input` points to `num_pixels` initialized bytes in a single allocation not aliased by
    //   any mutable buffer.
    let red = unsafe { core::slice::from_raw_parts(red_input, num_pixels) };
    // SAFETY:
    // - Operation: `core::slice::from_raw_parts(green_input, num_pixels)`.
    // - Contract & Evidence: Same as `red_input` above for `green_input`.
    let green = unsafe { core::slice::from_raw_parts(green_input, num_pixels) };
    // SAFETY:
    // - Operation: `core::slice::from_raw_parts(blue_input, num_pixels)`.
    // - Contract & Evidence: Same as `red_input` above for `blue_input`.
    let blue = unsafe { core::slice::from_raw_parts(blue_input, num_pixels) };
    // SAFETY:
    // - Operation: `core::slice::from_raw_parts_mut(output_buffer, num_pixels)`.
    // - Contract: `output_buffer` must be non-null, 1-byte aligned, point to `num_pixels`
    //   consecutive `GifByteType` elements valid for reads and writes in a single live allocation
    //   (`num_pixels <= isize::MAX as usize`), and be unaliased during the call.
    // - Evidence: `!output_buffer.is_null()` and `num_pixels <= isize::MAX as usize` are checked
    //   above, `align_of::<GifByteType>() == 1`, `quantize::gif_quantize_buffer` only writes into
    //   `out_buf` without reading uninitialized bytes (cf. UCG#414), and the caller's `# Safety`
    //   precondition guarantees `output_buffer` points to `num_pixels` writable bytes in a single
    //   live allocation disjoint from all other arguments.
    let out_buf = unsafe { core::slice::from_raw_parts_mut(output_buffer, num_pixels) };
    // SAFETY:
    // - Operation: `core::slice::from_raw_parts_mut(output_color_map, cms_len)`.
    // - Contract: `output_color_map` must be non-null, 1-byte aligned
    //   (`align_of::<GifColorType>() == 1`), point to `cms_len` (`1..=256`) consecutive
    //   `GifColorType` elements valid for reads and writes in a single live allocation
    //   (`cms_len * 3 <= 768 <= isize::MAX as usize`), and be unaliased during the call.
    // - Evidence: `!output_color_map.is_null()` and `(1..=256).contains(&cms_len)` are checked
    //   above, `quantize::gif_quantize_buffer` only writes into `out_cmap` without reading
    //   uninitialized bytes (cf. UCG#414), and the caller's `# Safety` precondition guarantees
    //   `output_color_map` points to at least `cms_len` writable `GifColorType` elements in a
    //   single live allocation disjoint from all other arguments.
    let out_cmap = unsafe { core::slice::from_raw_parts_mut(output_color_map, cms_len) };

    if quantize::gif_quantize_buffer(width, height, cms, red, green, blue, out_buf, out_cmap) {
        GIF_OK
    } else {
        GIF_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_quantize_buffer_rejects_out_of_range_color_map_size() {
        let rgb = [0u8; 4];
        let mut out_buf = [0u8; 4];
        let mut out_cmap = [GifColorType { Red: 0, Green: 0, Blue: 0 }; 256];

        for invalid_size in [-1, 0, 257] {
            let mut cms = invalid_size;
            // SAFETY:
            // - Operation: calling `GifQuantizeBuffer` with an out-of-range `cms`.
            // - Contract: When `*color_map_size` is outside `1..=256`, `GifQuantizeBuffer` returns
            //   `GIF_ERROR` before dereferencing any buffer pointers; moreover, all five pointers
            //   point to valid, disjoint stack arrays of 4 pixels and 256 colors.
            // - Evidence: `invalid_size` is outside `1..=256`, and `rgb`, `out_buf`, and
            //   `out_cmap` are live stack arrays that do not alias each other.
            let status = unsafe {
                GifQuantizeBuffer(
                    2,
                    2,
                    Some(&mut cms),
                    rgb.as_ptr(),
                    rgb.as_ptr(),
                    rgb.as_ptr(),
                    out_buf.as_mut_ptr(),
                    out_cmap.as_mut_ptr(),
                )
            };
            assert_eq!(status, GIF_ERROR);
            assert_eq!(cms, invalid_size);
        }
    }

    #[test]
    fn dgif_extension_to_gcb_rejects_invalid_extension_length() {
        let ext = [0u8; 8];
        let mut gcb = GraphicsControlBlock {
            DisposalMode: 0,
            UserInputFlag: false,
            DelayTime: 0,
            TransparentColor: -1,
        };

        for invalid_len in [0usize, 3, 5, usize::MAX] {
            // SAFETY:
            // - Operation: calling `DGifExtensionToGCB` with `gif_extension_length != 4`.
            // - Contract: When `gif_extension_length != 4`, `DGifExtensionToGCB` returns
            //   `GIF_ERROR` without reading from `gif_extension`.
            // - Evidence: `invalid_len != 4` holds for every tested length, and `ext` and `gcb`
            //   are live, disjoint stack values.
            let status = unsafe { DGifExtensionToGCB(invalid_len, ext.as_ptr(), Some(&mut gcb)) };
            assert_eq!(status, GIF_ERROR);
            assert_eq!(gcb.DelayTime, 0);
        }
    }
}
