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

//! GIF encoder logic — bug-for-bug reimplementation of egif_lib.c.
//!
//! Contains all encoder functions. The encoder-specific private state
//! (write callback, hash table) lives in `GifFilePrivateType` in decoder.rs.

use crate::c_types::WriteCallback;
use crate::c_types::{
    ColorMapObject, ExtensionBlock, GifByteType, GifFileType, GifPixelType, GifWord,
    GraphicsControlBlock, APPLICATION_EXT_FUNC_CODE, COMMENT_EXT_FUNC_CODE, CONTINUE_EXT_FUNC_CODE,
    GIF87_STAMP, GIF89_STAMP, GRAPHICS_EXT_FUNC_CODE, NO_TRANSPARENT_COLOR,
    PLAINTEXT_EXT_FUNC_CODE,
};
use crate::encoder_lzw;
use crate::err::GifError;
use crate::private::IoState;
use crate::private_c_types::{DESCRIPTOR_INTRODUCER, EXTENSION_INTRODUCER, TERMINATOR_INTRODUCER};
use core::ffi::c_int;
use core::ptr;

// ---------------------------------------------------------------------------
//  Internal I/O
// ---------------------------------------------------------------------------

impl GifFileType {
    /// Write bytes to file or callback from a buffer.
    /// Matches `InternalWrite` in C.
    pub(crate) fn write(&mut self, buf: &[u8]) -> usize {
        let gif_ptr = self as *mut GifFileType;
        self.Private.io.write(gif_ptr, buf)
    }

    /// Write exactly `buf.len()` bytes or return `WriteFailed`.
    pub(crate) fn write_exact(&mut self, buf: &[u8]) -> Result<(), GifError> {
        if self.write(buf) != buf.len() {
            Err(GifError::EWriteFailed)
        } else {
            Ok(())
        }
    }
}

/// Write a little-endian 16-bit word. Matches EGifPutWord in C.
fn egif_put_word(gif: &mut GifFileType, word: c_int) -> Result<(), GifError> {
    let c = [(word & 0xff) as u8, ((word >> 8) & 0xff) as u8];
    if gif.write(&c) == 2 {
        Ok(())
    } else {
        Err(GifError::EWriteFailed)
    }
}

// ---------------------------------------------------------------------------
//  Open helpers
// ---------------------------------------------------------------------------

/// Open a GIF file by filename for writing.
///
/// This is the inner logic for `EGifOpenFileName`.
pub fn egif_open_file_name(
    file_name: &core::ffi::CStr,
    test_existence: bool,
) -> Result<Box<GifFileType>, GifError> {
    let path = file_name.to_str().map_err(|_| GifError::EOpenFailed)?;

    // Open file — match C behavior of O_WRONLY|O_CREAT|(O_EXCL or O_TRUNC)
    let file = if test_existence {
        std::fs::OpenOptions::new().write(true).create_new(true).open(path)
    } else {
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)
    };

    let file = file.map_err(|_| GifError::EOpenFailed)?;

    egif_open(Some(file), None, ptr::null_mut())
}

/// Create and initialize a `GifFileType` for writing from the given I/O source.
///
/// Matches `EGifOpenFileHandle` and `EGifOpen` setup logic in C.
pub(crate) fn egif_open(
    file: Option<std::fs::File>,
    write_fn: Option<WriteCallback>,
    user_data: *mut core::ffi::c_void,
) -> Result<Box<GifFileType>, GifError> {
    let io = if let Some(write_fn) = write_fn {
        IoState::CallbackWrite { write_fn }
    } else if let Some(file) = file {
        IoState::FileWrite { file }
    } else {
        return Err(GifError::ENotEnoughMem);
    };
    let private = Box::new(crate::private::GifFilePrivateType::new(io));

    let gif = Box::new(GifFileType::new(private, user_data));

    Ok(gif)
}

// ---------------------------------------------------------------------------
//  Version
// ---------------------------------------------------------------------------

/// Compute the GIF version that will be written on output.
/// Matches EGifGetGifVersion in C.
pub fn egif_get_gif_version(gif: &mut GifFileType) -> &'static [u8] {
    // Bulletproofing - always write GIF89 if we need to.
    let mut needs_gif89 = gif.Private.gif89;

    if !needs_gif89 {
        'outer: for sp in gif.saved_images() {
            for ep in sp.extension_blocks() {
                let f = ep.Function;
                if f == COMMENT_EXT_FUNC_CODE
                    || f == GRAPHICS_EXT_FUNC_CODE
                    || f == PLAINTEXT_EXT_FUNC_CODE
                    || f == APPLICATION_EXT_FUNC_CODE
                {
                    needs_gif89 = true;
                    break 'outer;
                }
            }
        }
    }
    if !needs_gif89 {
        for ep in gif.extension_blocks() {
            let f = ep.Function;
            if f == COMMENT_EXT_FUNC_CODE
                || f == GRAPHICS_EXT_FUNC_CODE
                || f == PLAINTEXT_EXT_FUNC_CODE
                || f == APPLICATION_EXT_FUNC_CODE
            {
                needs_gif89 = true;
                break;
            }
        }
    }

    gif.Private.gif89 = needs_gif89;

    if gif.Private.gif89 {
        GIF89_STAMP
    } else {
        GIF87_STAMP
    }
}

/// Set the GIF version flag.
/// Matches EGifSetGifVersion in C.
pub fn egif_set_gif_version(gif: &mut GifFileType, gif89: bool) {
    gif.Private.gif89 = gif89;
}

// ---------------------------------------------------------------------------
//  Screen descriptor
// ---------------------------------------------------------------------------

/// Matches EGifPutScreenDesc in C.
pub fn egif_put_screen_desc(
    gif: &mut GifFileType,
    width: c_int,
    height: c_int,
    color_res: c_int,
    background: c_int,
    color_map: Option<&ColorMapObject>,
) -> Result<(), GifError> {
    if gif.Private.screen_desc_written {
        // If already has screen descriptor - something is wrong!
        return Err(GifError::EHasScreenDesc);
    }
    if !gif.Private.io.is_writable() {
        // This file was NOT open for writing.
        return Err(GifError::ENotWriteable);
    }

    let write_version = egif_get_gif_version(gif);

    // First write the version prefix into the file.
    // The generated constants include a trailing NUL byte (b"GIF87a\0"),
    // but we must only emit the 6 printable characters — matching the C
    // encoder, which uses strlen() to exclude the NUL.
    let stamp = write_version.strip_suffix(b"\0").unwrap_or(write_version);
    if gif.write(stamp) != stamp.len() {
        return Err(GifError::EWriteFailed);
    }

    gif.SWidth = width as GifWord;
    gif.SHeight = height as GifWord;
    gif.SColorResolution = color_res as GifWord;
    gif.SBackGroundColor = background as GifWord;

    if let Some(cm) = color_map {
        gif.SColorMap = Some(Box::new(cm.clone()));
    } else {
        gif.SColorMap = None;
    }

    // Put the logical screen descriptor into the file:
    // Logical Screen Descriptor: Dimensions
    egif_put_word(gif, width)?;
    egif_put_word(gif, height)?;

    // Logical Screen Descriptor: Packed Fields
    let mut packed: u8 = if color_map.is_some() { 0x80 } else { 0x00 }; // Yes/no global colormap
    packed |= (((color_res - 1) & 0x07) << 4) as u8; // Bits allocated to each primary color
    packed |= if let Some(cm) = color_map {
        (cm.BitsPerPixel - 1) as u8 // Actual size of the color table.
    } else {
        0x07 // Default to largest possible
    };
    if let Some(cm) = color_map {
        if cm.SortFlag {
            packed |= 0x08;
        }
    }

    let buf = [
        packed,
        background as u8, // Index into the ColorTable for background color
        gif.AspectByte,   // Pixel Aspect Ratio
    ];
    gif.write_exact(&buf)?;

    // If we have Global color map - dump it also:
    if let Some(cm) = color_map {
        for i in 0..cm.ColorCount as usize {
            let c = &cm.colors()[i];
            let rgb = [c.Red, c.Green, c.Blue];
            gif.write_exact(&rgb)?;
        }
    }

    // Mark this file as has screen descriptor, and no pixel written yet:
    gif.Private.screen_desc_written = true;

    Ok(())
}

// ---------------------------------------------------------------------------
//  Image descriptor
// ---------------------------------------------------------------------------

/// Matches EGifPutImageDesc in C.
pub fn egif_put_image_desc(
    gif: &mut GifFileType,
    left: c_int,
    top: c_int,
    width: c_int,
    height: c_int,
    interlace: bool,
    color_map: Option<&ColorMapObject>,
) -> Result<(), GifError> {
    // NOTE: The 0xffff0000 threshold is a C-ism: in the 32-bit C code this
    // caught unsigned int wrap-around.  On 64-bit Rust (usize) it's effectively
    // a no-op, but we keep it for bug-for-bug parity with the C implementation.
    if gif.Private.in_image && gif.Private.pixel_count > 0xffff0000 {
        // If already has active image descriptor - something is wrong!
        return Err(GifError::EHasImageDesc);
    }
    if !gif.Private.io.is_writable() {
        // This file was NOT open for writing.
        return Err(GifError::ENotWriteable);
    }

    gif.Image.Left = left as GifWord;
    gif.Image.Top = top as GifWord;
    gif.Image.Width = width as GifWord;
    gif.Image.Height = height as GifWord;
    gif.Image.Interlace = interlace;

    // Handle color map
    if let Some(cm) = color_map {
        gif.Image.ColorMap = Some(Box::new(cm.clone()));
    } else {
        gif.Image.ColorMap = None;
    }

    // Put the image descriptor into the file:
    let intro = [DESCRIPTOR_INTRODUCER]; // Image separator character.
    gif.write_exact(&intro)?;
    egif_put_word(gif, left)?;
    egif_put_word(gif, top)?;
    egif_put_word(gif, width)?;
    egif_put_word(gif, height)?;

    let mut packed: u8 = 0;
    if color_map.is_some() {
        packed |= 0x80;
    }
    if interlace {
        packed |= 0x40;
    }
    if let Some(cm) = color_map {
        packed |= (cm.BitsPerPixel - 1) as u8;
    }
    gif.write_exact(&[packed])?;

    // If we have local color map - dump it also:
    if let Some(cm) = color_map {
        for i in 0..cm.ColorCount as usize {
            let c = &cm.colors()[i];
            let rgb = [c.Red, c.Green, c.Blue];
            gif.write_exact(&rgb)?;
        }
    }

    if gif.SColorMap.is_none() && gif.Image.ColorMap.is_none() {
        return Err(GifError::ENoColorMap);
    }

    // Mark this file as has screen descriptor:
    gif.Private.in_image = true;
    gif.Private.pixel_count =
        (width as usize).checked_mul(height as usize).ok_or(GifError::EDataTooBig)?;

    // Reset compress algorithm parameters.
    encoder_lzw::egif_setup_compress(gif)
}

// ---------------------------------------------------------------------------
//  Line / Pixel writing
// ---------------------------------------------------------------------------

/// Matches EGifPutLine in C.
pub fn egif_put_line(gif: &mut GifFileType, line: &mut [GifPixelType]) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    // When line is empty, use Image.Width as the length (matching C behavior).
    // In C, the caller must provide a buffer of at least Width bytes;
    // here we create a local zero-filled buffer if the input slice is empty.
    if line.is_empty() {
        let line_len = gif.Image.Width.max(0) as usize;
        if gif.Private.pixel_count < line_len {
            return Err(GifError::EDataTooBig);
        }
        gif.Private.pixel_count -= line_len;
        let mask = encoder_lzw::get_code_mask(gif.Private.lzw.bits_per_pixel);
        let mut buf = vec![0u8; line_len];
        for pixel in buf.iter_mut() {
            *pixel &= mask;
        }
        return encoder_lzw::egif_compress_line(gif, &buf);
    }

    let line_len = line.len();

    if gif.Private.pixel_count < line_len {
        return Err(GifError::EDataTooBig);
    }
    gif.Private.pixel_count -= line_len;

    // Make sure the codes are not out of bit range, as we might generate
    // wrong code (because of overflow when we combine them) in this case:
    let mask = encoder_lzw::get_code_mask(gif.Private.lzw.bits_per_pixel);
    for i in 0..line_len {
        line[i] &= mask;
    }

    encoder_lzw::egif_compress_line(gif, &line[..line_len])
}

/// Matches EGifPutPixel in C.
pub fn egif_put_pixel(gif: &mut GifFileType, pixel: GifPixelType) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    if gif.Private.pixel_count == 0 {
        return Err(GifError::EDataTooBig);
    }
    gif.Private.pixel_count -= 1;

    // Make sure the code is not out of bit range.
    let masked = pixel & encoder_lzw::get_code_mask(gif.Private.lzw.bits_per_pixel);

    encoder_lzw::egif_compress_line(gif, &[masked])
}

// ---------------------------------------------------------------------------
//  Comment
// ---------------------------------------------------------------------------

/// Matches EGifPutComment in C.
pub fn egif_put_comment(gif: &mut GifFileType, comment: &[u8]) -> Result<(), GifError> {
    let length = comment.len();
    if length <= 255 {
        egif_put_extension(gif, COMMENT_EXT_FUNC_CODE, comment)
    } else {
        egif_put_extension_leader(gif, COMMENT_EXT_FUNC_CODE)?;

        // Break the comment into 255 byte sub blocks
        let mut offset = 0;
        while length - offset > 255 {
            egif_put_extension_block(gif, &comment[offset..offset + 255])?;
            offset += 255;
        }
        // Output any partial block
        if length - offset > 0 {
            egif_put_extension_block(gif, &comment[offset..length])?;
        }
        egif_put_extension_trailer(gif)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
//  Extension blocks
// ---------------------------------------------------------------------------

/// Matches EGifPutExtensionLeader in C.
pub fn egif_put_extension_leader(gif: &mut GifFileType, ext_code: c_int) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    let buf = [EXTENSION_INTRODUCER, ext_code as u8];
    gif.write_exact(&buf)?;

    Ok(())
}

/// Matches EGifPutExtensionBlock in C.
pub fn egif_put_extension_block(gif: &mut GifFileType, extension: &[u8]) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    let len_byte = [extension.len() as u8];
    gif.write_exact(&len_byte)?;
    gif.write_exact(extension)?;

    Ok(())
}

/// Matches EGifPutExtensionTrailer in C.
pub fn egif_put_extension_trailer(gif: &mut GifFileType) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    // Write the block terminator
    gif.write_exact(&[0u8])?;

    Ok(())
}

/// Matches EGifPutExtension in C.
/// Warning: only useful for Extension blocks that have at most one subblock.
pub fn egif_put_extension(
    gif: &mut GifFileType,
    ext_code: c_int,
    extension: &[u8],
) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    let ext_len = extension.len() as u8;

    if ext_code == 0 {
        gif.write_exact(&[ext_len])?;
    } else {
        let buf = [EXTENSION_INTRODUCER, ext_code as u8, ext_len];
        gif.write_exact(&buf)?;
    }
    gif.write_exact(extension)?;
    gif.write_exact(&[0u8])?; // Terminator

    Ok(())
}

// ---------------------------------------------------------------------------
//  Raw code passthrough
// ---------------------------------------------------------------------------

/// Matches EGifPutCode in C.
pub fn egif_put_code(gif: &mut GifFileType, code_block: &[u8]) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    // No need to dump code size as Compression set up does any for us.
    egif_put_code_next_bytes(gif, Some(code_block))
}

/// Matches EGifPutCodeNext in C.
pub fn egif_put_code_next_bytes(
    gif: &mut GifFileType,
    code_block: Option<&[u8]>,
) -> Result<(), GifError> {
    if let Some(block) = code_block {
        // The block is Pascal-string format: block[0] = len, block[1..=len] = data
        let total_len = block[0] as usize + 1;
        if gif.write(&block[..total_len]) != total_len {
            return Err(GifError::EWriteFailed);
        }
    } else {
        // NULL block = write empty block to mark end
        if gif.write(&[0u8]) != 1 {
            return Err(GifError::EWriteFailed);
        }
        gif.Private.pixel_count = 0; // And local info. indicate image read.
    }

    Ok(())
}

// ---------------------------------------------------------------------------
//  GCB helpers
// ---------------------------------------------------------------------------

/// Render a Graphics Control Block as raw extension data.
/// Matches EGifGCBToExtension in C.
pub fn egif_gcb_to_extension(gcb: &GraphicsControlBlock, ext: &mut [GifByteType; 4]) {
    ext[0] = 0;
    ext[0] |= if gcb.TransparentColor == NO_TRANSPARENT_COLOR { 0x00 } else { 0x01 };
    ext[0] |= if gcb.UserInputFlag { 0x02 } else { 0x00 };
    ext[0] |= ((gcb.DisposalMode & 0x07) << 2) as u8;
    ext[1] = (gcb.DelayTime & 0xff) as u8; // LOBYTE
    ext[2] = ((gcb.DelayTime >> 8) & 0xff) as u8; // HIBYTE
    ext[3] = gcb.TransparentColor as u8;
}

/// Replace the Graphics Control Block for a saved image, if it exists.
/// Matches EGifGCBToSavedExtension in C.
pub fn egif_gcb_to_saved_extension(
    gcb: &GraphicsControlBlock,
    gif: &mut GifFileType,
    image_index: c_int,
) -> Result<(), GifError> {
    if image_index < 0 || image_index > gif.ImageCount - 1 {
        return Err(GifError::ENotWriteable); // C returns GIF_ERROR without setting specific error
    }

    let saved = &mut gif.saved_images_mut()[image_index as usize];
    for ep in saved.extension_blocks_mut().iter_mut() {
        if ep.Function == GRAPHICS_EXT_FUNC_CODE {
            egif_gcb_to_extension(
                gcb,
                (&mut *ep.bytes_mut()).try_into().expect("GCB extension block must be 4 bytes"),
            );
            return Ok(());
        }
    }

    // No existing GCB block found — add a new one.
    let mut ext = [0u8; 4];
    egif_gcb_to_extension(gcb, &mut ext);
    saved.extension_blocks_mut().add(ExtensionBlock::new(GRAPHICS_EXT_FUNC_CODE, &ext));

    Ok(())
}

// ---------------------------------------------------------------------------
//  Close
// ---------------------------------------------------------------------------

/// Matches EGifCloseFile in C.
/// Writes the GIF terminator and flushes/closes the file.
pub fn egif_close_file(gif: &mut GifFileType) -> Result<(), GifError> {
    if !gif.Private.io.is_writable() {
        return Err(GifError::ENotWriteable);
    }

    // Write the terminator
    gif.write_exact(&[TERMINATOR_INTRODUCER])?;

    Ok(())
}

// ---------------------------------------------------------------------------
//  EGifWriteExtensions (internal helper)
// ---------------------------------------------------------------------------

/// Write extension blocks to the output.
/// Matches EGifWriteExtensions in C.
fn egif_write_extensions(
    gif: &mut GifFileType,
    ext_blocks: &[ExtensionBlock],
) -> Result<(), GifError> {
    for (j, ep) in ext_blocks.iter().enumerate() {
        if ep.Function != CONTINUE_EXT_FUNC_CODE {
            egif_put_extension_leader(gif, ep.Function)?;
        }
        egif_put_extension_block(gif, ep.bytes())?;
        if j == ext_blocks.len() - 1 || ext_blocks[j + 1].Function != CONTINUE_EXT_FUNC_CODE {
            egif_put_extension_trailer(gif)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
//  Spew (high-level write)
// ---------------------------------------------------------------------------

/// Matches EGifSpew in C.
/// Writes an in-core representation of a GIF to the output.
pub fn egif_spew(gif: &mut GifFileType) -> Result<(), GifError> {
    // Take SColorMap out to avoid borrowing gif while passing &mut gif
    // to egif_put_screen_desc. Restore the original after the call,
    // replacing the redundant clone that egif_put_screen_desc makes internally.
    let screen_map = gif.SColorMap.take();
    let result = egif_put_screen_desc(
        gif,
        gif.SWidth as c_int,
        gif.SHeight as c_int,
        gif.SColorResolution as c_int,
        gif.SBackGroundColor as c_int,
        screen_map.as_deref(),
    );
    gif.SColorMap = screen_map;
    result?;

    let mut line_buf: Vec<u8> = Vec::new();

    for i in 0..gif.ImageCount as usize {
        let saved = &gif.saved_images()[i];

        // this allows us to delete images by nuking their rasters
        if saved.RasterBits.is_null() {
            continue;
        }

        let saved_height = saved.ImageDesc.Height as c_int;
        let saved_width = saved.ImageDesc.Width as c_int;
        let interlace = saved.ImageDesc.Interlace;
        let left = saved.ImageDesc.Left as c_int;
        let top = saved.ImageDesc.Top as c_int;

        // Clone extension blocks and color map data to avoid borrow conflicts
        // (saved_images() borrows &gif, but write functions need &mut gif).
        let ext_blocks: Vec<ExtensionBlock> =
            saved.extension_blocks().iter().map(|e| e.clone()).collect();
        let local_map = saved.ImageDesc.ColorMap.as_deref().cloned();

        // Write extensions for this image
        egif_write_extensions(gif, &ext_blocks)?;

        // Write image descriptor
        egif_put_image_desc(
            gif,
            left,
            top,
            saved_width,
            saved_height,
            interlace,
            local_map.as_ref(),
        )?;

        // Pre-size the reusable line buffer for this image.
        let w = saved_width as usize;
        line_buf.resize(w, 0);

        // Write raster data
        if interlace {
            static INTERLACED_OFFSET: [c_int; 4] = [0, 4, 2, 1];
            static INTERLACED_JUMPS: [c_int; 4] = [8, 8, 4, 2];

            for k in 0..4 {
                let mut j = INTERLACED_OFFSET[k];
                while j < saved_height {
                    let raster_bits = gif.saved_images()[i].raster_bits();
                    let offset = (j as usize) * w;
                    line_buf.copy_from_slice(&raster_bits[offset..offset + w]);
                    egif_put_line(gif, &mut line_buf)?;
                    j += INTERLACED_JUMPS[k];
                }
            }
        } else {
            for j in 0..saved_height {
                let raster_bits = gif.saved_images()[i].raster_bits();
                let offset = (j as usize) * w;
                line_buf.copy_from_slice(&raster_bits[offset..offset + w]);
                egif_put_line(gif, &mut line_buf)?;
            }
        }
    }

    // Write trailing extension blocks (past last image)
    let trailing_ext: Vec<ExtensionBlock> =
        gif.extension_blocks().iter().map(|e| e.clone()).collect();
    egif_write_extensions(gif, &trailing_ext)?;

    Ok(())
}
