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

//! Helper methods on C-compatible GIF types.
//!
//! ColorMapObject helpers are reimplementations of the algorithms in gifalloc.c.
//! `apply_translation` mirrors the C `GifApplyTranslation`.

use crate::c_types::{ColorMapObject, GifColorType, GifPixelType, SavedImage};
use core::ffi::c_int;

impl ColorMapObject {
    /// Return smallest bitfield size `n` will fit in.
    ///
    /// Matches `GifBitSize` in C (gifalloc.c).
    pub fn bit_size(n: c_int) -> c_int {
        // The C code: for(i=1;i<=8;i++){if((1<<i)>=n) break;} return i;
        // returns 9 when n > 256 (loop exits without break, i incremented past 8).
        (1..=8).find(|&i| (1 << i) >= n).unwrap_or(9)
    }

    /// Compute the union of two color maps.
    ///
    /// Matches `GifUnionColorMap` in C (gifalloc.c).
    pub fn union_with(
        &self,
        other: &ColorMapObject,
        trans: &mut [GifPixelType],
    ) -> Option<ColorMapObject> {
        let c1_colors = self.colors();
        let c2_colors = other.colors();

        let max_count = self.ColorCount.max(other.ColorCount).checked_mul(2)?;
        let mut color_union = ColorMapObject::new(max_count).ok()?;
        let mut cu_colors = color_union.colors_mut();

        cu_colors[..c1_colors.len()].copy_from_slice(c1_colors);

        // Back off past trailing black entries
        let mut crnt_slot = c1_colors
            .iter()
            .rposition(|c| c.Red != 0 || c.Green != 0 || c.Blue != 0)
            .map_or(0, |i| i + 1);

        // Merge c2 colors (reuse existing where possible)
        for (i, c2_color) in c2_colors.iter().enumerate() {
            if crnt_slot >= max_count as usize {
                break;
            }
            if let Some(j) = c1_colors.iter().position(|c| {
                c.Red == c2_color.Red && c.Green == c2_color.Green && c.Blue == c2_color.Blue
            }) {
                trans[i] = j as GifPixelType;
            } else {
                cu_colors[crnt_slot] = *c2_color;
                trans[i] = crnt_slot as GifPixelType;
                crnt_slot += 1;
            }
        }

        if crnt_slot > max_count as usize {
            return None;
        }

        let new_bit_size = Self::bit_size(crnt_slot as c_int);
        let round_up_to = 1usize << new_bit_size;

        // Bounds check: the C original has `if (RoundUpTo > MaxCount)` guard.
        if round_up_to > max_count as usize {
            return None;
        }

        let zero = GifColorType { Red: 0, Green: 0, Blue: 0 };
        cu_colors[crnt_slot..round_up_to].fill(zero);

        // If the final count differs from the allocation size, reallocate to
        // the correct size so that Drop (which uses ColorCount as the
        // deallocation length) doesn't cause an allocator layout mismatch.
        if round_up_to != max_count as usize {
            let mut result = ColorMapObject::from_slice(&cu_colors[..round_up_to]).ok()?;
            result.BitsPerPixel = new_bit_size;
            return Some(result);
        }

        // Drop the CSliceRefMut before accessing color_union fields directly.
        drop(cu_colors);
        color_union.ColorCount = round_up_to as c_int;
        color_union.BitsPerPixel = new_bit_size;
        Some(color_union)
    }
}

impl SavedImage {
    /// Apply a color translation table to the image's raster bits.
    pub fn apply_translation(&mut self, translation: &[GifPixelType; 256]) {
        for pixel in self.raster_bits_mut() {
            *pixel = translation[*pixel as usize];
        }
    }
}
