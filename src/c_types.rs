#![allow(unused, nonstandard_style)]
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

pub use crate::c_types_gen::{
    GifByteType, GifColorType, GifPixelType, GifPrefixType, GifRecordType, GifWord,
    GraphicsControlBlock, InputFunc, OutputFunc, APPLICATION_EXT_FUNC_CODE, COMMENT_EXT_FUNC_CODE,
    CONTINUE_EXT_FUNC_CODE, DISPOSAL_UNSPECIFIED, D_GIF_ERR_CLOSE_FAILED, D_GIF_ERR_DATA_TOO_BIG,
    D_GIF_ERR_EOF_TOO_SOON, D_GIF_ERR_IMAGE_DEFECT, D_GIF_ERR_NOT_ENOUGH_MEM,
    D_GIF_ERR_NOT_GIF_FILE, D_GIF_ERR_NOT_READABLE, D_GIF_ERR_NO_COLOR_MAP, D_GIF_ERR_NO_IMAG_DSCR,
    D_GIF_ERR_NO_SCRN_DSCR, D_GIF_ERR_OPEN_FAILED, D_GIF_ERR_READ_FAILED, D_GIF_ERR_WRONG_RECORD,
    E_GIF_ERR_CLOSE_FAILED, E_GIF_ERR_DATA_TOO_BIG, E_GIF_ERR_DISK_IS_FULL,
    E_GIF_ERR_HAS_IMAG_DSCR, E_GIF_ERR_HAS_SCRN_DSCR, E_GIF_ERR_NOT_ENOUGH_MEM,
    E_GIF_ERR_NOT_WRITEABLE, E_GIF_ERR_NO_COLOR_MAP, E_GIF_ERR_OPEN_FAILED, E_GIF_ERR_WRITE_FAILED,
    E_GIF_SUCCEEDED, GIF87_STAMP, GIF89_STAMP, GIF_ERROR, GIF_OK, GIF_STAMP, GIF_VERSION_POS,
    GRAPHICS_EXT_FUNC_CODE, NO_TRANSPARENT_COLOR, PLAINTEXT_EXT_FUNC_CODE,
};
use safer_cffi::{CSlicePtr, CSliceRefMut};
use std::os::raw::c_int;
use std::os::raw::c_uchar;
use std::os::raw::c_uint;
use std::os::raw::c_void;

// Box<T> / Option<Box<T>> for single-element pointers (auto-drop)
// *mut T for array pointers (manual Drop)
//
// # Raw Slice Memory Invariant
//
// All raw-pointer-backed arrays in this crate maintain `len == capacity`
// at all times.
//
// ---------------------------------------------------------------------------
//  ColorMapObject
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug)]
pub struct ColorMapObject {
    pub ColorCount: c_int,
    pub BitsPerPixel: c_int,
    pub SortFlag: bool,
    // Safety invariant: the length of this array is `ColorCount`.
    pub Colors: CSlicePtr<GifColorType>,
}

// Array accessors
impl ColorMapObject {
    pub fn colors(&self) -> &[GifColorType] {
        // SAFETY: the length of `Colors` is `ColorCount`.
        unsafe { self.Colors.with_len(self.ColorCount) }
    }

    pub fn colors_mut(&mut self) -> CSliceRefMut<'_, GifColorType, c_int> {
        // SAFETY: the length of `Colors` is `ColorCount`.
        unsafe { self.Colors.with_len_mut(&mut self.ColorCount) }
    }
}

impl Drop for ColorMapObject {
    fn drop(&mut self) {
        self.colors_mut().clear();
    }
}

impl Clone for ColorMapObject {
    fn clone(&self) -> Self {
        Self {
            ColorCount: self.ColorCount,
            BitsPerPixel: self.BitsPerPixel,
            SortFlag: self.SortFlag,
            Colors: CSlicePtr::clone_and_leak(self.colors()),
        }
    }
}

/// Error returned when a color count is not a valid power of 2.
#[derive(Debug)]
pub struct InvalidColorCount;

impl ColorMapObject {
    /// Create a new zeroed color map.
    ///
    /// `color_count` must be > 0 and a power of 2.
    pub fn new(color_count: c_int) -> Result<Self, InvalidColorCount> {
        if !(0..=256).contains(&color_count) {
            return Err(InvalidColorCount);
        }
        let stack_colors: [GifColorType; 256] =
            std::array::from_fn(|_| GifColorType { Red: 0, Green: 0, Blue: 0 });
        Self::from_slice(&stack_colors[..color_count as usize])
    }

    /// Create a new color map from an existing slice of colors.
    ///
    /// `src.len()` must be > 0 and a power of 2.
    pub fn from_slice(src: &[GifColorType]) -> Result<Self, InvalidColorCount> {
        let color_count = src.len() as c_int;
        let bits = Self::bit_size(color_count);
        if color_count <= 0 || color_count != (1 << bits) {
            return Err(InvalidColorCount);
        }
        Ok(Self {
            ColorCount: color_count,
            BitsPerPixel: bits,
            SortFlag: false,
            Colors: CSlicePtr::clone_and_leak(src),
        })
    }
}

// ---------------------------------------------------------------------------
//  GifImageDesc
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone)]
pub struct GifImageDesc {
    pub Left: GifWord,
    pub Top: GifWord,
    pub Width: GifWord,
    pub Height: GifWord,
    pub Interlace: bool,
    pub ColorMap: Option<Box<ColorMapObject>>,
}

// ---------------------------------------------------------------------------
//  ExtensionBlock
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug)]
pub struct ExtensionBlock {
    pub ByteCount: c_int,
    // Safety invariant: the length of this array is `ByteCount`.
    pub Bytes: CSlicePtr<GifByteType>,
    pub Function: c_int,
}

// Array accessors
impl ExtensionBlock {
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the length of `Bytes` is `ByteCount`.
        unsafe { self.Bytes.with_len(self.ByteCount) }
    }

    pub fn bytes_mut(&mut self) -> CSliceRefMut<'_, u8, c_int> {
        // SAFETY: the length of `Bytes` is `ByteCount`.
        unsafe { self.Bytes.with_len_mut(&mut self.ByteCount) }
    }
}

impl Drop for ExtensionBlock {
    fn drop(&mut self) {
        self.bytes_mut().clear();
    }
}

impl Clone for ExtensionBlock {
    fn clone(&self) -> Self {
        Self {
            ByteCount: self.ByteCount,
            Bytes: CSlicePtr::clone_and_leak(self.bytes()),
            Function: self.Function,
        }
    }
}

impl ExtensionBlock {
    pub fn new(function: c_int, data: &[u8]) -> Self {
        let bytes =
            if data.is_empty() { CSlicePtr::null() } else { CSlicePtr::clone_and_leak(data) };
        Self { Function: function, ByteCount: data.len() as c_int, Bytes: bytes }
    }
}

// ---------------------------------------------------------------------------
//  SavedImage
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug)]
pub struct SavedImage {
    pub ImageDesc: GifImageDesc,
    // Safety invariant: When the image dimensions are valid (1..=65535), the length of this array
    // is `SavedImage::size`, which is equal to `ImageDesc.Width * ImageDesc.Height`.
    pub RasterBits: CSlicePtr<GifByteType>,
    pub ExtensionBlockCount: c_int,
    // Safety invariant: the length of this array is `ExtensionBlockCount`.
    pub ExtensionBlocks: CSlicePtr<ExtensionBlock>,
}

// Array accessors
impl SavedImage {
    /// Returns the validated size of the raster bits in bytes, or `None` if the image
    /// dimensions are out of range (0 or >65535).
    pub fn size(&self) -> Option<c_int> {
        match (self.ImageDesc.Width, self.ImageDesc.Height) {
            (1..=65535, 1..=65535) => self.ImageDesc.Width.checked_mul(self.ImageDesc.Height),
            _ => None,
        }
    }

    pub fn raster_bits(&self) -> &[u8] {
        let size = self.size().unwrap_or(0);
        // SAFETY: the length of `RasterBits` is `SavedImage::size`.
        unsafe { self.RasterBits.with_len(size) }
    }

    pub fn raster_bits_mut(&mut self) -> &mut [u8] {
        if self.RasterBits.is_null() {
            return &mut [];
        }
        let Some(size) = self.size() else {
            return &mut [];
        };
        // SAFETY:
        // - RasterBits is not null and has length `size` (dimensions validated by
        //   `SavedImage::size`).
        // - `SavedImage` owns the underlying array, so the pointer is valid for
        //   reads and writes as long as we borrow it via `&mut self`.
        unsafe { core::slice::from_raw_parts_mut(self.RasterBits.as_ptr(), size as usize) }
    }

    pub fn raster_bits_row_mut(&mut self, row: usize) -> Option<&mut [u8]> {
        let start = row.checked_mul(self.ImageDesc.Width as usize)?;
        let end = start.checked_add(self.ImageDesc.Width as usize)?;
        self.raster_bits_mut().get_mut(start..end)
    }

    pub fn extension_blocks(&self) -> &[ExtensionBlock] {
        // SAFETY: the length of `ExtensionBlocks` is `ExtensionBlockCount`.
        unsafe { self.ExtensionBlocks.with_len(self.ExtensionBlockCount) }
    }

    pub fn extension_blocks_mut(&mut self) -> CSliceRefMut<'_, ExtensionBlock, c_int> {
        // SAFETY: the length of `ExtensionBlocks` is `ExtensionBlockCount`.
        unsafe { self.ExtensionBlocks.with_len_mut(&mut self.ExtensionBlockCount) }
    }
}

impl Drop for SavedImage {
    fn drop(&mut self) {
        let mut count = self.raster_bits().len() as c_int;
        // SAFETY: By implementation of `.raster_bits()`, `count` is a safe length for `RasterBits`.
        let mut slice = unsafe { self.RasterBits.with_len_mut(&mut count) };
        slice.clear();
        self.extension_blocks_mut().clear();
    }
}

impl Clone for SavedImage {
    fn clone(&self) -> Self {
        Self {
            ImageDesc: self.ImageDesc.clone(),
            RasterBits: CSlicePtr::clone_and_leak(self.raster_bits()),
            ExtensionBlockCount: self.ExtensionBlockCount,
            ExtensionBlocks: CSlicePtr::clone_and_leak(self.extension_blocks()),
        }
    }
}

impl SavedImage {
    pub fn new(desc: GifImageDesc) -> Self {
        Self {
            ImageDesc: desc,
            RasterBits: CSlicePtr::null(),
            ExtensionBlockCount: 0,
            ExtensionBlocks: CSlicePtr::null(),
        }
    }
}

// ---------------------------------------------------------------------------
//  GifFileType
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct GifFileType {
    pub SWidth: GifWord,
    pub SHeight: GifWord,
    pub SColorResolution: GifWord,
    pub SBackGroundColor: GifWord,
    pub AspectByte: GifByteType,
    pub SColorMap: Option<Box<ColorMapObject>>,
    pub ImageCount: c_int,
    pub Image: GifImageDesc,
    // Safety invariant: the length of this array is `ImageCount`.
    pub SavedImages: CSlicePtr<SavedImage>,
    pub ExtensionBlockCount: c_int,
    // Safety invariant: the length of this array is `ExtensionBlockCount`.
    pub ExtensionBlocks: CSlicePtr<ExtensionBlock>,
    pub Error: c_int,
    pub UserData: UserData,
    pub Private: Box<crate::private::GifFilePrivateType>,
}

// Array accessors
impl GifFileType {
    pub fn saved_images(&self) -> &[SavedImage] {
        // SAFETY: the length of `SavedImages` is `ImageCount`.
        unsafe { self.SavedImages.with_len(self.ImageCount) }
    }

    pub fn saved_images_mut(&mut self) -> CSliceRefMut<'_, SavedImage, c_int> {
        // SAFETY: the length of `SavedImages` is `ImageCount`.
        unsafe { self.SavedImages.with_len_mut(&mut self.ImageCount) }
    }

    pub fn extension_blocks(&self) -> &[ExtensionBlock] {
        // SAFETY: the length of `ExtensionBlocks` is `ExtensionBlockCount`.
        unsafe { self.ExtensionBlocks.with_len(self.ExtensionBlockCount) }
    }

    pub fn extension_blocks_mut(&mut self) -> CSliceRefMut<'_, ExtensionBlock, c_int> {
        // SAFETY: the length of `ExtensionBlocks` is `ExtensionBlockCount`.
        unsafe { self.ExtensionBlocks.with_len_mut(&mut self.ExtensionBlockCount) }
    }
}

impl Drop for GifFileType {
    fn drop(&mut self) {
        self.saved_images_mut().clear();
        self.extension_blocks_mut().clear();
    }
}

impl GifFileType {
    /// Create a new, zeroed `GifFileType` with the given private state and user data.
    ///
    /// Shared by both the decoder (`dgif_open`) and encoder (`egif_open`)
    /// open paths, ensuring they stay in sync.
    pub(crate) fn new(
        private: Box<crate::private::GifFilePrivateType>,
        user_data: *mut c_void,
    ) -> Self {
        Self {
            SWidth: 0,
            SHeight: 0,
            SColorResolution: 0,
            SBackGroundColor: 0,
            AspectByte: 0,
            SColorMap: None,
            ImageCount: 0,
            Image: GifImageDesc {
                Left: 0,
                Top: 0,
                Width: 0,
                Height: 0,
                Interlace: false,
                ColorMap: None,
            },
            SavedImages: CSlicePtr::null(),
            ExtensionBlockCount: 0,
            ExtensionBlocks: CSlicePtr::null(),
            Error: 0,
            UserData: UserData(user_data),
            Private: private,
        }
    }

    /// Convert a `Result<(), GifError>` into the C-style `GIF_OK` / `GIF_ERROR`
    /// return code, setting `self.Error` on the error path.
    pub fn result_to_status(&mut self, result: Result<(), crate::err::GifError>) -> c_int {
        match result {
            Ok(()) => GIF_OK,
            Err(e) => {
                self.Error = e as c_int;
                GIF_ERROR
            }
        }
    }
}

// ---------------------------------------------------------------------------
//  UserData
// ---------------------------------------------------------------------------

#[repr(transparent)]
pub struct UserData(*mut c_void);

// SAFETY: UserData is a caller-provided opaque pointer that's never used on the Rust side.
unsafe impl Send for UserData {}

// ---------------------------------------------------------------------------
//  Callback wrappers
// ---------------------------------------------------------------------------

/// Wraps an C read callback to centralize the safety decision.
///
/// # Safety Invariants
///
/// - The function pointer is valid for the lifetime of `self`.
/// - The function pointer is safe to call with a valid `GifFileType*` and buffer arguments,
///   i.e. if `*mut GifByteType` is a valid pointer to a writable buffer of at least the given size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadCallback(
    unsafe extern "C" fn(*mut GifFileType, *mut GifByteType, c_int) -> c_int,
);

impl ReadCallback {
    /// # Safety
    /// The caller must guarantee the function pointer upholds the struct's safety invariants.
    #[rustfmt::skip]
    pub(crate) unsafe fn new(
        func: unsafe extern "C" fn(*mut crate::c_types_gen::GifFileType, *mut GifByteType, c_int) -> c_int,
    ) -> Self {
        // SAFETY: `c_types_gen::InputFunc` uses `c_types_gen::GifFileType` for the first argument,
        // but we want to use our own layout-identical `c_types::GifFileType`.
        let func = unsafe {
            core::mem::transmute::<
                unsafe extern "C" fn(*mut crate::c_types_gen::GifFileType, *mut GifByteType, c_int) -> c_int,
                unsafe extern "C" fn(*mut crate::c_types::GifFileType, *mut GifByteType, c_int) -> c_int,
            >(func)
        };
        Self(func)
    }

    pub(crate) fn read(&self, gif: *mut GifFileType, buf: &mut [u8]) -> c_int {
        // SAFETY: `ReadCallback`'s safety invariants guarantee that `self.0` is safe to call
        // with the given arguments. `buf` is a valid slice, so `buf.as_mut_ptr()` points to
        // a writable buffer of `buf.len()` bytes.
        unsafe { (self.0)(gif, buf.as_mut_ptr(), buf.len() as c_int) }
    }
}

/// Wraps an C write callback to centralize the safety decision.
///
/// # Safety Invariants
///
/// - The function pointer is valid for the lifetime of the GIF handle.
/// - The function pointer is safe to call with a valid `GifFileType*` and buffer arguments,
///   i.e. `*const GifByteType` is a valid pointer to a readable buffer of at least the given size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WriteCallback(
    unsafe extern "C" fn(*mut GifFileType, *const GifByteType, c_int) -> c_int,
);

impl WriteCallback {
    /// # Safety
    /// The caller must guarantee the function pointer upholds the struct's safety invariants.
    #[rustfmt::skip]
    pub(crate) unsafe fn new(
        func: unsafe extern "C" fn(*mut crate::c_types_gen::GifFileType, *const GifByteType, c_int) -> c_int,
    ) -> Self {
        // SAFETY: `c_types_gen::OutputFunc` uses `c_types_gen::GifFileType` for the first argument,
        // but we want to use our own layout-identical `c_types::GifFileType`.
        let func = unsafe {
            core::mem::transmute::<
                unsafe extern "C" fn(*mut crate::c_types_gen::GifFileType, *const GifByteType, c_int) -> c_int,
                unsafe extern "C" fn(*mut crate::c_types::GifFileType, *const GifByteType, c_int) -> c_int,
            >(func)
        };
        Self(func)
    }

    pub(crate) fn write(&self, gif: *mut GifFileType, buf: &[u8]) -> c_int {
        // SAFETY: `WriteCallback`'s safety invariants guarantee that `self.0` is safe to call
        // with the given arguments. `buf` is a valid slice, so `buf.as_ptr()` points to
        // a readable buffer of `buf.len()` bytes.
        unsafe { (self.0)(gif, buf.as_ptr(), buf.len() as c_int) }
    }
}
