#![allow(nonstandard_style)]
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

#[allow(unused, nonstandard_style)]
mod c_types_gen {
    include!(concat!(env!("OUT_DIR"), "/c_types_gen.rs"));
}

pub use c_types_gen::{
    GifByteType, GifColorType, GifPixelType, GifPrefixType, GifRecordType, GifWord,
    GraphicsControlBlock, InputFunc, OutputFunc, APPLICATION_EXT_FUNC_CODE, COMMENT_EXT_FUNC_CODE,
    CONTINUE_EXT_FUNC_CODE, DISPOSAL_UNSPECIFIED, D_GIF_ERR_CLOSE_FAILED, D_GIF_ERR_DATA_TOO_BIG,
    D_GIF_ERR_EOF_TOO_SOON, D_GIF_ERR_IMAGE_DEFECT, D_GIF_ERR_NOT_ENOUGH_MEM,
    D_GIF_ERR_NOT_GIF_FILE, D_GIF_ERR_NOT_READABLE, D_GIF_ERR_NO_COLOR_MAP, D_GIF_ERR_NO_IMAG_DSCR,
    D_GIF_ERR_NO_SCRN_DSCR, D_GIF_ERR_OPEN_FAILED, D_GIF_ERR_READ_FAILED, D_GIF_ERR_WRONG_RECORD,
    D_GIF_SUCCEEDED, E_GIF_ERR_CLOSE_FAILED, E_GIF_ERR_DATA_TOO_BIG, E_GIF_ERR_DISK_IS_FULL,
    E_GIF_ERR_HAS_IMAG_DSCR, E_GIF_ERR_HAS_SCRN_DSCR, E_GIF_ERR_NOT_ENOUGH_MEM,
    E_GIF_ERR_NOT_WRITEABLE, E_GIF_ERR_NO_COLOR_MAP, E_GIF_ERR_OPEN_FAILED, E_GIF_ERR_WRITE_FAILED,
    E_GIF_SUCCEEDED, GIF87_STAMP, GIF89_STAMP, GIF_ERROR, GIF_OK, GIF_STAMP, GIF_VERSION_POS,
    GRAPHICS_EXT_FUNC_CODE, NO_TRANSPARENT_COLOR, PLAINTEXT_EXT_FUNC_CODE,
};
use safer_cffi::{CBufPtr, CVecRefMut, OwnedCBufPtr};
use std::os::raw::c_int;
use std::os::raw::c_void;

// Box<T> / Option<Box<T>> for single-element pointers (auto-drop)
// CBufPtr<T> / OwnedCBufPtr<T> for array pointers
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
    // Safety invariant: `ColorCount >= 0`, and either `Colors` is null (when `ColorCount == 0`),
    // or `ColorCount > 0` and `Colors` points to a `LibcAlloc`-allocated array of at least
    // `ColorCount` consecutive initialized `GifColorType` elements within a single live allocation
    // (with `ColorCount as usize * size_of::<GifColorType>() <= isize::MAX as usize`), uniquely
    // owned by `self` with length-only capacity (`capacity == ColorCount`).
    pub Colors: OwnedCBufPtr<GifColorType>,
}

// Array accessors
impl ColorMapObject {
    pub fn colors(&self) -> &[GifColorType] {
        // SAFETY:
        // - Operation: `self.Colors.with_len(self.ColorCount)`.
        // - Contract: `self.ColorCount >= 0`, and if `self.Colors` is non-null and
        //   `self.ColorCount > 0`, `self.Colors` must point to at least `self.ColorCount`
        //   consecutive initialized, properly aligned `GifColorType` elements within a single live
        //   allocation (`<= isize::MAX` bytes) that is not mutated for the borrow of `&self`.
        // - Evidence: Guaranteed by the field safety invariant on `ColorMapObject::Colors` and the
        //   shared borrow `&self`.
        unsafe { self.Colors.with_len(self.ColorCount) }
    }

    pub fn colors_mut(&mut self) -> &mut [GifColorType] {
        // SAFETY:
        // - Operation: `self.Colors.with_len_mut(self.ColorCount)`.
        // - Contract: `self.ColorCount >= 0`, and if `self.Colors` is non-null and
        //   `self.ColorCount > 0`, `self.Colors` must point to at least `self.ColorCount`
        //   consecutive initialized, properly aligned `GifColorType` elements within a single live
        //   allocation (`<= isize::MAX` bytes) valid for reads and writes and unaliased for the
        //   exclusive borrow of `&mut self`.
        // - Evidence: Guaranteed by the field safety invariant on `ColorMapObject::Colors` and the
        //   exclusive borrow `&mut self`.
        unsafe { self.Colors.with_len_mut(self.ColorCount) }
    }
}

impl Clone for ColorMapObject {
    fn clone(&self) -> Self {
        Self {
            ColorCount: self.ColorCount,
            BitsPerPixel: self.BitsPerPixel,
            SortFlag: self.SortFlag,
            Colors: OwnedCBufPtr::clone_from_slice(self.colors()),
        }
    }
}

/// Error returned when a color count is not a valid power of 2.
#[derive(Debug)]
pub struct InvalidColorCount;

impl ColorMapObject {
    /// Create a new zeroed color map.
    ///
    /// `color_count` must be a power of 2 from 2 to 512. Like C's `GifMakeMapObject`, this
    /// accepts 512 because `GifBitSize` returns 9 for anything above 256, which
    /// `GifUnionColorMap` relies on for its scratch map when given a 256-color input.
    pub fn new(color_count: c_int) -> Result<Self, InvalidColorCount> {
        if !(0..=512).contains(&color_count) {
            return Err(InvalidColorCount);
        }
        let stack_colors: [GifColorType; 512] =
            std::array::from_fn(|_| GifColorType { Red: 0, Green: 0, Blue: 0 });
        Self::from_slice(&stack_colors[..color_count as usize])
    }

    /// Create a new color map from an existing slice of colors.
    ///
    /// `src.len()` must be a power of 2 from 2 to 512.
    pub fn from_slice(src: &[GifColorType]) -> Result<Self, InvalidColorCount> {
        let color_count = c_int::try_from(src.len()).map_err(|_| InvalidColorCount)?;
        let bits = Self::bit_size(color_count);
        if color_count != (1 << bits) {
            return Err(InvalidColorCount);
        }
        let colors = OwnedCBufPtr::try_clone_from_slice(src).map_err(|_| InvalidColorCount)?;
        Ok(Self { ColorCount: color_count, BitsPerPixel: bits, SortFlag: false, Colors: colors })
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
    // Safety invariant: `ByteCount >= 0`, and either `Bytes` is null (when `ByteCount == 0`), or
    // `ByteCount > 0` and `Bytes` points to a `LibcAlloc`-allocated array of at least `ByteCount`
    // consecutive initialized `GifByteType` elements within a single live allocation (with
    // `ByteCount as usize <= isize::MAX as usize`), uniquely owned by `self` with length-only
    // capacity (`capacity == ByteCount`).
    pub Bytes: OwnedCBufPtr<GifByteType>,
    pub Function: c_int,
}

// Array accessors
impl ExtensionBlock {
    pub fn bytes(&self) -> &[u8] {
        // SAFETY:
        // - Operation: `self.Bytes.with_len(self.ByteCount)`.
        // - Contract: `self.ByteCount >= 0`, and if `self.Bytes` is non-null and
        //   `self.ByteCount > 0`, `self.Bytes` must point to at least `self.ByteCount` consecutive
        //   initialized, 1-byte-aligned `GifByteType` elements within a single live allocation
        //   (`<= isize::MAX` bytes) that is not mutated for the borrow of `&self`.
        // - Evidence: Guaranteed by the field safety invariant on `ExtensionBlock::Bytes` and the
        //   shared borrow `&self`.
        unsafe { self.Bytes.with_len(self.ByteCount) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY:
        // - Operation: `self.Bytes.with_len_mut(self.ByteCount)`.
        // - Contract: `self.ByteCount >= 0`, and if `self.Bytes` is non-null and
        //   `self.ByteCount > 0`, `self.Bytes` must point to at least `self.ByteCount` consecutive
        //   initialized, 1-byte-aligned `GifByteType` elements within a single live allocation
        //   (`<= isize::MAX` bytes) valid for reads and writes and unaliased for the exclusive
        //   borrow of `&mut self`.
        // - Evidence: Guaranteed by the field safety invariant on `ExtensionBlock::Bytes` and the
        //   exclusive borrow `&mut self`.
        unsafe { self.Bytes.with_len_mut(self.ByteCount) }
    }
}

impl Clone for ExtensionBlock {
    fn clone(&self) -> Self {
        Self {
            ByteCount: self.ByteCount,
            Bytes: OwnedCBufPtr::clone_from_slice(self.bytes()),
            Function: self.Function,
        }
    }
}

impl ExtensionBlock {
    pub fn try_new(function: c_int, data: &[u8]) -> Result<Self, safer_cffi::AllocError> {
        let byte_count = c_int::try_from(data.len()).map_err(|_| safer_cffi::AllocError)?;
        let bytes = OwnedCBufPtr::try_clone_from_slice(data)?;
        Ok(Self { Function: function, ByteCount: byte_count, Bytes: bytes })
    }
}

// ---------------------------------------------------------------------------
//  SavedImage
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug)]
pub struct SavedImage {
    pub ImageDesc: GifImageDesc,
    // Safety invariant: Either `RasterBits` is null (e.g. before raster allocation or when
    // `self.size()` is `None`), or `self.size()` is `Some(n)` (`n > 0`) and `RasterBits` points
    // to a `LibcAlloc`-allocated array of at least `n` consecutive initialized `GifByteType`
    // elements within a single live allocation (`n as usize <= isize::MAX as usize`), uniquely
    // owned by `self`.
    pub RasterBits: OwnedCBufPtr<GifByteType>,
    pub ExtensionBlockCount: c_int,
    // Safety invariant: `ExtensionBlockCount >= 0`, and either `ExtensionBlocks` is null with
    // `ExtensionBlockCount == 0`, or `ExtensionBlocks` points to a `LibcAlloc`-allocated array of
    // exactly `ExtensionBlockCount` consecutive initialized `ExtensionBlock` elements within a
    // single live allocation (`ExtensionBlockCount as usize * size_of::<ExtensionBlock>() <= isize::MAX as usize`),
    // uniquely owned by `self` with length-only capacity (`capacity == ExtensionBlockCount`).
    pub ExtensionBlocks: CBufPtr<ExtensionBlock>,
}

// Array accessors
impl SavedImage {
    /// Returns the validated size of the raster bits in bytes, or `None` if the image
    /// dimensions are out of range (0 or >65535).
    ///
    /// # Safety-usable invariant
    ///
    /// Returns `Some(n)` if and only if `1 <= self.ImageDesc.Width <= 65535`,
    /// `1 <= self.ImageDesc.Height <= 65535`, and `n = Width.checked_mul(Height)` succeeds
    /// without `c_int` overflow; in that case `1 <= n <= c_int::MAX` and
    /// `n as usize <= isize::MAX as usize`. Otherwise returns `None`.
    pub fn size(&self) -> Option<c_int> {
        match (self.ImageDesc.Width, self.ImageDesc.Height) {
            (1..=65535, 1..=65535) => self.ImageDesc.Width.checked_mul(self.ImageDesc.Height),
            _ => None,
        }
    }

    pub fn raster_bits(&self) -> &[u8] {
        let size = self.size().unwrap_or(0);
        // SAFETY:
        // - Operation: `self.RasterBits.with_len(size)`.
        // - Contract: `size >= 0`, and if `self.RasterBits` is non-null and `size > 0`,
        //   `self.RasterBits` must point to at least `size` consecutive initialized,
        //   1-byte-aligned `GifByteType` elements within a single live allocation
        //   (`<= isize::MAX` bytes) that is not mutated for the borrow of `&self`.
        // - Evidence: By the safety-usable invariant of `SavedImage::size`, `size >= 0` and
        //   `size as usize <= isize::MAX as usize`. When `self.RasterBits` is null or `size == 0`,
        //   `CBufPtr::with_len` returns `&[]` without dereferencing `self.RasterBits`. When
        //   `self.RasterBits` is non-null and `size > 0`, the field safety invariant on
        //   `SavedImage::RasterBits` and the shared borrow `&self` guarantee `self.RasterBits`
        //   points to `size` initialized `GifByteType` bytes in a single live allocation.
        unsafe { self.RasterBits.with_len(size) }
    }

    pub fn raster_bits_mut(&mut self) -> &mut [u8] {
        let size = self.size().unwrap_or(0);
        // SAFETY:
        // - Operation: `self.RasterBits.with_len_mut(size)`.
        // - Contract: `size >= 0`, and if `self.RasterBits` is non-null and `size > 0`,
        //   `self.RasterBits` must point to at least `size` consecutive initialized,
        //   1-byte-aligned `GifByteType` elements within a single live allocation
        //   (`<= isize::MAX` bytes) valid for reads and writes and unaliased for the exclusive
        //   borrow of `&mut self`.
        // - Evidence: By the safety-usable invariant of `SavedImage::size`, `size >= 0` and
        //   `size as usize <= isize::MAX as usize`. When `self.RasterBits` is null or `size == 0`,
        //   `CBufPtr::with_len_mut` returns `&mut []` without dereferencing `self.RasterBits`.
        //   When `self.RasterBits` is non-null and `size > 0`, the field safety invariant on
        //   `SavedImage::RasterBits` and the exclusive borrow `&mut self` guarantee
        //   `self.RasterBits` points to `size` initialized `GifByteType` bytes in a single live
        //   allocation.
        unsafe { self.RasterBits.with_len_mut(size) }
    }

    pub fn raster_bits_row_mut(&mut self, row: usize) -> Option<&mut [u8]> {
        let start = row.checked_mul(self.ImageDesc.Width as usize)?;
        let end = start.checked_add(self.ImageDesc.Width as usize)?;
        self.raster_bits_mut().get_mut(start..end)
    }

    pub fn extension_blocks(&self) -> &[ExtensionBlock] {
        // SAFETY:
        // - Operation: `self.ExtensionBlocks.with_len(self.ExtensionBlockCount)`.
        // - Contract: `self.ExtensionBlockCount >= 0`, and if `self.ExtensionBlocks` is non-null
        //   and `self.ExtensionBlockCount > 0`, `self.ExtensionBlocks` must point to at least
        //   `self.ExtensionBlockCount` consecutive initialized, properly aligned `ExtensionBlock`
        //   elements within a single live allocation (`<= isize::MAX` bytes) that is not mutated
        //   for the borrow of `&self`.
        // - Evidence: Guaranteed by the field safety invariant on `SavedImage::ExtensionBlocks`
        //   and the shared borrow `&self`.
        unsafe { self.ExtensionBlocks.with_len(self.ExtensionBlockCount) }
    }

    pub fn extension_blocks_mut(&mut self) -> &mut [ExtensionBlock] {
        // SAFETY:
        // - Operation: `self.ExtensionBlocks.with_len_mut(self.ExtensionBlockCount)`.
        // - Contract: `self.ExtensionBlockCount >= 0`, and if `self.ExtensionBlocks` is non-null
        //   and `self.ExtensionBlockCount > 0`, `self.ExtensionBlocks` must point to at least
        //   `self.ExtensionBlockCount` consecutive initialized, properly aligned `ExtensionBlock`
        //   elements within a single live allocation (`<= isize::MAX` bytes) valid for reads and
        //   writes and unaliased for the exclusive borrow of `&mut self`.
        // - Evidence: Guaranteed by the field safety invariant on `SavedImage::ExtensionBlocks`
        //   and the exclusive borrow `&mut self`.
        unsafe { self.ExtensionBlocks.with_len_mut(self.ExtensionBlockCount) }
    }

    pub fn extension_blocks_vec(&mut self) -> CVecRefMut<'_, ExtensionBlock, c_int> {
        // SAFETY:
        // - Operation: `self.ExtensionBlocks.as_vec_mut(&mut self.ExtensionBlockCount)`.
        // - Contract: `self.ExtensionBlockCount >= 0`, and either `self.ExtensionBlocks` is null
        //   with `self.ExtensionBlockCount == 0`, or `self.ExtensionBlocks` points to a
        //   `LibcAlloc`-allocated array of `self.ExtensionBlockCount` initialized `ExtensionBlock`
        //   elements with length-only capacity (`capacity == self.ExtensionBlockCount`).
        // - Evidence: Guaranteed by the field safety invariant on `SavedImage::ExtensionBlocks`
        //   and the split exclusive borrows `&mut self.ExtensionBlocks` and
        //   `&mut self.ExtensionBlockCount`.
        unsafe { self.ExtensionBlocks.as_vec_mut(&mut self.ExtensionBlockCount) }
    }
}

impl Drop for SavedImage {
    fn drop(&mut self) {
        self.extension_blocks_vec().clear();
    }
}

impl Clone for SavedImage {
    fn clone(&self) -> Self {
        Self {
            ImageDesc: self.ImageDesc.clone(),
            RasterBits: OwnedCBufPtr::clone_from_slice(self.raster_bits()),
            ExtensionBlockCount: self.ExtensionBlockCount,
            ExtensionBlocks: CBufPtr::clone_and_leak(self.extension_blocks()),
        }
    }
}

impl SavedImage {
    pub fn new(desc: GifImageDesc) -> Self {
        Self {
            ImageDesc: desc,
            RasterBits: OwnedCBufPtr::null(),
            ExtensionBlockCount: 0,
            ExtensionBlocks: CBufPtr::null(),
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
    // Safety invariant: `ImageCount >= 0`, and either `SavedImages` is null with
    // `ImageCount == 0`, or `SavedImages` points to a `LibcAlloc`-allocated array of exactly
    // `ImageCount` consecutive initialized `SavedImage` elements within a single live allocation
    // (`ImageCount as usize * size_of::<SavedImage>() <= isize::MAX as usize`), uniquely owned by
    // `self` with length-only capacity (`capacity == ImageCount`).
    pub SavedImages: CBufPtr<SavedImage>,
    pub ExtensionBlockCount: c_int,
    // Safety invariant: `ExtensionBlockCount >= 0`, and either `ExtensionBlocks` is null with
    // `ExtensionBlockCount == 0`, or `ExtensionBlocks` points to a `LibcAlloc`-allocated array of
    // exactly `ExtensionBlockCount` consecutive initialized `ExtensionBlock` elements within a
    // single live allocation
    // (`ExtensionBlockCount as usize * size_of::<ExtensionBlock>() <= isize::MAX as usize`),
    // uniquely owned by `self` with length-only capacity (`capacity == ExtensionBlockCount`).
    pub ExtensionBlocks: CBufPtr<ExtensionBlock>,
    pub Error: c_int,
    pub UserData: UserData,
    pub Private: Box<crate::private::GifFilePrivateType>,
}

// Array accessors
impl GifFileType {
    pub fn saved_images(&self) -> &[SavedImage] {
        // SAFETY:
        // - Operation: `self.SavedImages.with_len(self.ImageCount)`.
        // - Contract: `self.ImageCount >= 0`, and if `self.SavedImages` is non-null and
        //   `self.ImageCount > 0`, `self.SavedImages` must point to at least `self.ImageCount`
        //   consecutive initialized, properly aligned `SavedImage` elements within a single live
        //   allocation (`<= isize::MAX` bytes) that is not mutated for the borrow of `&self`.
        // - Evidence: Guaranteed by the field safety invariant on `GifFileType::SavedImages` and
        //   the shared borrow `&self`.
        unsafe { self.SavedImages.with_len(self.ImageCount) }
    }

    pub fn saved_images_mut(&mut self) -> &mut [SavedImage] {
        // SAFETY:
        // - Operation: `self.SavedImages.with_len_mut(self.ImageCount)`.
        // - Contract: `self.ImageCount >= 0`, and if `self.SavedImages` is non-null and
        //   `self.ImageCount > 0`, `self.SavedImages` must point to at least `self.ImageCount`
        //   consecutive initialized, properly aligned `SavedImage` elements within a single live
        //   allocation (`<= isize::MAX` bytes) valid for reads and writes and unaliased for the
        //   exclusive borrow of `&mut self`.
        // - Evidence: Guaranteed by the field safety invariant on `GifFileType::SavedImages` and
        //   the exclusive borrow `&mut self`.
        unsafe { self.SavedImages.with_len_mut(self.ImageCount) }
    }

    pub fn saved_images_vec(&mut self) -> CVecRefMut<'_, SavedImage, c_int> {
        // SAFETY:
        // - Operation: `self.SavedImages.as_vec_mut(&mut self.ImageCount)`.
        // - Contract: `self.ImageCount >= 0`, and either `self.SavedImages` is null with
        //   `self.ImageCount == 0`, or `self.SavedImages` points to a `LibcAlloc`-allocated array
        //   of `self.ImageCount` initialized `SavedImage` elements with length-only capacity
        //   (`capacity == self.ImageCount`).
        // - Evidence: Guaranteed by the field safety invariant on `GifFileType::SavedImages` and
        //   the split exclusive borrows `&mut self.SavedImages` and `&mut self.ImageCount`.
        unsafe { self.SavedImages.as_vec_mut(&mut self.ImageCount) }
    }

    pub fn extension_blocks(&self) -> &[ExtensionBlock] {
        // SAFETY:
        // - Operation: `self.ExtensionBlocks.with_len(self.ExtensionBlockCount)`.
        // - Contract: `self.ExtensionBlockCount >= 0`, and if `self.ExtensionBlocks` is non-null
        //   and `self.ExtensionBlockCount > 0`, `self.ExtensionBlocks` must point to at least
        //   `self.ExtensionBlockCount` consecutive initialized, properly aligned `ExtensionBlock`
        //   elements within a single live allocation (`<= isize::MAX` bytes) that is not mutated
        //   for the borrow of `&self`.
        // - Evidence: Guaranteed by the field safety invariant on `GifFileType::ExtensionBlocks`
        //   and the shared borrow `&self`.
        unsafe { self.ExtensionBlocks.with_len(self.ExtensionBlockCount) }
    }

    pub fn extension_blocks_vec(&mut self) -> CVecRefMut<'_, ExtensionBlock, c_int> {
        // SAFETY:
        // - Operation: `self.ExtensionBlocks.as_vec_mut(&mut self.ExtensionBlockCount)`.
        // - Contract: `self.ExtensionBlockCount >= 0`, and either `self.ExtensionBlocks` is null
        //   with `self.ExtensionBlockCount == 0`, or `self.ExtensionBlocks` points to a
        //   `LibcAlloc`-allocated array of `self.ExtensionBlockCount` initialized `ExtensionBlock`
        //   elements with length-only capacity (`capacity == self.ExtensionBlockCount`).
        // - Evidence: Guaranteed by the field safety invariant on `GifFileType::ExtensionBlocks`
        //   and the split exclusive borrows `&mut self.ExtensionBlocks` and
        //   `&mut self.ExtensionBlockCount`.
        unsafe { self.ExtensionBlocks.as_vec_mut(&mut self.ExtensionBlockCount) }
    }
}

impl Drop for GifFileType {
    fn drop(&mut self) {
        self.saved_images_vec().clear();
        self.extension_blocks_vec().clear();
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
            SavedImages: CBufPtr::null(),
            ExtensionBlockCount: 0,
            ExtensionBlocks: CBufPtr::null(),
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

// SAFETY:
// - Operation: `unsafe impl Send for UserData`.
// - Contract: Moving `UserData` across thread boundaries must not cause data races or violate
//   thread-locality invariants in safe Rust code.
// - Evidence: The inner `*mut c_void` field is private to this module and is never dereferenced,
//   read, or written by any Rust code in this crate; it is an opaque caller-supplied context
//   pointer stored in `GifFileType` and only accessed by foreign C callbacks under `DGifOpen` /
//   `EGifOpen`'s `# Safety` contract.
unsafe impl Send for UserData {}

// ---------------------------------------------------------------------------
//  Callback wrappers
// ---------------------------------------------------------------------------

/// Wraps a C read callback to centralize the safety decision.
///
/// # Safety Invariants
///
/// - The function pointer is valid for the lifetime of `self`.
/// - The function pointer is safe to call with a valid `GifFileType*` and buffer arguments, i.e. if
///   `*mut GifByteType` is a valid pointer to a writable buffer of at least the given non-negative
///   `c_int` size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadCallback(
    // Safety invariant: `0` is a valid C ABI function pointer for the lifetime of the owning
    // `GifFileType` handle, and invoking `(self.0)(gif, buf, len)` is sound whenever `gif` is a
    // valid pointer to the active `GifFileType`, `len >= 0`, and `buf` is a non-null pointer valid
    // for writes of `len as usize` bytes within a single live allocation for the duration of the
    // callback call (without unwinding across FFI or invalidating `*gif`).
    unsafe extern "C" fn(*mut GifFileType, *mut GifByteType, c_int) -> c_int,
);

impl ReadCallback {
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `func` is a valid C ABI function pointer that remains valid to invoke for the lifetime of
    ///   the `GifFileType` handle that stores this `ReadCallback`.
    /// - Whenever `func(gif, buf, len)` is called with a valid pointer `gif` to that `GifFileType`,
    ///   `len >= 0`, and a non-null pointer `buf` valid for writes of `len as usize` bytes within a
    ///   single live allocation for the duration of the call, `func` writes at most `len as usize`
    ///   bytes to `buf`, does not read uninitialized bytes from `buf`, does not unwind across the
    ///   FFI boundary, and does not deallocate or invalidate `*gif`.
    #[rustfmt::skip]
    pub(crate) unsafe fn new(
        func: unsafe extern "C" fn(*mut c_types_gen::GifFileType, *mut GifByteType, c_int) -> c_int,
    ) -> Self {
        // SAFETY:
        // - Operation: `core::mem::transmute` between two `unsafe extern "C" fn` pointer types
        //   differing only in `*mut c_types_gen::GifFileType` vs `*mut crate::c_types::GifFileType`.
        // - Same size & bit validity: Both types are thin `extern "C"` function pointers of equal
        //   size and alignment on the target platform, and `func` is a valid initialized function
        //   pointer.
        // - ABI equivalence: `c_types_gen::GifFileType` (generated by bindgen from `gif_lib.h`) and
        //   `crate::c_types::GifFileType` (`#[repr(C)]`, layout-verified by `gif_signatures_test`)
        //   have identical C ABI layout, and thin raw pointers `*mut T` have identical C ABI
        //   representation and calling convention.
        // - Ownership & drop: Function pointers are `Copy` and have no drop glue.
        // - Invariant establishment: By this function's `# Safety` precondition, `func` upholds
        //   the safety invariant of `ReadCallback.0`.
        let func = unsafe {
            core::mem::transmute::<
                unsafe extern "C" fn(*mut c_types_gen::GifFileType, *mut GifByteType, c_int) -> c_int,
                unsafe extern "C" fn(*mut crate::c_types::GifFileType, *mut GifByteType, c_int) -> c_int,
            >(func)
        };
        Self(func)
    }

    pub(crate) fn read(&self, gif: *mut GifFileType, buf: &mut [u8]) -> c_int {
        let len = c_int::try_from(buf.len()).unwrap_or(c_int::MAX);
        // SAFETY:
        // - Operation: invoking `(self.0)(gif, buf.as_mut_ptr(), len)`.
        // - Contract: `self.0` requires a valid `GifFileType*`, a non-negative `len: c_int`, and a
        //   non-null pointer `buf.as_mut_ptr()` valid for writes of `len as usize` bytes within a
        //   single live allocation for the duration of the call.
        // - Evidence: `len` is converted from `buf.len()` via `c_int::try_from(buf.len()).unwrap_or(c_int::MAX)`,
        //   so `0 <= len` and `len as usize <= buf.len()`. Because `buf: &mut [u8]` is an exclusive
        //   slice reference, `buf.as_mut_ptr()` is non-null, 1-byte aligned, and valid for writes
        //   of `len as usize` bytes in a single allocation, and the field safety invariant on
        //   `ReadCallback.0` guarantees `self.0` is safe to invoke with `(gif, buf.as_mut_ptr(), len)`.
        unsafe { (self.0)(gif, buf.as_mut_ptr(), len) }
    }
}

/// Wraps a C write callback to centralize the safety decision.
///
/// # Safety Invariants
///
/// - The function pointer is valid for the lifetime of the GIF handle.
/// - The function pointer is safe to call with a valid `GifFileType*` and buffer arguments, i.e.
///   `*const GifByteType` is a valid pointer to a readable buffer of at least the given
///   non-negative `c_int` size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WriteCallback(
    // Safety invariant: `0` is a valid C ABI function pointer for the lifetime of the owning
    // `GifFileType` handle, and invoking `(self.0)(gif, buf, len)` is sound whenever `gif` is a
    // valid pointer to the active `GifFileType`, `len >= 0`, and `buf` is a non-null pointer valid
    // for reads of `len as usize` initialized bytes within a single live allocation for the
    // duration of the callback call (without unwinding across FFI or invalidating `*gif`).
    unsafe extern "C" fn(*mut GifFileType, *const GifByteType, c_int) -> c_int,
);

impl WriteCallback {
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `func` is a valid C ABI function pointer that remains valid to invoke for the lifetime of
    ///   the `GifFileType` handle that stores this `WriteCallback`.
    /// - Whenever `func(gif, buf, len)` is called with a valid pointer `gif` to that `GifFileType`,
    ///   `len >= 0`, and a non-null pointer `buf` valid for reads of `len as usize` initialized
    ///   bytes within a single live allocation for the duration of the call, `func` reads at most
    ///   `len as usize` bytes from `buf`, does not mutate memory through `buf`, does not unwind
    ///   across the FFI boundary, and does not deallocate or invalidate `*gif`.
    #[rustfmt::skip]
    pub(crate) unsafe fn new(
        func: unsafe extern "C" fn(*mut c_types_gen::GifFileType, *const GifByteType, c_int) -> c_int,
    ) -> Self {
        // SAFETY:
        // - Operation: `core::mem::transmute` between two `unsafe extern "C" fn` pointer types
        //   differing only in `*mut c_types_gen::GifFileType` vs `*mut crate::c_types::GifFileType`.
        // - Same size & bit validity: Both types are thin `extern "C"` function pointers of equal
        //   size and alignment on the target platform, and `func` is a valid initialized function
        //   pointer.
        // - ABI equivalence: `c_types_gen::GifFileType` (generated by bindgen from `gif_lib.h`) and
        //   `crate::c_types::GifFileType` (`#[repr(C)]`, layout-verified by `gif_signatures_test`)
        //   have identical C ABI layout, and thin raw pointers `*mut T` have identical C ABI
        //   representation and calling convention.
        // - Ownership & drop: Function pointers are `Copy` and have no drop glue.
        // - Invariant establishment: By this function's `# Safety` precondition, `func` upholds
        //   the safety invariant of `WriteCallback.0`.
        let func = unsafe {
            core::mem::transmute::<
                unsafe extern "C" fn(*mut c_types_gen::GifFileType, *const GifByteType, c_int) -> c_int,
                unsafe extern "C" fn(*mut crate::c_types::GifFileType, *const GifByteType, c_int) -> c_int,
            >(func)
        };
        Self(func)
    }

    pub(crate) fn write(&self, gif: *mut GifFileType, buf: &[u8]) -> c_int {
        let len = c_int::try_from(buf.len()).unwrap_or(c_int::MAX);
        // SAFETY:
        // - Operation: invoking `(self.0)(gif, buf.as_ptr(), len)`.
        // - Contract: `self.0` requires a valid `GifFileType*`, a non-negative `len: c_int`, and a
        //   non-null pointer `buf.as_ptr()` valid for reads of `len as usize` initialized bytes
        //   within a single live allocation for the duration of the call.
        // - Evidence: `len` is converted from `buf.len()` via `c_int::try_from(buf.len()).unwrap_or(c_int::MAX)`,
        //   so `0 <= len` and `len as usize <= buf.len()`. Because `buf: &[u8]` is a shared slice
        //   reference, `buf.as_ptr()` is non-null, 1-byte aligned, and valid for reads of
        //   `len as usize` initialized bytes in a single allocation, and the field safety invariant
        //   on `WriteCallback.0` guarantees `self.0` is safe to invoke with `(gif, buf.as_ptr(), len)`.
        unsafe { (self.0)(gif, buf.as_ptr(), len) }
    }
}
