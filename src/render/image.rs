use std::path::PathBuf;
use once_cell::sync::Lazy;
use regex::Regex;
use lru::LruCache;
use std::num::NonZeroUsize;
use image::DynamicImage;

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
    media_dir: PathBuf,
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
            let img = image::open(&path).ok()?;
            self.cache.put(src.to_string(), img);
        }
        self.cache.get(src).cloned()
    }
}
