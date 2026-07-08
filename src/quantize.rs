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

//! Median-cut color quantization.
//!
//! Reimplementation of `quantize.c` from giflib — converts a 24-bit RGB image
//! into an indexed 8-bit image with an optimized color map.
//!
//! Based on: "Color Image Quantization for frame buffer Display", by
//! Paul Heckbert SIGGRAPH 1982 page 297-307.

use crate::c_types::{GifByteType, GifColorType};
use core::ffi::c_int;

const COLOR_ARRAY_SIZE: usize = 32768;
const BITS_PER_PRIM_COLOR: usize = 5;
const MAX_PRIM_COLOR: u8 = 0x1f;

#[derive(Clone)]
struct QuantizedColor {
    rgb: [u8; 3],
    new_color_index: u8,
    count: i64,
}

struct ColorSubdiv {
    rgb_min: [u8; 3],
    rgb_width: [u8; 3],
    num_entries: usize,
    count: i64,
    /// Indices into the color array for this subdivision.
    color_indices: Vec<usize>,
}

/// Quantize a 24-bit RGB image into an indexed image with at most
/// `*color_map_size` colors.
///
/// This is a faithful, bug-for-bug reimplementation of the C
/// `GifQuantizeBuffer`. The algorithm uses median-cut quantization.
///
/// Returns `true` on success (`GIF_OK`), `false` on error (`GIF_ERROR`).
pub fn gif_quantize_buffer(
    width: u32,
    height: u32,
    color_map_size: &mut c_int,
    red_input: &[GifByteType],
    green_input: &[GifByteType],
    blue_input: &[GifByteType],
    output_buffer: &mut [GifByteType],
    output_color_map: &mut [GifColorType],
) -> bool {
    let num_pixels = (width as usize) * (height as usize);

    // Initialize the color array with all possible 15-bit colors.
    let mut color_array = Vec::with_capacity(COLOR_ARRAY_SIZE);
    for i in 0..COLOR_ARRAY_SIZE {
        color_array.push(QuantizedColor {
            rgb: [
                (i >> (2 * BITS_PER_PRIM_COLOR)) as u8,
                ((i >> BITS_PER_PRIM_COLOR) & MAX_PRIM_COLOR as usize) as u8,
                (i & MAX_PRIM_COLOR as usize) as u8,
            ],
            new_color_index: 0,
            count: 0,
        });
    }

    // Sample the colors and their distribution.
    for i in 0..num_pixels {
        let index = ((red_input[i] >> (8 - BITS_PER_PRIM_COLOR)) as usize)
            << (2 * BITS_PER_PRIM_COLOR)
            | ((green_input[i] >> (8 - BITS_PER_PRIM_COLOR)) as usize) << BITS_PER_PRIM_COLOR
            | (blue_input[i] >> (8 - BITS_PER_PRIM_COLOR)) as usize;
        color_array[index].count += 1;
    }

    // Initialize subdivisions.
    let mut subdivs: Vec<ColorSubdiv> = Vec::with_capacity(256);
    for _ in 0..256 {
        subdivs.push(ColorSubdiv {
            rgb_min: [0, 0, 0],
            rgb_width: [255, 255, 255],
            num_entries: 0,
            count: 0,
            color_indices: Vec::new(),
        });
    }

    // Find the non-empty entries in the color table and chain them into
    // the first subdivision.
    let non_empty: Vec<usize> =
        (0..COLOR_ARRAY_SIZE).filter(|&i| color_array[i].count > 0).collect();

    if non_empty.is_empty() {
        // No colors sampled — nothing to quantize. Match C behavior: return OK
        // with empty color map.
        *color_map_size = 0;
        return true;
    }

    subdivs[0].color_indices = non_empty;
    subdivs[0].num_entries = subdivs[0].color_indices.len();
    subdivs[0].count = (width as i64) * (height as i64);

    let mut new_color_map_size: usize = 1;

    if !subdiv_color_map(
        &mut subdivs,
        *color_map_size as usize,
        &mut new_color_map_size,
        &mut color_array,
    ) {
        return false;
    }

    if new_color_map_size < *color_map_size as usize {
        // Clear rest of color map.
        for entry in
            output_color_map.iter_mut().take(*color_map_size as usize).skip(new_color_map_size)
        {
            entry.Red = 0;
            entry.Green = 0;
            entry.Blue = 0;
        }
    }

    // Average the colors in each entry to be the color to be used in the
    // output color map, and plug it into the output color map itself.
    for i in 0..new_color_map_size {
        let j = subdivs[i].num_entries;
        if j > 0 {
            let mut red: i64 = 0;
            let mut green: i64 = 0;
            let mut blue: i64 = 0;
            for &ci in &subdivs[i].color_indices {
                color_array[ci].new_color_index = i as u8;
                red += color_array[ci].rgb[0] as i64;
                green += color_array[ci].rgb[1] as i64;
                blue += color_array[ci].rgb[2] as i64;
            }
            output_color_map[i].Red = ((red << (8 - BITS_PER_PRIM_COLOR)) / j as i64) as u8;
            output_color_map[i].Green = ((green << (8 - BITS_PER_PRIM_COLOR)) / j as i64) as u8;
            output_color_map[i].Blue = ((blue << (8 - BITS_PER_PRIM_COLOR)) / j as i64) as u8;
        }
    }

    // Finally scan the input buffer again and put the mapped index in the
    // output buffer.
    for i in 0..num_pixels {
        let index = ((red_input[i] >> (8 - BITS_PER_PRIM_COLOR)) as usize)
            << (2 * BITS_PER_PRIM_COLOR)
            | ((green_input[i] >> (8 - BITS_PER_PRIM_COLOR)) as usize) << BITS_PER_PRIM_COLOR
            | (blue_input[i] >> (8 - BITS_PER_PRIM_COLOR)) as usize;
        output_buffer[i] = color_array[index].new_color_index;
    }

    *color_map_size = new_color_map_size as c_int;
    true
}

/// Recursively subdivide the RGB space using median cut until
/// `color_map_size` different cubes exist.
fn subdiv_color_map(
    subdivs: &mut Vec<ColorSubdiv>,
    color_map_size: usize,
    new_color_map_size: &mut usize,
    color_array: &mut [QuantizedColor],
) -> bool {
    while color_map_size > *new_color_map_size {
        // Find candidate for subdivision: the subdivision with the widest
        // dimension that has more than one entry.
        let mut max_size: i32 = -1;
        let mut index: usize = 0;
        let mut sort_rgb_axis: usize = 0;

        for i in 0..*new_color_map_size {
            for j in 0..3 {
                if (subdivs[i].rgb_width[j] as i32) > max_size && subdivs[i].num_entries > 1 {
                    max_size = subdivs[i].rgb_width[j] as i32;
                    index = i;
                    sort_rgb_axis = j;
                }
            }
        }

        if max_size == -1 {
            return true;
        }

        // Sort all elements in the entry along the given axis and split at
        // the median. We sort on all three axes (primary + secondary + tertiary)
        // to avoid instability from qsort, matching the C behavior.
        let mut sort_array: Vec<usize> = subdivs[index].color_indices.clone();
        sort_array.sort_by(|&a, &b| {
            let ca = &color_array[a];
            let cb = &color_array[b];
            let hash_a = (ca.rgb[sort_rgb_axis] as i32) * 256 * 256
                + (ca.rgb[(sort_rgb_axis + 1) % 3] as i32) * 256
                + (ca.rgb[(sort_rgb_axis + 2) % 3] as i32);
            let hash_b = (cb.rgb[sort_rgb_axis] as i32) * 256 * 256
                + (cb.rgb[(sort_rgb_axis + 1) % 3] as i32) * 256
                + (cb.rgb[(sort_rgb_axis + 2) % 3] as i32);
            hash_a.cmp(&hash_b)
        });

        // Relink into subdivs[index].
        subdivs[index].color_indices = sort_array;

        // Now simply add the Counts until we have half of the Count.
        let half_count = subdivs[index].count / 2;
        let mut sum = half_count - color_array[subdivs[index].color_indices[0]].count;
        let mut num_entries: usize = 1;
        let mut count: i64 = color_array[subdivs[index].color_indices[0]].count;

        let total_entries = subdivs[index].num_entries;
        let mut split_pos = 0;

        for pos in 0..total_entries - 1 {
            let next_idx = subdivs[index].color_indices[pos + 1];
            if sum - color_array[next_idx].count < 0 {
                split_pos = pos;
                break;
            }
            // Also check that there's at least one more entry after the next
            if pos + 2 >= total_entries {
                split_pos = pos;
                break;
            }
            sum -= color_array[next_idx].count;
            let cur_idx = subdivs[index].color_indices[pos + 1];
            num_entries += 1;
            count += color_array[cur_idx].count;
            split_pos = pos + 1;
        }

        // Save the values of the last color of the first half, and first of the
        // second half so we can update the bounding boxes later.
        let max_color_val = color_array[subdivs[index].color_indices[split_pos]].rgb[sort_rgb_axis];
        let min_color_val =
            color_array[subdivs[index].color_indices[split_pos + 1]].rgb[sort_rgb_axis];
        let max_color = (max_color_val as u32) << (8 - BITS_PER_PRIM_COLOR);
        let min_color = (min_color_val as u32) << (8 - BITS_PER_PRIM_COLOR);

        // Partition: second half goes to new_color_map_size.
        let second_half: Vec<usize> = subdivs[index].color_indices[split_pos + 1..].to_vec();
        subdivs[index].color_indices.truncate(split_pos + 1);

        let new_idx = *new_color_map_size;

        // Copy bounding box from parent.
        let parent_rgb_min = subdivs[index].rgb_min;
        let parent_rgb_width = subdivs[index].rgb_width;

        subdivs[new_idx].color_indices = second_half;
        subdivs[new_idx].count = subdivs[index].count - count;
        subdivs[index].count = count;
        subdivs[new_idx].num_entries = subdivs[index].num_entries - num_entries;
        subdivs[index].num_entries = num_entries;

        for j in 0..3 {
            subdivs[new_idx].rgb_min[j] = parent_rgb_min[j];
            subdivs[new_idx].rgb_width[j] = parent_rgb_width[j];
        }
        subdivs[new_idx].rgb_width[sort_rgb_axis] = (subdivs[new_idx].rgb_min[sort_rgb_axis] as u32
            + subdivs[new_idx].rgb_width[sort_rgb_axis] as u32
            - min_color) as u8;
        subdivs[new_idx].rgb_min[sort_rgb_axis] = min_color as u8;

        subdivs[index].rgb_width[sort_rgb_axis] =
            (max_color - subdivs[index].rgb_min[sort_rgb_axis] as u32) as u8;

        *new_color_map_size += 1;
    }

    true
}
