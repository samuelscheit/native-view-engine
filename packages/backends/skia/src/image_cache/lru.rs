//! Thread-safe bounded LRU image cache backed by [`crate::slab_lru::SlabLru`] with `O(1)`
//! zero-allocation touch, lock-free per-slot handle validation, and CLOCK second-chance recency.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::data_uri::decode_image_uri;
use super::png::decode_png;
use super::{
    DecodedImage, ImageDecodeError, ImageSlot, ImageSlotHandle, DEFAULT_MAX_CACHE_BYTES,
    DEFAULT_MAX_CACHE_ENTRIES, MAX_IMAGE_CACHE_SLOTS,
};
use crate::slab_lru::SlabLru;

/// Thread-safe bounded LRU cache for decoded PNG and raw RGBA images.
#[derive(Debug)]
pub struct ImageCache {
    inner: Mutex<SlabLru<Arc<str>, Arc<DecodedImage>>>,
    /// Per-slot monotonic `DecodedImage::id()` table written under `inner` lock (`0` = empty/evicted).
    /// Warm Skia draws validate an [`ImageSlotHandle`] in one lock-free `Acquire` load without
    /// invalidating unrelated slots when other images are inserted or oversized images miss.
    slot_decoded_ids: Box<[AtomicU64]>,
    /// Per-slot CLOCK second-chance bit set lock-free (`Relaxed`) on fast-path `is_handle_live`
    /// hits and consulted/cleared during eviction so hot every-frame images retain true LRU recency.
    slot_referenced: Box<[AtomicBool]>,
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

    /// Creates a new [`ImageCache`] bounded by both `max_entries` (clamped to
    /// [`MAX_IMAGE_CACHE_SLOTS`]) and `max_bytes`.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        let clamped_entries = max_entries.min(MAX_IMAGE_CACHE_SLOTS);
        let lru =
            SlabLru::with_initial_capacity(clamped_entries, max_bytes, clamped_entries.min(64));
        let slot_cap = lru.max_entries();
        let inner = Mutex::new(lru);
        // Eagerly prime OS mutex storage so steady-state lookups incur zero heap allocations.
        drop(inner.lock());
        let slot_decoded_ids = (0..slot_cap)
            .map(|_| AtomicU64::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let slot_referenced = (0..slot_cap)
            .map(|_| AtomicBool::new(false))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            inner,
            slot_decoded_ids,
            slot_referenced,
        }
    }

    #[inline]
    fn store_slot_id(&self, slot: u32, decoded_id: u64) {
        if let Some(cell) = self.slot_decoded_ids.get(slot as usize) {
            cell.store(decoded_id, Ordering::Release);
        }
        if let Some(flag) = self.slot_referenced.get(slot as usize) {
            flag.store(false, Ordering::Release);
        }
    }

    fn insert_locked(
        &self,
        guard: &mut SlabLru<Arc<str>, Arc<DecodedImage>>,
        key: Arc<str>,
        decoded: &Arc<DecodedImage>,
    ) -> Option<ImageSlotHandle> {
        let bytes = decoded.byte_len();
        let (slot_opt, _) = guard.insert_with_slot_and_second_chance(
            key,
            Arc::clone(decoded),
            bytes,
            |tail_idx| {
                self.slot_referenced
                    .get(tail_idx as usize)
                    .is_some_and(|flag| flag.swap(false, Ordering::AcqRel))
            },
            |evicted_idx| {
                self.store_slot_id(evicted_idx, 0);
            },
        );
        slot_opt.map(|slot| {
            self.store_slot_id(slot, decoded.id());
            ImageSlotHandle::new(ImageSlot::new(slot), decoded.id())
        })
    }

    /// Lock-free liveness check for an [`ImageSlotHandle`] (`Acquire` load on the handle's slot),
    /// marking the slot referenced for CLOCK second-chance LRU retention on hit.
    #[inline]
    pub fn is_handle_live(&self, handle: ImageSlotHandle) -> bool {
        let idx = handle.slot().index();
        let expected = handle.decoded_id();
        if expected != 0
            && self
                .slot_decoded_ids
                .get(idx)
                .is_some_and(|cell| cell.load(Ordering::Acquire) == expected)
        {
            if let Some(flag) = self.slot_referenced.get(idx) {
                flag.store(true, Ordering::Relaxed);
            }
            true
        } else {
            false
        }
    }

    /// Returns a cached image by key, updating its LRU recency in `O(1)` with zero heap allocations.
    pub fn get(&self, key: &str) -> Option<Arc<DecodedImage>> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.get_mut(key).map(|img| Arc::clone(img))
    }

    /// Inserts (or unconditionally replaces) a validated raw RGBA8888 buffer under `key` and
    /// returns its cached handle.
    pub fn insert_raw_rgba(
        &self,
        key: impl Into<String>,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        let decoded = Arc::new(DecodedImage::from_rgba(width, height, rgba)?);
        let arc_key: Arc<str> = Arc::from(key.into());
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.insert_locked(&mut guard, arc_key, &decoded);
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
        let arc_key: Arc<str> = Arc::from(key);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = guard.get_mut(key) {
            return Ok(Arc::clone(existing));
        }
        let _ = self.insert_locked(&mut guard, arc_key, &decoded);
        Ok(decoded)
    }

    /// Resolves `src` from the cache or decodes it on demand, returning
    /// `(Option<ImageSlotHandle>, Arc<DecodedImage>)`. Oversized images exceeding `max_bytes`
    /// return `None` for the handle without evicting or invalidating any cached slot.
    pub fn get_or_decode_with_handle(
        &self,
        src: &str,
    ) -> Result<(Option<ImageSlotHandle>, Arc<DecodedImage>), ImageDecodeError> {
        if src.is_empty() {
            return Err(ImageDecodeError::InvalidDataUri);
        }
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((slot, img)) = guard.get_slot_and_value_mut(src) {
                let handle = ImageSlotHandle::new(ImageSlot::new(slot), img.id());
                return Ok((Some(handle), Arc::clone(img)));
            }
        }
        let decoded = Arc::new(decode_image_uri(src)?);
        let arc_key: Arc<str> = Arc::from(src);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // Re-check under lock so concurrent cold decodes return the winning entry instead of
        // clobbering `slot_decoded_ids[slot]` and invalidating handles minted by the winner.
        if let Some((slot, img)) = guard.get_slot_and_value_mut(src) {
            let handle = ImageSlotHandle::new(ImageSlot::new(slot), img.id());
            return Ok((Some(handle), Arc::clone(img)));
        }
        let handle_opt = self.insert_locked(&mut guard, arc_key, &decoded);
        Ok((handle_opt, decoded))
    }

    /// Resolves `src` from the cache or decodes it on demand, returning `(Option<ImageSlot>, Arc<DecodedImage>)`.
    pub fn get_or_decode_with_slot(
        &self,
        src: &str,
    ) -> Result<(Option<ImageSlot>, Arc<DecodedImage>), ImageDecodeError> {
        self.get_or_decode_with_handle(src)
            .map(|(h, img)| (h.map(ImageSlotHandle::slot), img))
    }

    /// Peek-only lookup of an already-cached image into a sealed `Copy` [`ImageSlotHandle`]
    /// without pinning `Arc<DecodedImage>` or decoding PNG/`file://` payloads during commit/emit.
    pub fn resolve_cached_handle(&self, src: &str) -> Option<ImageSlotHandle> {
        if src.is_empty() {
            return None;
        }
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let (slot, img) = guard.peek_slot_and_value(src)?;
        Some(ImageSlotHandle::new(ImageSlot::new(slot), img.id()))
    }

    /// Resolves `src` from the cache or decodes it on demand (`data:image/png;base64,...`,
    /// `data:image/rgba;width=W;height=H;base64,...`, or `file://...`).
    pub fn get_or_decode(&self, src: &str) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        self.get_or_decode_with_handle(src).map(|(_, img)| img)
    }

    /// Registers raw PNG bytes under a deterministic content-addressed cache key and returns the key.
    pub fn register_png_bytes(&self, png_bytes: &[u8]) -> Result<String, ImageDecodeError> {
        let key = format!(
            "nve-mem://png/{:016x}-{}",
            nve_abi::xxh64(png_bytes, 0),
            png_bytes.len()
        );
        let _ = self.get_or_decode_png_bytes(&key, png_bytes)?;
        Ok(key)
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn total_bytes(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .live_bytes()
    }

    pub fn clear(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
        for cell in &self.slot_decoded_ids {
            cell.store(0, Ordering::Release);
        }
        for flag in &self.slot_referenced {
            flag.store(false, Ordering::Release);
        }
    }
}
