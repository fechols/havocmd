//! Aspect-corrected, 16-color panorama authoring and column-major tile storage.
use image::{Rgb, RgbImage, imageops::FilterType};
use std::fmt;

pub const PIXEL_ASPECT: f64 = 1.6;
/// Splash sources are authored at twice the intended panorama size. The planner
/// never exceeds this scale, so every picture is halved (Lanczos3, in the same
/// resample pass as the aspect correction) before it is quantized.
pub const MAX_SCALE: f64 = 0.5;
pub const PER_PICTURE_BYTES: usize = 3 * 1024 * 1024;
// Four payloads minus the final 21 stereo16k3 assets, aligned to 256 bytes.
pub const TOTAL_ART_BYTES: usize = 38_951_424;
pub const TILE: u32 = 16;
pub const TILE_HEIGHT: u32 = 8;
pub const SEED_BYTES: usize = 32768;
pub const PALETTE: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xf7, 0xf7, 0xf7],
    [0x8d, 0x2f, 0x34],
    [0x6a, 0xd4, 0xcd],
    [0x98, 0x35, 0xa4],
    [0x4c, 0xb4, 0x42],
    [0x2c, 0x29, 0xb1],
    [0xef, 0xef, 0x5d],
    [0x98, 0x4e, 0x20],
    [0x5b, 0x38, 0x00],
    [0xd1, 0x67, 0x6d],
    [0x4a, 0x4a, 0x4a],
    [0x7b, 0x7b, 0x7b],
    [0x9f, 0xef, 0x93],
    [0x6d, 0x6a, 0xef],
    [0xb2, 0xb2, 0xb2],
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtError(pub String);
impl fmt::Display for ArtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ArtError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub storage_width: u32,
    pub storage_height: u32,
    pub bytes: usize,
}

fn layout(width: u32, height: u32, scale: f64, aspect: f64) -> Layout {
    let width = ((width as f64 / aspect * scale).round() as u32).max(1);
    let height = ((height as f64 * scale).round() as u32).max(1);
    let storage_width = width.div_ceil(TILE) * TILE;
    let storage_height = height.div_ceil(TILE_HEIGHT) * TILE_HEIGHT;
    Layout {
        width,
        height,
        storage_width,
        storage_height,
        bytes: SEED_BYTES + storage_width as usize * storage_height as usize / 2,
    }
}

/// One common reduction factor, at most MAX_SCALE, preserves relative source
/// scale as art is added.
/// The displayed width is logical width * pixel_aspect; source composition stays
/// intact. Edge padding to 16-pixel tiles is excluded from the authored viewport.
pub fn plan_layouts(
    sizes: &[(u32, u32)],
    aspect: f64,
    per_picture: usize,
    total: usize,
) -> Result<(f64, Vec<Layout>), ArtError> {
    if sizes.is_empty()
        || !aspect.is_finite()
        || aspect <= 0.0
        || sizes
            .iter()
            .any(|&(w, h)| w == 0 || h == 0 || w > 65535 || h > 65535)
        || per_picture < SEED_BYTES + 64
        || total / (SEED_BYTES + 64) < sizes.len()
    {
        return Err(ArtError(
            "invalid image dimensions, aspect, or art budget".into(),
        ));
    }
    let fits = |scale: f64| {
        let layouts: Vec<_> = sizes
            .iter()
            .map(|&(w, h)| layout(w, h, scale, aspect))
            .collect();
        layouts.iter().all(|l| l.bytes <= per_picture)
            && layouts
                .iter()
                .map(|l| l.bytes.div_ceil(crate::volume::ALIGNMENT) * crate::volume::ALIGNMENT)
                .sum::<usize>()
                <= total
    };
    let mut low = 0.0;
    let mut high = MAX_SCALE;
    if fits(high) {
        low = high;
    } else {
        for _ in 0..60 {
            let middle = (low + high) * 0.5;
            if fits(middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
    }
    Ok((
        low,
        sizes
            .iter()
            .map(|&(w, h)| layout(w, h, low, aspect))
            .collect(),
    ))
}

#[derive(Clone, Debug)]
pub struct IndexedPanorama {
    pub layout: Layout,
    pub pixels: Vec<u8>,
}

fn linear(value: u8) -> f32 {
    let v = value as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Lanczos resampling followed by linear-light serpentine Floyd-Steinberg error
/// diffusion. All stored pixels are palette indices; no VIC cell color limit is
/// imposed on this source format (the border renderer owns display constraints).
pub fn quantize(source: &RgbImage, layout: Layout) -> IndexedPanorama {
    let resized =
        image::imageops::resize(source, layout.width, layout.height, FilterType::Lanczos3);
    let palette: [[f32; 3]; 16] = PALETTE.map(|p| p.map(linear));
    let mut pixels = vec![0; layout.width as usize * layout.height as usize];
    let mut current = vec![[0f32; 3]; layout.width as usize + 2];
    let mut next = current.clone();
    for y in 0..layout.height {
        let reverse = y % 2 != 0;
        for step in 0..layout.width {
            let x = if reverse {
                layout.width - 1 - step
            } else {
                step
            };
            let at = x as usize + 1;
            let src = resized.get_pixel(x, y).0;
            let value: [f32; 3] =
                std::array::from_fn(|c| (linear(src[c]) + current[at][c]).clamp(0.0, 1.0));
            let mut best = 0;
            let mut best_error = f32::MAX;
            for (index, color) in palette.iter().enumerate() {
                let error = (value[0] - color[0]).powi(2) * 0.2126
                    + (value[1] - color[1]).powi(2) * 0.7152
                    + (value[2] - color[2]).powi(2) * 0.0722;
                if error < best_error {
                    best = index;
                    best_error = error;
                }
            }
            pixels[(y * layout.width + x) as usize] = best as u8;
            let direction: isize = if reverse { -1 } else { 1 };
            let ahead = (at as isize + direction) as usize;
            let behind = (at as isize - direction) as usize;
            for (c, &component) in value.iter().enumerate() {
                let error = component - palette[best][c];
                current[ahead][c] += error * (7.0 / 16.0);
                next[behind][c] += error * (3.0 / 16.0);
                next[at][c] += error * (5.0 / 16.0);
                next[ahead][c] += error * (1.0 / 16.0);
            }
        }
        std::mem::swap(&mut current, &mut next);
        next.fill([0.0; 3]);
    }
    IndexedPanorama { layout, pixels }
}

impl IndexedPanorama {
    pub fn pixel(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y.min(self.layout.height - 1) * self.layout.width
            + x.min(self.layout.width - 1)) as usize]
    }
    /// Version 2 payload: a render-native 32 KiB seed followed by X-major 16x8
    /// tiles. Each 64-byte tile has eight packed X columns of eight rows;
    /// even X is the low nibble, odd X the high nibble.
    pub fn pack_tiles(&self) -> Vec<u8> {
        let mut bytes = self.cache_256();
        bytes.reserve(self.layout.bytes.saturating_sub(SEED_BYTES));
        for tx in 0..self.layout.storage_width / TILE {
            for ty in 0..self.layout.storage_height / TILE_HEIGHT {
                for x in (0..TILE).step_by(2) {
                    for y in 0..TILE_HEIGHT {
                        bytes.push(
                            self.pixel(tx * TILE + x, ty * TILE_HEIGHT + y)
                                | self.pixel(tx * TILE + x + 1, ty * TILE_HEIGHT + y) << 4,
                        );
                    }
                }
            }
        }
        bytes
    }
    /// Direct raster cache: 128 byte columns of 256 rows. Each byte contains
    /// two adjacent X pixels, even X in the low nibble, as the raster expects.
    /// Tile-aligned near the center and clamped at small image edges.
    pub fn cache_256(&self) -> Vec<u8> {
        let (start_x, start_y) = self.seed_origin();
        let mut cache = Vec::with_capacity(32768);
        for x in (0..256).step_by(2) {
            for y in 0..256 {
                cache.push(
                    self.pixel(start_x + x, start_y + y)
                        | self.pixel(start_x + x + 1, start_y + y) << 4,
                );
            }
        }
        cache
    }
    pub fn seed_origin(&self) -> (u32, u32) {
        ((self.layout.width.saturating_sub(256) / 2) & !15,
         (self.layout.height.saturating_sub(256) / 2) & !7)
    }
    pub fn preview(&self, aspect: f64) -> RgbImage {
        let image = RgbImage::from_fn(self.layout.width, self.layout.height, |x, y| {
            Rgb(PALETTE[self.pixel(x, y) as usize])
        });
        let physical_width = (self.layout.width as f64 * aspect).round().max(1.0);
        let scale = (1200.0 / physical_width)
            .min(900.0 / self.layout.height as f64)
            .min(1.0);
        image::imageops::resize(
            &image,
            (physical_width * scale).round().max(1.0) as u32,
            (self.layout.height as f64 * scale).round().max(1.0) as u32,
            FilterType::Nearest,
        )
    }
}

pub fn unpack_tile_pixel(bytes: &[u8], layout: Layout, x: u32, y: u32) -> Result<u8, ArtError> {
    if bytes.len() != layout.bytes || x >= layout.storage_width || y >= layout.storage_height {
        return Err(ArtError("invalid tile extent or coordinate".into()));
    }
    let tile_index = (x / TILE) * (layout.storage_height / TILE_HEIGHT) + y / TILE_HEIGHT;
    let offset = SEED_BYTES + tile_index as usize * 64 + (x % TILE / 2) as usize * 8 + (y % TILE_HEIGHT) as usize;
    let byte = bytes[offset];
    Ok(if x.is_multiple_of(2) { byte & 15 } else { byte >> 4 })
}

pub fn slug(name: &str) -> String {
    let mut result = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
        } else if !result.is_empty() && !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-');
    let result = result.strip_suffix("-splash").unwrap_or(result);
    if result == "shattered-moon" { "lunar-awe".to_owned() } else { result.to_owned() }
}

pub const TRACK_SLUGS: [&str; 21] = [
    "galactic-dash",
    "hammond-rocker",
    "mission-failed",
    "lowland-approach",
    "crystal-cave",
    "iron-canyon",
    "canyon-gate",
    "ashen-wastes",
    "foundry-belt",
    "fortress-run",
    "storm-front",
    "meteor-reach",
    "carrier-coast",
    "carrier-assault",
    "asteroid-belt",
    "lunar-awe",
    "ship-graveyard",
    "citadel-core",
    "orbital-shipyard",
    "space-carrier",
    "jump-gate",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_scale_honors_both_budgets_and_physical_aspect() {
        let sizes = [(4096, 2048); 21];
        let (scale, layouts) =
            plan_layouts(&sizes, PIXEL_ASPECT, PER_PICTURE_BYTES, TOTAL_ART_BYTES).unwrap();
        assert!(scale <= MAX_SCALE);
        assert!(layouts.iter().all(|l| l.bytes <= PER_PICTURE_BYTES));
        assert!(
            layouts
                .iter()
                .map(|l| l.bytes.div_ceil(256) * 256)
                .sum::<usize>()
                <= TOTAL_ART_BYTES
        );
        assert!(layouts.windows(2).all(|pair| pair[0] == pair[1]));
        assert!(
            (layouts[0].width as f64 * PIXEL_ASPECT / layouts[0].height as f64 - 2.0).abs() < 0.01
        );
        assert!(plan_layouts(&[(0, 3)], 1.6, 1000, 1000).is_err());
    }
    #[test]
    fn tile_roundtrip_and_cache_crop_preserve_column_order() {
        let layout = Layout {
            width: 273,
            height: 271,
            storage_width: 288,
            storage_height: 272,
            bytes: SEED_BYTES + 288 * 272 / 2,
        };
        let panorama = IndexedPanorama {
            layout,
            pixels: (0..273 * 271)
                .map(|i| ((i / 273 * 3 + i % 273 * 7) % 16) as u8)
                .collect(),
        };
        let packed = panorama.pack_tiles();
        for y in 0..272 {
            for x in 0..288 {
                assert_eq!(
                    unpack_tile_pixel(&packed, layout, x, y).unwrap(),
                    panorama.pixel(x, y)
                );
            }
        }
        let cache = panorama.cache_256();
        assert_eq!(cache.len(), 32768);
        for x in 0..256 {
            for y in 0..256 {
                let byte = cache[(x / 2) * 256 + y];
                let pixel = if x % 2 == 0 { byte & 15 } else { byte >> 4 };
                assert_eq!(pixel, panorama.pixel(x as u32, y as u32));
            }
        }
        assert!(unpack_tile_pixel(&packed, layout, 288, 0).is_err());
    }
    #[test]
    fn exact_palette_pixels_survive_and_slug_removes_only_suffix() {
        let source = RgbImage::from_fn(16, 16, |x, _| Rgb(PALETTE[x as usize]));
        let layout = Layout {
            width: 16,
            height: 16,
            storage_width: 16,
            storage_height: 16,
            bytes: 128,
        };
        let indexed = quantize(&source, layout);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(indexed.pixel(x, y), x as u8);
            }
        }
        assert_eq!(slug("Lowland Approach Splash"), "lowland-approach");
        assert_eq!(slug("Splash Coast"), "splash-coast");
        assert_eq!(slug("Shattered Moon Splash"), "lunar-awe");
    }
    #[test]
    fn complete_catalog_halves_every_source_within_budget() {
        let (scale, layouts) = plan_layouts(&[(1448,1086);21], PIXEL_ASPECT,
            PER_PICTURE_BYTES,TOTAL_ART_BYTES).unwrap();
        assert_eq!(scale,MAX_SCALE);
        assert!(layouts.iter().all(|p| p.width==453 && p.height==543 &&
            p.storage_width==464 && p.storage_height==544 && p.bytes==158976));
        assert_eq!(layouts.iter().map(|p|p.bytes).sum::<usize>(),3338496);
        let l=layouts[0];
        let picture=IndexedPanorama {layout:l,pixels:vec![3;l.width as usize*l.height as usize]};
        assert_eq!(picture.seed_origin(),(96,136));
        let payload=picture.pack_tiles();
        assert_eq!(&payload[..SEED_BYTES],picture.cache_256());
        assert_eq!(payload.len(),l.bytes);
        assert_eq!(unpack_tile_pixel(&payload,l,463,543).unwrap(),3);
    }
}
