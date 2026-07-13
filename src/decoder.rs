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

use crate::c_types::ReadCallback;
use crate::c_types::{
    ColorMapObject, ExtensionBlock, GifByteType, GifFileType, GifPixelType, GifRecordType, GifWord,
    GraphicsControlBlock, SavedImage, CONTINUE_EXT_FUNC_CODE, DISPOSAL_UNSPECIFIED, GIF87_STAMP,
    GIF89_STAMP, GIF_STAMP, GIF_VERSION_POS, GRAPHICS_EXT_FUNC_CODE, NO_TRANSPARENT_COLOR,
};
use crate::decoder_lzw::{dgif_decompress_input, dgif_decompress_line, dgif_setup_decompress};
use crate::err::GifError;
use crate::private::{GifFilePrivateType, IoState};
use crate::private_c_types::{DESCRIPTOR_INTRODUCER, EXTENSION_INTRODUCER, TERMINATOR_INTRODUCER};
use core::ffi::c_int;
use core::ptr;
use safer_cffi::CSlicePtr;

// ---------------------------------------------------------------------------
//  Open helpers
// ---------------------------------------------------------------------------

/// Open a GIF file by filename, returning the initialized `GifFileType`.
///
/// This is the inner logic for `DGifOpenFileName`.
pub fn dgif_open_file_name(file_name: &core::ffi::CStr) -> Result<Box<GifFileType>, GifError> {
    let path = file_name.to_str().map_err(|_| GifError::DOpenFailed)?;

    let file = std::fs::File::open(path).map_err(|_| GifError::DOpenFailed)?;

    dgif_open(Some(file), None, ptr::null_mut())
}

/// Create and initialize a `GifFileType` from the given I/O source.
///
/// Validates the GIF stamp and reads the screen descriptor.
pub(crate) fn dgif_open(
    file: Option<std::fs::File>,
    read_fn: Option<ReadCallback>,
    user_data: *mut core::ffi::c_void,
) -> Result<Box<GifFileType>, GifError> {
    let io = if let Some(read_fn) = read_fn {
        IoState::CallbackRead { read_fn }
    } else if let Some(file) = file {
        IoState::FileRead { file }
    } else {
        return Err(GifError::DOpenFailed);
    };
    let private = Box::new(GifFilePrivateType::new(io));

    let mut gif = Box::new(GifFileType::new(private, user_data));

    // Read and validate the GIF stamp
    let mut buf = [0u8; GIF_STAMP.len() - 1];
    if gif.read(&mut buf) != buf.len() {
        return Err(GifError::DReadFailed);
    }

    // Check "GIF" prefix
    if &buf[..GIF_VERSION_POS as usize] != &GIF_STAMP[..GIF_VERSION_POS as usize] {
        return Err(GifError::DNotGifFile);
    }

    // Detect GIF89a
    gif.Private.gif89 = buf == GIF89_STAMP[..GIF89_STAMP.len() - 1];

    // Read screen descriptor
    dgif_get_screen_desc(&mut gif).map_err(|_| GifError::DNoScreenDesc)?;

    Ok(gif)
}

// ---------------------------------------------------------------------------
//  Internal I/O
// ---------------------------------------------------------------------------

impl GifFileType {
    /// Read bytes from file or callback into an external buffer.
    /// Matches `InternalRead` in C.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> usize {
        let gif_ptr = self as *mut GifFileType;
        self.Private.io.read(gif_ptr, buf)
    }

    /// Read exactly `buf.len()` bytes or return `ReadFailed`.
    pub(crate) fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), GifError> {
        if self.read(buf) != buf.len() {
            Err(GifError::DReadFailed)
        } else {
            Ok(())
        }
    }
}

/// Read a little-endian 16-bit word. Matches DGifGetWord in C.
pub fn dgif_get_word(gif: &mut GifFileType) -> Result<GifWord, GifError> {
    let mut c = [0u8; 2];
    gif.read_exact(&mut c)?;
    Ok(u16::from_le_bytes(c) as GifWord)
}

// ---------------------------------------------------------------------------
//  Screen descriptor
// ---------------------------------------------------------------------------

/// Matches DGifGetScreenDesc in C.
pub fn dgif_get_screen_desc(gif: &mut GifFileType) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }

    // Read width and height
    gif.SWidth = dgif_get_word(gif)?;
    gif.SHeight = dgif_get_word(gif)?;

    let mut buf3 = [0u8; 3];
    if gif.read(&mut buf3) != 3 {
        gif.SColorMap = None;
        return Err(GifError::DReadFailed);
    }

    // NOTE: Matches the C code exactly, including its precedence quirk.
    // The spec says ((Buf[0] >> 4) & 7) + 1, but the original C does
    // (((Buf[0] & 0x70) + 1) >> 4) + 1.  We preserve the C behavior.
    gif.SColorResolution = ((((buf3[0] as i32) & 0x70) + 1) >> 4) + 1;
    let sort_flag = (buf3[0] & 0x08) != 0;
    let bits_per_pixel = (buf3[0] & 0x07) + 1;
    gif.SBackGroundColor = buf3[1] as GifWord;
    gif.AspectByte = buf3[2];

    if (buf3[0] & 0x80) != 0 {
        // Global color map present
        let color_count = 1i32 << bits_per_pixel;
        let mut map = ColorMapObject::new(color_count).map_err(|_| GifError::DNotEnoughMem)?;

        // GOOGLE MODIFICATION: clamp background color
        if gif.SBackGroundColor >= map.ColorCount as GifWord {
            gif.SBackGroundColor = map.ColorCount as GifWord - 1;
        }
        map.SortFlag = sort_flag;

        // Read color table entries.
        for i in 0..map.ColorCount as usize {
            let mut rgb = [0u8; 3];
            if gif.read(&mut rgb) != 3 {
                gif.SColorMap = None;
                return Err(GifError::DReadFailed);
            }
            let c = &mut map.colors_mut()[i];
            c.Red = rgb[0];
            c.Green = rgb[1];
            c.Blue = rgb[2];
        }
        gif.SColorMap = Some(Box::new(map));
    } else {
        gif.SColorMap = None;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
//  Version
// ---------------------------------------------------------------------------

pub fn dgif_get_gif_version(gif: &GifFileType) -> &'static [u8; 7] {
    if gif.Private.gif89 {
        GIF89_STAMP
    } else {
        GIF87_STAMP
    }
}

// ---------------------------------------------------------------------------
//  Record type
// ---------------------------------------------------------------------------
pub fn dgif_get_record_type(gif: &mut GifFileType) -> Result<GifRecordType, GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }

    let mut byte_buf = [0u8; 1];
    gif.read_exact(&mut byte_buf)?;

    match byte_buf[0] {
        DESCRIPTOR_INTRODUCER => Ok(GifRecordType::IMAGE_DESC_RECORD_TYPE),
        EXTENSION_INTRODUCER => Ok(GifRecordType::EXTENSION_RECORD_TYPE),
        TERMINATOR_INTRODUCER => Ok(GifRecordType::TERMINATE_RECORD_TYPE),
        _ => Err(GifError::DWrongRecord),
    }
}

// ---------------------------------------------------------------------------
//  Image header / descriptor
// ---------------------------------------------------------------------------

/// Matches DGifGetImageHeader in C.
pub fn dgif_get_image_header(gif: &mut GifFileType) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }

    gif.Image.Left = dgif_get_word(gif)?;
    gif.Image.Top = dgif_get_word(gif)?;
    gif.Image.Width = dgif_get_word(gif)?;
    gif.Image.Height = dgif_get_word(gif)?;

    let mut flags = [0u8; 1];
    if gif.read(&mut flags) != 1 {
        gif.Image.ColorMap = None;
        return Err(GifError::DReadFailed);
    }

    let bits_per_pixel = (flags[0] & 0x07) + 1;
    gif.Image.Interlace = (flags[0] & 0x40) != 0;

    // Free any existing local color map
    gif.Image.ColorMap = None;

    // Local color map?
    if (flags[0] & 0x80) != 0 {
        let color_count = 1i32 << bits_per_pixel;
        let mut map = ColorMapObject::new(color_count).map_err(|_| GifError::DNotEnoughMem)?;
        for i in 0..map.ColorCount as usize {
            let mut rgb = [0u8; 3];
            if gif.read(&mut rgb) != 3 {
                gif.Image.ColorMap = None;
                return Err(GifError::DReadFailed);
            }
            let c = &mut map.colors_mut()[i];
            c.Red = rgb[0];
            c.Green = rgb[1];
            c.Blue = rgb[2];
        }
        gif.Image.ColorMap = Some(Box::new(map));
    }

    let pixel_count = (gif.Image.Width as usize)
        .checked_mul(gif.Image.Height as usize)
        .ok_or(GifError::DDataTooBig)?;
    gif.Private.pixel_count = pixel_count;

    // Setup LZW decompression
    dgif_setup_decompress(gif)
}

/// Matches DGifGetImageDesc in C. Calls DGifGetImageHeader, then saves image.
pub fn dgif_get_image_desc(gif: &mut GifFileType) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }

    dgif_get_image_header(gif)?;

    // Create new SavedImage locally and initialize.
    let image = SavedImage::new(gif.Image.clone());

    gif.saved_images_mut().add(image);

    Ok(())
}

// ---------------------------------------------------------------------------
//  Line / Pixel reading
// ---------------------------------------------------------------------------

/// Drain remaining sub-blocks from the stream after image data is complete.
/// Shared helper for dgif_get_line, dgif_get_pixel, and dgif_get_lz_codes.
fn flush_remaining_blocks(gif: &mut GifFileType) -> Result<(), GifError> {
    while dgif_get_code_next(gif)? {}
    Ok(())
}

pub fn dgif_get_line(gif: &mut GifFileType, line: &mut [GifPixelType]) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }

    let line_len = line.len();

    if gif.Private.pixel_count < line_len {
        return Err(GifError::DDataTooBig);
    }
    gif.Private.pixel_count -= line_len;

    dgif_decompress_line(gif, line)?;
    if gif.Private.pixel_count == 0 {
        flush_remaining_blocks(gif)?;
    }
    Ok(())
}

/// Matches DGifGetPixel in C.
pub fn dgif_get_pixel(gif: &mut GifFileType, pixel: &mut GifPixelType) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }
    if gif.Private.pixel_count == 0 {
        return Err(GifError::DDataTooBig);
    }
    gif.Private.pixel_count -= 1;

    let mut px = [0u8];
    dgif_decompress_line(gif, &mut px)?;
    *pixel = px[0];
    if gif.Private.pixel_count == 0 {
        flush_remaining_blocks(gif)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
//  Extensions
// ---------------------------------------------------------------------------

/// Matches DGifGetExtension in C.
pub fn dgif_get_extension(
    gif: &mut GifFileType,
    ext_code: &mut c_int,
    extension: &mut *mut GifByteType,
) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }

    let mut byte_buf = [0u8; 1];
    gif.read_exact(&mut byte_buf)?;
    *ext_code = byte_buf[0] as c_int;

    match dgif_get_extension_next(gif)? {
        true => *extension = gif.Private.lzw.buf.as_mut_ptr(),
        false => *extension = ptr::null_mut(),
    }
    Ok(())
}

/// Read one GIF sub-block into `gif.Private.lzw.buf` (Pascal string layout: buf[0] = length).
/// Returns `Ok(true)` if data was read, `Ok(false)` on a zero-length terminator block.
fn read_subblock(gif: &mut GifFileType) -> Result<bool, GifError> {
    let mut len_buf = [0u8; 1];
    gif.read_exact(&mut len_buf)?;
    let block_len = len_buf[0];
    if block_len > 0 {
        gif.Private.lzw.buf[0] = block_len;
        let gif_ptr = gif as *mut GifFileType;
        if gif.Private.io.read(gif_ptr, &mut gif.Private.lzw.buf[1..1 + block_len as usize])
            != block_len as usize
        {
            return Err(GifError::DReadFailed);
        }
        Ok(true)
    } else {
        gif.Private.lzw.buf[0] = 0;
        Ok(false)
    }
}

/// Reads the next extension sub-block.
/// Returns `Ok(true)` if data was read into `gif.Private.lzw.buf`, `Ok(false)` on terminator.
pub fn dgif_get_extension_next(gif: &mut GifFileType) -> Result<bool, GifError> {
    read_subblock(gif)
}

// ---------------------------------------------------------------------------
//  Raw code access
// ---------------------------------------------------------------------------

/// Matches DGifGetCode in C.
pub fn dgif_get_code(
    gif: &mut GifFileType,
    code_size: &mut c_int,
    code_block: &mut *mut GifByteType,
) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }
    *code_size = gif.Private.lzw.bits_per_pixel;
    match dgif_get_code_next(gif)? {
        true => *code_block = gif.Private.lzw.buf.as_mut_ptr(),
        false => *code_block = ptr::null_mut(),
    }
    Ok(())
}

/// Reads the next code sub-block.
/// Returns `Ok(true)` if data was read into `gif.Private.lzw.buf`, `Ok(false)` on terminator.
pub fn dgif_get_code_next(gif: &mut GifFileType) -> Result<bool, GifError> {
    match read_subblock(gif)? {
        true => Ok(true),
        false => {
            gif.Private.lzw.buf[0] = 0;
            gif.Private.pixel_count = 0;
            Ok(false)
        }
    }
}

// ---------------------------------------------------------------------------
//  LZ codes
// ---------------------------------------------------------------------------

/// Matches DGifGetLZCodes in C.
pub fn dgif_get_lz_codes(gif: &mut GifFileType, code: &mut c_int) -> Result<(), GifError> {
    if !gif.Private.io.is_readable() {
        return Err(GifError::DNotReadable);
    }

    dgif_decompress_input(gif, code)?;

    if *code == gif.Private.lzw.eof_code {
        // Skip remaining blocks
        flush_remaining_blocks(gif)?;
        *code = -1;
    } else if *code == gif.Private.lzw.clear_code {
        gif.Private.lzw.running_code = gif.Private.lzw.eof_code + 1;
        gif.Private.lzw.running_bits = gif.Private.lzw.bits_per_pixel + 1;
        gif.Private.lzw.max_code1 = 1 << gif.Private.lzw.running_bits;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
//  GCB extraction
// ---------------------------------------------------------------------------

/// Matches DGifExtensionToGCB in C.
pub fn dgif_extension_to_gcb(
    ext: &[GifByteType],
    gcb: &mut GraphicsControlBlock,
) -> Result<(), GifError> {
    if ext.len() != 4 {
        return Err(GifError::DImageDefect);
    }
    gcb.DisposalMode = ((ext[0] >> 2) & 0x07) as c_int;
    gcb.UserInputFlag = (ext[0] & 0x02) != 0;
    gcb.DelayTime = (ext[1] as c_int) | ((ext[2] as c_int) << 8);
    if (ext[0] & 0x01) != 0 {
        gcb.TransparentColor = ext[3] as c_int;
    } else {
        gcb.TransparentColor = NO_TRANSPARENT_COLOR;
    }
    Ok(())
}

/// Matches DGifSavedExtensionToGCB in C.
pub fn dgif_saved_extension_to_gcb(
    gif: &GifFileType,
    image_index: c_int,
    gcb: &mut GraphicsControlBlock,
) -> Result<(), GifError> {
    if image_index < 0 || image_index > gif.ImageCount - 1 {
        return Err(GifError::DNoImageDesc);
    }

    gcb.DisposalMode = DISPOSAL_UNSPECIFIED;
    gcb.UserInputFlag = false;
    gcb.DelayTime = 0;
    gcb.TransparentColor = NO_TRANSPARENT_COLOR;

    let saved = &gif.saved_images()[image_index as usize];
    for ep in saved.extension_blocks() {
        if ep.Function == GRAPHICS_EXT_FUNC_CODE {
            return dgif_extension_to_gcb(ep.bytes(), gcb);
        }
    }

    Err(GifError::DNoImageDesc)
}

/// Extract the payload from a GIF sub-block stored in Pascal string layout
/// (buf[0] = length, buf[1..=len] = data). Returns `None` if length is zero.
fn buf_payload(buf: &[u8]) -> Option<&[u8]> {
    let len = buf[0] as usize;
    if len == 0 {
        return None;
    }
    Some(&buf[1..1 + len])
}

/// Matches DGifSlurp in C. Reads entire GIF into core.
pub fn dgif_slurp(gif: &mut GifFileType) -> Result<(), GifError> {
    gif.ExtensionBlocks = CSlicePtr::null();
    gif.ExtensionBlockCount = 0;

    loop {
        let record_type = dgif_get_record_type(gif)?;

        match record_type {
            GifRecordType::IMAGE_DESC_RECORD_TYPE => {
                // Read the image header (populates gif.Image) without adding
                // to SavedImages yet.  We build the SavedImage locally and
                // only commit it on success, so error paths just drop it.
                dgif_get_image_header(gif)?;

                let mut sp = SavedImage::new(gif.Image.clone());

                // Validate dimensions
                if sp.ImageDesc.Width <= 0
                    || sp.ImageDesc.Height <= 0
                    || sp.ImageDesc.Width > (i32::MAX / sp.ImageDesc.Height)
                {
                    return Err(GifError::DDataTooBig);
                }

                // GOOGLE MODIFICATION: size limits
                let image_size = (sp.ImageDesc.Width as usize)
                    .checked_mul(sp.ImageDesc.Height as usize)
                    .ok_or(GifError::DDataTooBig)?;
                if image_size > ((1usize << 31) / core::mem::size_of::<GifPixelType>()) {
                    return Err(GifError::DDataTooBig);
                }

                sp.RasterBits = unsafe {
                    // SAFETY: Rust allocator is compatible with the C allocator.
                    CSlicePtr::from_raw(
                        Box::into_raw(vec![0; image_size].into_boxed_slice()) as *mut u8
                    )
                };

                if sp.ImageDesc.Interlace {
                    static INTERLACED_OFFSET: [c_int; 4] = [0, 4, 2, 1];
                    static INTERLACED_JUMPS: [c_int; 4] = [8, 8, 4, 2];

                    for pass in 0..4 {
                        let mut j = INTERLACED_OFFSET[pass];
                        while j < sp.ImageDesc.Height {
                            let line_slice =
                                sp.raster_bits_row_mut(j as usize).ok_or(GifError::DDataTooBig)?;
                            dgif_get_line(gif, line_slice)?;
                            j += INTERLACED_JUMPS[pass];
                        }
                    }
                } else {
                    let raster_slice = sp.raster_bits_mut();
                    dgif_get_line(gif, raster_slice)?;
                }

                // Move pending extension blocks to this image
                assert!(sp.extension_blocks().is_empty());
                gif.extension_blocks_mut().swap(&mut sp.extension_blocks_mut());

                // Commit the fully-built image.
                gif.saved_images_mut().add(sp);
            }
            GifRecordType::EXTENSION_RECORD_TYPE => {
                let mut ext_function: c_int = 0;
                let mut ext_data_unused: *mut GifByteType = ptr::null_mut();

                dgif_get_extension(gif, &mut ext_function, &mut ext_data_unused)?;

                if let Some(data) = buf_payload(&gif.Private.lzw.buf) {
                    let block = ExtensionBlock::new(ext_function, data);
                    gif.extension_blocks_mut().add(block);
                }

                loop {
                    match dgif_get_extension_next(gif)? {
                        true => {}
                        false => break,
                    }
                    let Some(data) = buf_payload(&gif.Private.lzw.buf) else {
                        break;
                    };
                    let block = ExtensionBlock::new(CONTINUE_EXT_FUNC_CODE, data);
                    gif.extension_blocks_mut().add(block);
                }
            }
            GifRecordType::TERMINATE_RECORD_TYPE => break,
            GifRecordType::UNDEFINED_RECORD_TYPE | GifRecordType::SCREEN_DESC_RECORD_TYPE => {
                unreachable!()
            }
        }
    }

    // Sanity check
    if gif.ImageCount == 0 {
        return Err(GifError::DNoImageDesc);
    }

    Ok(())
}
