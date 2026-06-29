use std::path::{Path, PathBuf};
use once_cell::sync::Lazy;
use regex::Regex;
use lru::LruCache;
use std::num::NonZeroUsize;
use image::{DynamicImage, GenericImageView, Pixel, RgbaImage};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// Load an image file from disk (handles JPEG/PNG/etc. and SVG).
pub fn load_from_disk(path: &Path) -> Option<DynamicImage> {
    if path.extension().and_then(|e| e.to_str()) == Some("svg") {
        rasterize_svg(path)
    } else {
        image::open(path).ok()
    }
}

fn rasterize_svg(path: &Path) -> Option<DynamicImage> {
    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let w = size.width() as u32;
    let h = size.height() as u32;
    // Cap to a reasonable raster size to avoid huge allocations.
    let (w, h) = if w > 2048 || h > 2048 {
        let scale = 2048.0 / (w.max(h) as f32);
        ((w as f32 * scale) as u32, (h as f32 * scale) as u32)
    } else {
        (w.max(1), h.max(1))
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        w as f32 / size.width(),
        h as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia uses premultiplied RGBA; un-premultiply for image crate.
    let rgba = RgbaImage::from_raw(w, h, pixmap.take())?;
    Some(DynamicImage::ImageRgba8(rgba))
}

static IMG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<img[^>]+src="([^"]+)"[^>]*>"#).unwrap()
});

pub fn extract_srcs(html: &str) -> Vec<String> {
    IMG_RE.captures_iter(html)
        .map(|c| c[1].to_string())
        .collect()
}

pub struct ImageCache {
    cache: LruCache<String, DynamicImage>,
    pub media_dir: PathBuf,
}

impl ImageCache {
    pub fn new(media_dir: PathBuf) -> Self {
        Self {
            cache: LruCache::new(NonZeroUsize::new(20).unwrap()),
            media_dir,
        }
    }

    pub fn get(&mut self, src: &str) -> Option<DynamicImage> {
        if !self.cache.contains(src) {
            let path = self.media_dir.join(src);
            if !path.exists() { return None; }
            let img = load_from_disk(&path)?;
            self.cache.put(src.to_string(), img);
        }
        self.cache.get(src).cloned()
    }

    /// Store a pre-loaded image (e.g. from background thread) into the LRU cache.
    pub fn store(&mut self, src: &str, img: DynamicImage) {
        self.cache.put(src.to_string(), img);
    }

    /// Returns pixel (width, height) without cloning the image data.
    pub fn dimensions(&mut self, src: &str) -> Option<(u32, u32)> {
        self.get(src).map(|img| (img.width(), img.height()))
    }

}

/// Quadrant block characters indexed by 4-bit pattern (bit3=UL, bit2=UR, bit1=LL, bit0=LR).
/// Each char encodes which quadrants are "foreground" vs "background".
const QUAD: [char; 16] = [
    ' ', '▗', '▖', '▄',
    '▝', '▐', '▞', '▟',
    '▘', '▚', '▌', '▙',
    '▀', '▛', '▜', '█',
];

/// Render an image using Unicode quadrant blocks (▖▗▘▙ etc.) with truecolor fg/bg.
///
/// Each terminal cell covers a 2×2 pixel block: fg = mean of "bright" quadrants,
/// bg = mean of "dark" quadrants, character chosen by which quadrants are bright.
/// This gives 4× the pixel density of halfblocks.
pub fn to_quadrant_blocks(img: &DynamicImage, cell_w: u16, cell_h: u16) -> Vec<Line<'static>> {
    if cell_w == 0 || cell_h == 0 { return vec![]; }
    // Sample 2×2 image pixels per character cell; aspect ratio already corrected
    // by fit_dimensions (cell_h accounts for 2:1 terminal char height).
    let px_w = (cell_w as u32) * 2;
    let px_h = (cell_h as u32) * 2;
    // Triangle (bilinear) is fast enough at quadrant-block resolution.
    let resized = img.resize_exact(px_w, px_h, image::imageops::FilterType::Triangle);

    let mut lines = Vec::with_capacity(cell_h as usize);
    for row in (0..px_h).step_by(2) {
        let mut spans = Vec::with_capacity(cell_w as usize);
        for col in (0..px_w).step_by(2) {
            let pxs = [
                resized.get_pixel(col,                    row                   ).to_rgba(),
                resized.get_pixel((col+1).min(px_w-1),   row                   ).to_rgba(),
                resized.get_pixel(col,                    (row+1).min(px_h-1)  ).to_rgba(),
                resized.get_pixel((col+1).min(px_w-1),   (row+1).min(px_h-1)  ).to_rgba(),
            ];
            let lumas: [f32; 4] = pxs.map(|p| {
                0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
            });
            let lmin = lumas.iter().cloned().fold(f32::INFINITY, f32::min);
            let lmax = lumas.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

            let (ch, fg, bg) = if lmax - lmin < 30.0 {
                // Uniform block — solid fill avoids salt-and-pepper noise.
                let (r, g, b) = avg_rgb(&pxs, &[0, 1, 2, 3]);
                ('█', (r, g, b), (r, g, b))
            } else {
                let mid = (lmin + lmax) / 2.0;
                let bits = (0u8..4).fold(0u8, |acc, i| {
                    if lumas[i as usize] >= mid { acc | (8u8 >> i) } else { acc }
                });
                let bright: Vec<usize> = (0..4).filter(|&i| lumas[i] >= mid).collect();
                let dark:   Vec<usize> = (0..4).filter(|&i| lumas[i] <  mid).collect();
                let fg = if bright.is_empty() { (255,255,255) } else { avg_rgb(&pxs, &bright) };
                let bg = if dark.is_empty()   { (0,  0,  0  ) } else { avg_rgb(&pxs, &dark)   };
                (QUAD[bits as usize], fg, bg)
            };

            spans.push(Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(Color::Rgb(fg.0, fg.1, fg.2))
                    .bg(Color::Rgb(bg.0, bg.1, bg.2)),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn avg_rgb(pxs: &[image::Rgba<u8>; 4], idxs: &[usize]) -> (u8, u8, u8) {
    let n = idxs.len() as u32;
    let (r, g, b) = idxs.iter().fold((0u32, 0u32, 0u32), |(r, g, b), &i| {
        (r + pxs[i][0] as u32, g + pxs[i][1] as u32, b + pxs[i][2] as u32)
    });
    ((r / n) as u8, (g / n) as u8, (b / n) as u8)
}


/// Largest (w, h) in cells that fits `img` into `max_w × max_h`
/// while preserving aspect ratio. Each cell = ~1:2 pixel aspect.
pub fn fit_dimensions(img: &DynamicImage, max_w: u16, max_h: u16) -> (u16, u16) {
    if max_w == 0 || max_h == 0 { return (0, 0); }
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    let scale_w = max_w as f64 / iw;
    let scale_h = (max_h as f64 * 2.0) / ih;
    let scale = scale_w.min(scale_h);
    let w = ((iw * scale).round() as u16).clamp(1, max_w);
    let h = (((ih * scale) / 2.0).ceil() as u16).clamp(1, max_h);
    (w, h)
}
