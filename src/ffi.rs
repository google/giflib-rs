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
/// * file_handle must be a valid, opened file descriptor and ownership must be passed as part of
///   this method call.
unsafe fn make_file(file_handle: c_int) -> Option<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        if file_handle >= 0 {
            // SAFETY: Per function contract, file_handle is a valid file descriptor.
            // Caller transfers ownership of the file descriptor.
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
            // SAFETY: Per function contract, file_handle is a valid file descriptor.
            let handle = unsafe { libc::get_osfhandle(file_handle) };
            if handle == -1 {
                None
            } else {
                // SAFETY: fd is non-negative; caller transfers ownership.
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
        // SAFETY: Per function contract, file_handle is a valid file descriptor
        // that this function takes ownership of, so it is safe to close.
        unsafe {
            let _ = libc::close(file_handle);
        }
        None
    }
}

/// DGifOpenFileHandle — open a GIF from a raw file descriptor.
///
/// # Safety
///
/// This function **takes ownership** of `file_handle` via `File::from_raw_fd`.
/// The file descriptor will be closed when the `GifFileType` is closed via
/// `DGifCloseFile`. The caller MUST NOT close the fd separately.
/// This matches the C original's use of `fdopen()`, which similarly takes
/// ownership of the descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifOpenFileHandle(
    file_handle: c_int,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    // SAFETY: Per function contract, `file_handle` is a valid file descriptor.
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
/// Caller must guarantee that `read_func` points to a valid function that
/// implements the ReadCallback contract. If `user_data` is non-null, it must
/// be valid for the lifetime of the returned `GifFileType` struct (i.e., until
/// `DGifCloseFile` is called), as it may be accessed by `read_func` via
/// `gif->UserData`.
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
        // SAFETY: The caller guarantees that `f` upholds the ReadCallback contract.
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

#[unsafe(no_mangle)]
pub extern "C" fn DGifGetScreenDesc(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_get_screen_desc(gif);
    gif.result_to_status(r)
}

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

#[unsafe(no_mangle)]
pub extern "C" fn DGifGetImageHeader(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_get_image_header(gif);
    gif.result_to_status(r)
}

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
/// Caller must guarantee that `line` points to a buffer of at least `line_len` bytes that is
/// valid for writes for the duration of the function call.
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
    // SAFETY: caller guarantees safety invariants on `line` and `line_len`.
    let line_slice = unsafe { core::slice::from_raw_parts_mut(line, len) };
    let r = decoder::dgif_get_line(gif, line_slice);
    gif.result_to_status(r)
}

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

#[unsafe(no_mangle)]
pub extern "C" fn DGifSlurp(gif_file: Option<&mut GifFileType>) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = decoder::dgif_slurp(gif);
    gif.result_to_status(r)
}

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
/// Caller must guarantee that `gif_extension` points to a buffer of at least
/// `gif_extension_length` bytes that is valid for reads for the duration of the
/// function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DGifExtensionToGCB(
    gif_extension_length: usize,
    gif_extension: *const GifByteType,
    gcb: Option<&mut GraphicsControlBlock>,
) -> c_int {
    let Some(gcb) = gcb else {
        return GIF_ERROR;
    };
    if gif_extension.is_null() {
        return GIF_ERROR;
    }
    // SAFETY: caller guarantees gif_extension points to gif_extension_length bytes.
    let ext = unsafe { core::slice::from_raw_parts(gif_extension, gif_extension_length) };
    match decoder::dgif_extension_to_gcb(ext, gcb) {
        Ok(()) => GIF_OK,
        Err(_) => GIF_ERROR,
    }
}

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
/// If `color_map` is non-null, it must reside within a single allocated object, point to at least
/// `color_count` initialized, valid `GifColorType` elements, and not be mutated by any other
/// pointer or thread for the duration of the function call. Alignment is guaranteed implicitly
/// by the fact that `GifColorType` is 1-byte aligned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifMakeMapObject(
    color_count: c_int,
    color_map: *const GifColorType,
) -> Option<Box<ColorMapObject>> {
    let mut object = ColorMapObject::new(color_count).ok()?;
    if !color_map.is_null() {
        let colors = object.colors_mut();
        // SAFETY:
        // - `color_map` is checked non-null above.
        // - By function `# Safety` contract, `color_map` is aligned (align_of::<GifColorType>() ==
        //   1), points to at least `color_count` initialized `GifColorType` elements in a single
        //   allocation, and is not concurrently mutated during the call.
        // - `colors.len()` equals `color_count` as allocated by `ColorMapObject::new(color_count)`.
        // - `colors` is a freshly allocated buffer disjoint from `color_map`, so they do not alias.
        colors.copy_from_slice(unsafe { core::slice::from_raw_parts(color_map, colors.len()) });
    }
    Some(Box::new(object))
}

#[unsafe(no_mangle)]
pub extern "C" fn GifFreeMapObject(object: Option<Box<ColorMapObject>>) {
    drop(object);
}

/// GifUnionColorMap — compute the union of two color map objects.
///
/// # Safety
///
/// Caller must guarantee that `color_trans_in2` points to a valid array of
/// `GifPixelType` of at least `color_in2->ColorCount` elements for the duration
/// of the function call.
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
    // - `color_trans_in2` is non-null (checked above) and aligned (`align_of::<GifPixelType>() ==
    //   1`).
    // - By C API contract, `color_trans_in2` points to at least `trans_len` elements in a single
    //   allocation valid for writes.
    // - The caller guarantees exclusive access to `color_trans_in2` (does not alias `color_in1` or
    //   `color_in2`).
    let trans = unsafe { core::slice::from_raw_parts_mut(color_trans_in2, trans_len) };

    color_in1.union_with(color_in2, trans).map(Box::new)
}

#[unsafe(no_mangle)]
pub extern "C" fn GifBitSize(n: c_int) -> c_int {
    ColorMapObject::bit_size(n)
}

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
/// If `ext_data` is non-null, it must point to a buffer of at least
/// `len` bytes that is valid for reads for the duration of the function call.
/// `*extension_blocks` must be null or a malloc-allocated array of `*extension_block_count`
/// initialized `ExtensionBlock`s.
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
        // SAFETY: caller guarantees ext_data points to at least `len` bytes.
        unsafe { core::slice::from_raw_parts(ext_data, len as usize) }
    };
    let Ok(block) = ExtensionBlock::try_new(function, data) else {
        return GIF_ERROR;
    };
    // SAFETY: the length of `*extension_blocks` is `*extension_block_count`.
    match unsafe { extension_blocks.as_vec_mut(extension_block_count) }.try_push_back(block) {
        Ok(()) => GIF_OK,
        Err(_) => GIF_ERROR,
    }
}

/// GifFreeExtensions — free the memory allocated for the extension blocks.
///
/// # Safety
///
/// `*extension_blocks` must be null or a malloc-allocated array of `*extension_block_count`
/// initialized `ExtensionBlock`s.
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
    // SAFETY: the length of `*extension_blocks` is `*extension_block_count`.
    unsafe { extension_blocks.as_vec_mut(extension_block_count) }.clear();
}

/// GifMakeSavedImage — append an image to `gif_file->SavedImages`, copying `copy_from` if
/// non-null.
///
/// # Safety
///
/// `copy_from` must be null or point to a valid `SavedImage`, which may be an element of
/// `gif_file->SavedImages`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GifMakeSavedImage<'a>(
    gif_file: Option<&'a mut GifFileType>,
    copy_from: *const SavedImage,
) -> Option<&'a mut SavedImage> {
    let gif = gif_file?;

    // `copy_from` may point into `gif.SavedImages`, which `try_push_back` reallocates. It is a raw
    // pointer because a reference parameter would have to stay valid until the function returns.
    // SAFETY: the caller guarantees that `copy_from` is null or points to a valid `SavedImage`. The
    // temporary `&SavedImage` is dropped at the end of the `match` after cloning, before
    // `try_push_back` reallocates `gif.SavedImages`.
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

#[unsafe(no_mangle)]
pub extern "C" fn GifFreeSavedImages(gif_file: Option<&mut GifFileType>) {
    if let Some(gif) = gif_file {
        gif.saved_images_vec().clear();
    }
}

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
/// This function **takes ownership** of `file_handle` via `File::from_raw_fd`.
/// The file descriptor will be closed when the `GifFileType` is closed via
/// `EGifCloseFile`. The caller MUST NOT close the fd separately.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifOpenFileHandle(
    file_handle: c_int,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    // SAFETY: Per function contract, `file_handle` is a valid file descriptor.
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
/// Caller must guarantee that `write_func` points to a valid function that
/// upholds the WriteCallback contract for the duration of the function call and
/// until the GIF is closed via `EGifCloseFile`. If `user_data` is non-null, it
/// must be valid for the lifetime of the returned `GifFileType` struct (i.e.,
/// until `EGifCloseFile` is called), as it may be accessed by `write_func` via
/// `gif->UserData`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EGifOpen(
    user_data: *mut c_void,
    write_func: OutputFunc,
    error: Option<&mut c_int>,
) -> Option<Box<GifFileType>> {
    let callback = write_func.map(|f| {
        // SAFETY: The caller guarantees that `f` upholds the WriteCallback contract.
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

#[unsafe(no_mangle)]
pub extern "C" fn EGifGetGifVersion(gif_file: Option<&mut GifFileType>) -> *const c_char {
    let Some(gif) = gif_file else {
        return GIF87_STAMP.as_ptr() as *const c_char;
    };
    encoder::egif_get_gif_version(gif).as_ptr() as *const c_char
}

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
/// `color_map` must be null or point to a valid `ColorMapObject`. It may be one of `gif_file`'s
/// own maps, e.g. `gif_file->SColorMap` or `gif_file->Image.ColorMap`.
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
        // SAFETY: the caller guarantees that `color_map` is null or points to a valid
        // `ColorMapObject`. It may be owned by `gif` (e.g. `gif->Image.ColorMap`), so the
        // reference only lives for the copy, before `gif` is used again.
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
/// `color_map` must be null or point to a `ColorMapObject` with valid `ColorCount`, `BitsPerPixel`,
/// and `Colors`. It may be one of `gif_file`'s own maps, e.g. `gif_file->Image.ColorMap` or
/// `gif_file->SColorMap`.
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
            // SAFETY: the caller guarantees that `color_map` is null or points to a
            // `ColorMapObject` with valid `ColorCount`, `BitsPerPixel`, and `Colors` fields.
            // It may be owned by `gif` (e.g. `gif->SColorMap` or a saved
            // image's map), so the reference only lives for the copy, before `gif` is used
            // again. Some callers may pass in uninitialized maps for SortFlag (which is never
            // read in C), so we copy only the initialized fields.
            // Partially uninit references are not UB as long as uninitialized fields are not
            // read, cf. https://github.com/rust-lang/unsafe-code-guidelines/issues/414.
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
/// The caller must guarantee that `line` points to at least `line_len` bytes.
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
    // SAFETY: caller guarantees `line` points to at least `len` bytes.
    // When len is 0, we pass an empty slice; egif_put_line defaults to Image.Width.
    let line_slice = if len == 0 || line.is_null() {
        &mut []
    } else {
        unsafe { core::slice::from_raw_parts_mut(line, len) }
    };
    let r = encoder::egif_put_line(gif, line_slice);
    gif.result_to_status(r)
}

#[unsafe(no_mangle)]
pub extern "C" fn EGifPutPixel(gif_file: Option<&mut GifFileType>, pixel: GifPixelType) -> c_int {
    let Some(gif) = gif_file else {
        return GIF_ERROR;
    };
    let r = encoder::egif_put_pixel(gif, pixel);
    gif.result_to_status(r)
}

/// EGifPutComment — write a comment to the GIF.
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
/// The caller must guarantee that `extension` points to at least `ext_len` bytes.
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
        // SAFETY: caller guarantees extension points to at least ext_len bytes.
        unsafe { core::slice::from_raw_parts(extension as *const u8, ext_len as usize) }
    };
    let r = encoder::egif_put_extension_block(gif, ext_data);
    gif.result_to_status(r)
}

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
/// The caller must guarantee that `extension` points to at least `ext_len` bytes.
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
        // SAFETY: caller guarantees extension points to at least ext_len bytes.
        unsafe { core::slice::from_raw_parts(extension as *const u8, ext_len as usize) }
    };
    let r = encoder::egif_put_extension(gif, ext_code, ext_data);
    gif.result_to_status(r)
}

/// EGifPutCode
///
/// # Safety
///
/// The caller must guarantee that `code_block` is a valid Pascal-string, i.e.
/// `code_block[0]` is the length of the string and `code_block[1..]` are the
/// bytes of the string, and that the pointer is valid for at least that many
/// bytes.
///
/// The `code_size` parameter is unused and should be ignored. This behavior
/// matches the C library.
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
    // SAFETY: code_block is non-null (checked above). Pascal-string: block[0] = len.
    let len = unsafe { *code_block } as usize + 1;
    let block = unsafe { core::slice::from_raw_parts(code_block, len) };
    let r = encoder::egif_put_code(gif, block);
    gif.result_to_status(r)
}

/// EGifPutCodeNext
///
/// # Safety
///
/// The caller must guarantee that `code_block` is a valid Pascal-string, i.e.
/// `code_block[0]` is the length of the string and `code_block[1..]` are the
/// bytes of the string, and that the pointer is valid for at least that many
/// bytes.
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
        // SAFETY: code_block is non-null (checked above). Pascal-string: block[0] = len.
        let len = unsafe { *code_block } as usize + 1;
        let block = unsafe { core::slice::from_raw_parts(code_block, len) };
        Some(block)
    };
    let r = encoder::egif_put_code_next_bytes(gif, block);
    gif.result_to_status(r)
}

/// EGifSpew — write an in-core representation of a GIF to the output.
///
/// This is a consume-and-close operation.
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
#[unsafe(no_mangle)]
pub static GifAsciiTable8x8: [[u8; font::GIF_FONT_WIDTH]; 128] = font::GIF_ASCII_TABLE_8X8;

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
/// The caller must guarantee that `red_input`, `green_input`, `blue_input`, and
/// `output_buffer` point to buffers of size at least `width * height`, and that
/// `output_color_map` points to a buffer of size at least `*color_map_size`.
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
    let Some(num_pixels) = (width as usize).checked_mul(height as usize) else {
        return GIF_ERROR;
    };

    if red_input.is_null()
        || green_input.is_null()
        || blue_input.is_null()
        || output_buffer.is_null()
        || output_color_map.is_null()
    {
        return GIF_ERROR;
    }

    // SAFETY: caller guarantees each input buffer has at least width*height elements.
    let red = unsafe { core::slice::from_raw_parts(red_input, num_pixels) };
    let green = unsafe { core::slice::from_raw_parts(green_input, num_pixels) };
    let blue = unsafe { core::slice::from_raw_parts(blue_input, num_pixels) };
    let out_buf = unsafe { core::slice::from_raw_parts_mut(output_buffer, num_pixels) };
    // SAFETY: caller guarantees output_color_map has at least cms entries.
    let out_cmap = unsafe { core::slice::from_raw_parts_mut(output_color_map, *cms as usize) };

    if quantize::gif_quantize_buffer(width, height, cms, red, green, blue, out_buf, out_cmap) {
        GIF_OK
    } else {
        GIF_ERROR
    }
}
