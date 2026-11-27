//! Thread-safe bounded LRU image cache with `O(log n)` touch and eviction via a `BTreeMap`
//! recency index keyed by strictly monotonic access tick.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use super::data_uri::decode_image_uri;
use super::png::decode_png;
use super::{
    DecodedImage, ImageDecodeError, DEFAULT_MAX_CACHE_BYTES, DEFAULT_MAX_CACHE_ENTRIES,
};

#[derive(Debug)]
struct CacheEntry {
    image: Arc<DecodedImage>,
    bytes: usize,
    last_used: u64,
}

#[derive(Debug)]
struct LruInner {
    max_entries: usize,
    max_bytes: usize,
    total_bytes: usize,
    tick: u64,
    entries: HashMap<Arc<str>, CacheEntry>,
    /// Ordered index `last_used_tick -> key` enabling `O(log n)` `pop_first()` eviction
    /// instead of scanning every entry on eviction.
    recency: BTreeMap<u64, Arc<str>>,
}

impl LruInner {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
            total_bytes: 0,
            tick: 0,
            entries: HashMap::new(),
            recency: BTreeMap::new(),
        }
    }

    #[inline]
    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    fn get(&mut self, key: &str) -> Option<Arc<DecodedImage>> {
        let new_tick = self.next_tick();
        let entry = self.entries.get_mut(key)?;
        let old_tick = entry.last_used;
        entry.last_used = new_tick;
        let img = Arc::clone(&entry.image);
        if let Some(arc_key) = self.recency.remove(&old_tick) {
            self.recency.insert(new_tick, arc_key);
        }
        Some(img)
    }

    fn insert(&mut self, key: String, image: Arc<DecodedImage>) {
        let bytes = image.byte_len();
        if let Some(prev) = self.entries.remove(key.as_str()) {
            self.recency.remove(&prev.last_used);
            self.total_bytes = self.total_bytes.saturating_sub(prev.bytes);
        }
        if bytes > self.max_bytes {
            // Single image exceeds entire cache byte budget; caller still receives it, but skip caching.
            return;
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries
                || self.total_bytes.saturating_add(bytes) > self.max_bytes)
        {
            let Some((_, evict_key)) = self.recency.pop_first() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&evict_key) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.bytes);
            }
        }
        let tick = self.next_tick();
        let arc_key: Arc<str> = Arc::from(key);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.recency.insert(tick, Arc::clone(&arc_key));
        self.entries.insert(
            arc_key,
            CacheEntry {
                image,
                bytes,
                last_used: tick,
            },
        );
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.recency.clear();
        self.total_bytes = 0;
    }
}

/// Thread-safe bounded LRU cache for decoded PNG and raw RGBA images.
#[derive(Debug)]
pub struct ImageCache {
    inner: Mutex<LruInner>,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CACHE_ENTRIES, DEFAULT_MAX_CACHE_BYTES)
    }
}

impl ImageCache {
    /// Returns the process-wide shared [`ImageCache`].
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<ImageCache>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Self::default())))
    }

    /// Creates a new [`ImageCache`] bounded by both `max_entries` and `max_bytes`.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(LruInner::new(max_entries, max_bytes)),
        }
    }

    /// Returns a cached image by key, updating its LRU recency in `O(log n)`.
    pub fn get(&self, key: &str) -> Option<Arc<DecodedImage>> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key)
    }

    /// Inserts a validated raw RGBA8888 buffer under `key` and returns its cached handle.
    pub fn insert_raw_rgba(
        &self,
        key: impl Into<String>,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        let decoded = Arc::new(DecodedImage::from_rgba(width, height, rgba)?);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.into(), Arc::clone(&decoded));
        Ok(decoded)
    }

    /// Decodes raw PNG bytes and caches the resulting image under `key`.
    pub fn get_or_decode_png_bytes(
        &self,
        key: &str,
        png_bytes: &[u8],
    ) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        if let Some(hit) = self.get(key) {
            return Ok(hit);
        }
        let decoded = Arc::new(decode_png(png_bytes)?);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.to_owned(), Arc::clone(&decoded));
        Ok(decoded)
    }

    /// Resolves `src` from the cache or decodes it on demand (`data:image/png;base64,...`,
    /// `data:image/rgba;width=W;height=H;base64,...`, or `file://...`).
    pub fn get_or_decode(&self, src: &str) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        if src.is_empty() {
            return Err(ImageDecodeError::InvalidDataUri);
        }
        if let Some(hit) = self.get(src) {
            return Ok(hit);
        }
        let decoded = Arc::new(decode_image_uri(src)?);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(src.to_owned(), Arc::clone(&decoded));
        Ok(decoded)
    }

    /// Registers raw PNG bytes under a deterministic content-addressed cache key and returns the key.
    pub fn register_png_bytes(&self, png_bytes: &[u8]) -> Result<String, ImageDecodeError> {
        let key = format!("nve-mem://png/{:016x}-{}", nve_abi::xxh64(png_bytes, 0), png_bytes.len());
        let _ = self.get_or_decode_png_bytes(&key, png_bytes)?;
        Ok(key)
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn total_bytes(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .total_bytes
    }

    pub fn clear(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
    }
}
