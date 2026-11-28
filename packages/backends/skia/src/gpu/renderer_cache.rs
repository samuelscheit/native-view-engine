//! Per-presenter `!Send` bounded `SlabLru` caches for Skia GPU/CPU rendering (`--features skia`):
//! zero-copy mmap base + inline variable-`wght` `Typeface` cache, `SkiaTextBlobCache`,
//! slot-keyed `SkiaImageCache` (in-place overwrite on URI swap, zero `Arc` pinning, zero per-frame
//! mutex/URI hashing), `SkiaPluginCache` (keyed by `(node_id, context_tag)` checking both
//! `plugin_epoch` and `SkiaTableRegistry::generation()`), and `MaskFilter` blur cache.

use nve_abi::EventBatchBuf;
use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore, Rgba};
#[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
use skia_safe::gpu::{self, DirectContext, DirectContextId};
use skia_safe::{
    images, AlphaType, BlurStyle, ColorType, Data, FontMgr, Image, ImageInfo, MaskFilter, TextBlob,
};
#[cfg(test)]
use skia_safe::Typeface;

use super::skia_graphics::GradientScratch;
use super::text_blob::{
    build_transient_text_blob, get_or_build_text_blob, BaseTypefaceEntry, CachedTextBlob,
    TextBlobKey, TypefaceBacking,
};
use crate::image_cache::{DecodedImage, ImageSlot};
use crate::raster::PixelBuffer;
use crate::slab_lru::{FastBuildHasher, IdentityU64BuildHasher, SlabLru};
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry};
use crate::text::{CosmicTextEngine, SkiaShapedRuns, TextAlign};
use crate::text_cache::canonical_f32;

pub(crate) const MAX_BASE_TYPEFACES: usize = 128;
/// Heap byte budget for in-memory (`Binary`) typefaces in `base_typefaces`; OS-mmap faces charge
/// `0` bytes toward this cap so large color emoji fonts are never evicted by heap budget.
pub(crate) const MAX_COPIED_TYPEFACE_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_TEXT_BLOBS: usize = 2048;
pub(crate) const MAX_TEXT_BLOB_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_SKIA_IMAGES: usize = 256;
pub(crate) const MAX_SKIA_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_SKIA_PLUGINS: usize = 256;
pub(crate) const MAX_SKIA_PLUGIN_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_MASK_FILTERS: usize = 64;
pub(crate) const MAX_MASK_FILTER_BYTES: usize = 64 * 1024;
/// Maximum retained plugin scratch buffer footprint (1 Mi pixels = 4 MiB RGBA); transient larger
/// plugin draws (up to 4Kx4K) release their oversized scratch buffers immediately after upload.
const MAX_RETAINED_PLUGIN_SCRATCH_PIXELS: usize = 1024 * 1024;
const MAX_RETAINED_PLUGIN_SCRATCH_RGBA_BYTES: usize = MAX_RETAINED_PLUGIN_SCRATCH_PIXELS * 4;

/// Live hit/miss/upload/eviction telemetry snapshot for [`super::SkiaCanvasRenderer`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RendererCacheStats {
    pub text_blob_hits: u64,
    pub text_blob_misses: u64,
    pub typeface_hits: u64,
    pub typeface_misses: u64,
    pub image_hits: u64,
    /// Subset of `image_hits` satisfied via the lock-free per-slot [`ImageSlotHandle`] fast path.
    pub image_fast_hits: u64,
    pub image_misses: u64,
    pub image_uploads: u64,
    pub plugin_hits: u64,
    pub plugin_misses: u64,
    pub plugin_rasterizations: u64,
    pub plugin_uploads: u64,
    pub mask_filter_hits: u64,
    pub mask_filter_misses: u64,
    pub evictions: u64,
    pub text_blob_entries: usize,
    pub typeface_entries: usize,
    pub typeface_mmap_entries: usize,
    pub typeface_copied_entries: usize,
    pub typeface_variant_entries: usize,
    pub image_entries: usize,
    pub plugin_entries: usize,
    pub mask_filter_entries: usize,
    pub typeface_bytes: usize,
    pub text_blob_bytes: usize,
    pub image_bytes: usize,
    pub plugin_bytes: usize,
    pub mask_filter_bytes: usize,
    pub live_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SkiaImageKey {
    slot: ImageSlot,
    context_tag: u32,
}

struct CachedSkiaImage {
    decoded_id: u64,
    width: u32,
    height: u32,
    image: Image,
    gpu_backed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SkiaPluginKey {
    node_id_raw: u32,
    context_tag: u32,
}

struct CachedPluginImage {
    plugin_epoch: u64,
    registry_generation: u64,
    pw: u32,
    ph: u32,
    eff_scale_bits: u32,
    inherited_text_color: u32,
    image: Option<Image>,
    gpu_backed: bool,
}

/// All per-presenter Skia caches and reusable scratch buffers. Owned inside a `RefCell` on
/// `!Send` `SkiaCanvasRenderer` so warm frames incur zero locking and zero heap churn.
pub(crate) struct SkiaRendererCaches {
    base_typefaces: SlabLru<cosmic_text::fontdb::ID, BaseTypefaceEntry, FastBuildHasher>,
    text_blobs: SlabLru<TextBlobKey, CachedTextBlob, IdentityU64BuildHasher>,
    images: SlabLru<SkiaImageKey, CachedSkiaImage, FastBuildHasher>,
    plugins: SlabLru<SkiaPluginKey, CachedPluginImage, FastBuildHasher>,
    mask_filters: SlabLru<u32, MaskFilter, FastBuildHasher>,
    shaped_scratch: SkiaShapedRuns,
    plugin_buf: PixelBuffer,
    plugin_rgba: Vec<u8>,
    pub(crate) grad_scratch: GradientScratch,
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    active_context_id: Option<DirectContextId>,
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    active_context_tag: u32,
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    next_context_tag: u32,
    stats: RendererCacheStats,
}

impl SkiaRendererCaches {
    pub(crate) fn new() -> Self {
        Self {
            // Mmap faces charge 0 bytes toward `MAX_COPIED_TYPEFACE_BYTES` (bounded by
            // `MAX_BASE_TYPEFACES`), while heap-backed `Binary` faces are strictly bounded by both.
            base_typefaces: SlabLru::with_initial_capacity_and_hasher(
                MAX_BASE_TYPEFACES,
                MAX_COPIED_TYPEFACE_BYTES,
                16,
                FastBuildHasher,
            ),
            text_blobs: SlabLru::with_initial_capacity_and_hasher(
                MAX_TEXT_BLOBS,
                MAX_TEXT_BLOB_BYTES,
                64,
                IdentityU64BuildHasher,
            ),
            images: SlabLru::with_initial_capacity_and_hasher(
                MAX_SKIA_IMAGES,
                MAX_SKIA_IMAGE_BYTES,
                32,
                FastBuildHasher,
            ),
            plugins: SlabLru::with_initial_capacity_and_hasher(
                MAX_SKIA_PLUGINS,
                MAX_SKIA_PLUGIN_BYTES,
                32,
                FastBuildHasher,
            ),
            mask_filters: SlabLru::with_initial_capacity_and_hasher(
                MAX_MASK_FILTERS,
                MAX_MASK_FILTER_BYTES,
                16,
                FastBuildHasher,
            ),
            shaped_scratch: SkiaShapedRuns::default(),
            plugin_buf: PixelBuffer::new_transparent_rgba(0, 0),
            plugin_rgba: Vec::new(),
            grad_scratch: GradientScratch::default(),
            #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
            active_context_id: None,
            #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
            active_context_tag: 0,
            #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
            next_context_tag: 1,
            stats: RendererCacheStats::default(),
        }
    }

    pub(crate) fn stats(&self) -> RendererCacheStats {
        let mut s = self.stats;
        s.text_blob_entries = self.text_blobs.len();
        s.typeface_entries = self.base_typefaces.len();
        let (mut mmap_count, mut copied_count, mut variant_count, mut raw_typeface_bytes) =
            (0usize, 0usize, 0usize, 0usize);
        for entry in self.base_typefaces.values() {
            match entry.backing {
                TypefaceBacking::Mmap => mmap_count += 1,
                TypefaceBacking::Heap => copied_count += 1,
                TypefaceBacking::Failed => {}
            }
            variant_count += entry.variants.len();
            raw_typeface_bytes = raw_typeface_bytes.saturating_add(entry.raw_bytes);
        }
        s.typeface_mmap_entries = mmap_count;
        s.typeface_copied_entries = copied_count;
        s.typeface_variant_entries = variant_count;
        s.image_entries = self.images.len();
        s.plugin_entries = self.plugins.len();
        s.mask_filter_entries = self.mask_filters.len();
        s.evictions = self
            .base_typefaces
            .evictions()
            .saturating_add(self.text_blobs.evictions())
            .saturating_add(self.images.evictions())
            .saturating_add(self.plugins.evictions())
            .saturating_add(self.mask_filters.evictions());
        s.typeface_bytes = raw_typeface_bytes;
        s.text_blob_bytes = self.text_blobs.live_bytes();
        s.image_bytes = self.images.live_bytes();
        s.plugin_bytes = self.plugins.live_bytes();
        s.mask_filter_bytes = self.mask_filters.live_bytes();
        s.live_bytes = s
            .typeface_bytes
            .saturating_add(s.text_blob_bytes)
            .saturating_add(s.image_bytes)
            .saturating_add(s.plugin_bytes)
            .saturating_add(s.mask_filter_bytes);
        s
    }

    pub(crate) fn reset_stats(&mut self) {
        self.stats = RendererCacheStats::default();
    }

    /// Drops every GPU-backed `skia_safe::Image` and any unreachable non-zero `context_tag`
    /// fallback/negative entries immediately. MUST be called before dropping or abandoning the
    /// associated Ganesh `DirectContext` / native GPU device.
    pub(crate) fn evict_gpu_entries(&mut self) {
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        {
            self.images.retain(|k, v| !v.gpu_backed && k.context_tag == 0);
            self.plugins.retain(|k, v| !v.gpu_backed && k.context_tag == 0);
            self.active_context_id = None;
            self.active_context_tag = 0;
        }
        #[cfg(not(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d")))]
        {
            self.images.retain(|_, v| !v.gpu_backed);
            self.plugins.retain(|_, v| !v.gpu_backed);
        }
    }

    pub(crate) fn clear_all(&mut self) {
        self.base_typefaces.clear();
        self.text_blobs.clear();
        self.images.clear();
        self.plugins.clear();
        self.mask_filters.clear();
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        {
            self.active_context_id = None;
            self.active_context_tag = 0;
        }
    }

    /// Synchronizes the cache's active Ganesh context tag. If `direct_context` is abandoned or
    /// belongs to a different `DirectContextId`, all stale GPU textures are evicted immediately.
    /// Returns the `context_tag` (`0` for CPU raster / abandoned context, non-zero for live GPU).
    #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
    pub(crate) fn sync_gpu_context(&mut self, direct_context: Option<&mut DirectContext>) -> u32 {
        let Some(dc) = direct_context else {
            if self.active_context_id.is_some() {
                self.evict_gpu_entries();
            }
            return 0;
        };
        if dc.abandoned() {
            if self.active_context_id.is_some() {
                self.evict_gpu_entries();
            }
            return 0;
        }
        let id = dc.id();
        if self.active_context_id != Some(id) {
            if self.active_context_id.is_some() {
                self.evict_gpu_entries();
            }
            let tag = self.next_context_tag.max(1);
            self.next_context_tag = tag.wrapping_add(1).max(1);
            self.active_context_id = Some(id);
            self.active_context_tag = tag;
        }
        self.active_context_tag
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn get_or_build_text_blob(
        &mut self,
        font_mgr: &FontMgr,
        text_engine: &CosmicTextEngine,
        text: &str,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        wrap_width: Option<f32>,
        align_width: f32,
    ) -> Option<TextBlob> {
        get_or_build_text_blob(
            font_mgr,
            text_engine,
            &mut self.base_typefaces,
            &mut self.text_blobs,
            &mut self.shaped_scratch,
            text,
            font_size,
            font_weight,
            text_align,
            wrap_width,
            align_width,
            &mut self.stats.text_blob_hits,
            &mut self.stats.text_blob_misses,
            &mut self.stats.typeface_hits,
            &mut self.stats.typeface_misses,
        )
    }

    /// Shapes a one-shot `TextBlob` bypassing `text_blobs` and its hit/miss telemetry.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_transient_text_blob(
        &mut self,
        font_mgr: &FontMgr,
        text_engine: &CosmicTextEngine,
        text: &str,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        wrap_width: Option<f32>,
        align_width: f32,
    ) -> Option<TextBlob> {
        build_transient_text_blob(
            font_mgr,
            text_engine,
            &mut self.base_typefaces,
            &mut self.shaped_scratch,
            text,
            font_size,
            font_weight,
            text_align,
            wrap_width,
            align_width,
            &mut self.stats.typeface_hits,
            &mut self.stats.typeface_misses,
        )
    }

    /// Returns a cached CTM-respecting Gaussian blur `MaskFilter` for `logical_sigma` (quantized
    /// to `1/64` px so HiDPI fractional sigmas like `0.25`/`0.3` logical px stay accurate without
    /// hitting an artificial `0.5` logical floor).
    pub(crate) fn get_or_create_blur_filter(&mut self, logical_sigma: f32) -> Option<MaskFilter> {
        if !logical_sigma.is_finite() || logical_sigma <= 0.0 {
            return None;
        }
        let clamped = logical_sigma.clamp(1.0 / 64.0, 16_384.0);
        let quantized = (clamped * 64.0).round().max(1.0) as u32;
        if let Some(hit) = self.mask_filters.get_mut(&quantized) {
            self.stats.mask_filter_hits = self.stats.mask_filter_hits.saturating_add(1);
            return Some(hit.clone());
        }
        self.stats.mask_filter_misses = self.stats.mask_filter_misses.saturating_add(1);
        let filter = MaskFilter::blur(BlurStyle::Normal, quantized as f32 / 64.0, true)?;
        self.mask_filters.insert(quantized, filter.clone(), 64);
        Some(filter)
    }

    /// Lock-free, zero-URI-hashing warm lookup by `(ImageSlot, decoded_id, context_tag)`.
    #[inline]
    pub(crate) fn get_cached_image_by_slot(
        &mut self,
        slot: ImageSlot,
        decoded_id: u64,
        context_tag: u32,
    ) -> Option<(Image, u32, u32)> {
        let key = SkiaImageKey { slot, context_tag };
        let entry = self.images.get_mut(&key)?;
        if entry.decoded_id != decoded_id {
            return None;
        }
        self.stats.image_hits = self.stats.image_hits.saturating_add(1);
        self.stats.image_fast_hits = self.stats.image_fast_hits.saturating_add(1);
        Some((entry.image.clone(), entry.width, entry.height))
    }

    /// Looks up or uploads `decoded` into `SkiaImageCache` keyed by `(ImageSlot, context_tag)`,
    /// overwriting any prior `decoded_id` for that slot in-place (zero stale texture pileup on
    /// repeated URI swaps). Oversized uncached images (`slot == None`) bypass cache insertion.
    pub(crate) fn get_or_upload_image_for_slot(
        &mut self,
        slot: Option<ImageSlot>,
        decoded: &DecodedImage,
        context_tag: u32,
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        direct_context: Option<&mut DirectContext>,
    ) -> Option<Image> {
        let decoded_id = decoded.id();
        if let Some(slot) = slot {
            let key = SkiaImageKey { slot, context_tag };
            if let Some(entry) = self.images.get_mut(&key)
                && entry.decoded_id == decoded_id
            {
                self.stats.image_hits = self.stats.image_hits.saturating_add(1);
                return Some(entry.image.clone());
            }
        }
        self.stats.image_misses = self.stats.image_misses.saturating_add(1);

        let info = ImageInfo::new(
            (decoded.width() as i32, decoded.height() as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let row_bytes = (decoded.width() as usize) * 4;
        let raster = images::raster_from_data(&info, Data::new_copy(decoded.rgba()), row_bytes)?;

        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        let (final_img, gpu_backed) = if context_tag != 0
            && let Some(dc) = direct_context
            && !dc.abandoned()
        {
            if let Some(tex) =
                gpu::images::texture_from_image(dc, &raster, gpu::Mipmapped::No, gpu::Budgeted::Yes)
            {
                self.stats.image_uploads = self.stats.image_uploads.saturating_add(1);
                (tex, true)
            } else {
                (raster, false)
            }
        } else {
            (raster, false)
        };
        #[cfg(not(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d")))]
        let (final_img, gpu_backed) = (raster, false);

        if let Some(slot) = slot {
            let key = SkiaImageKey { slot, context_tag };
            let bytes = decoded.rgba().len().max(64);
            self.images.insert(
                key,
                CachedSkiaImage {
                    decoded_id,
                    width: decoded.width(),
                    height: decoded.height(),
                    image: final_img.clone(),
                    gpu_backed,
                },
                bytes,
            );
        }
        Some(final_img)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn get_or_render_plugin(
        &mut self,
        node_id: NodeId,
        plugin_epoch: u64,
        pw: u32,
        ph: u32,
        eff_scale: f32,
        inherited_text_color: Rgba,
        store: &NodeStore,
        registry: &SkiaTableRegistry,
        events: &mut EventBatchBuf,
        context_tag: u32,
        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        direct_context: Option<&mut DirectContext>,
    ) -> Option<Image> {
        let eff_scale_bits = canonical_f32(eff_scale).to_bits();
        let registry_generation = registry.generation();
        let key = SkiaPluginKey {
            node_id_raw: node_id.raw(),
            context_tag,
        };
        if let Some(entry) = self.plugins.get_mut(&key)
            && entry.plugin_epoch == plugin_epoch
            && entry.registry_generation == registry_generation
            && entry.pw == pw
            && entry.ph == ph
            && entry.eff_scale_bits == eff_scale_bits
            && entry.inherited_text_color == inherited_text_color.0
        {
            self.stats.plugin_hits = self.stats.plugin_hits.saturating_add(1);
            return entry.image.clone();
        }
        self.stats.plugin_misses = self.stats.plugin_misses.saturating_add(1);
        self.stats.plugin_rasterizations = self.stats.plugin_rasterizations.saturating_add(1);

        self.plugin_buf.reset_transparent_rgba(pw, ph);
        let mut ctx = SkiaSystemPaintCtx {
            buffer: &mut self.plugin_buf,
            rect: LayoutRect {
                x: 0.0,
                y: 0.0,
                width: pw as f32,
                height: ph as f32,
            },
            clip_rect: None,
            scale_factor: eff_scale,
            opacity: 1.0,
            inherited_text_color,
            events,
        };
        let painted = registry.paint_node(&mut ctx, store, node_id);
        if painted == 0 {
            self.shrink_oversized_plugin_scratch();
            self.plugins.insert(
                key,
                CachedPluginImage {
                    plugin_epoch,
                    registry_generation,
                    pw,
                    ph,
                    eff_scale_bits,
                    inherited_text_color: inherited_text_color.0,
                    image: None,
                    gpu_backed: false,
                },
                64,
            );
            return None;
        }

        self.plugin_buf.write_rgba8_into(&mut self.plugin_rgba);
        let info = ImageInfo::new(
            (pw as i32, ph as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let raster =
            images::raster_from_data(&info, Data::new_copy(&self.plugin_rgba), (pw as usize) * 4);
        self.shrink_oversized_plugin_scratch();
        let raster = raster?;

        #[cfg(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d"))]
        let (final_img, gpu_backed) = if context_tag != 0
            && let Some(dc) = direct_context
            && !dc.abandoned()
        {
            if let Some(tex) =
                gpu::images::texture_from_image(dc, &raster, gpu::Mipmapped::No, gpu::Budgeted::Yes)
            {
                self.stats.plugin_uploads = self.stats.plugin_uploads.saturating_add(1);
                (tex, true)
            } else {
                (raster, false)
            }
        } else {
            (raster, false)
        };
        #[cfg(not(any(feature = "skia-metal", feature = "skia-vulkan", feature = "skia-d3d")))]
        let (final_img, gpu_backed) = (raster, false);

        let bytes = ((pw as usize) * (ph as usize) * 4).max(64);
        // Overwrites any previous-epoch texture for `(node_id_raw, context_tag)` in-place.
        self.plugins.insert(
            key,
            CachedPluginImage {
                plugin_epoch,
                registry_generation,
                pw,
                ph,
                eff_scale_bits,
                inherited_text_color: inherited_text_color.0,
                image: Some(final_img.clone()),
                gpu_backed,
            },
            bytes,
        );
        Some(final_img)
    }

    #[inline]
    fn shrink_oversized_plugin_scratch(&mut self) {
        self.plugin_buf
            .shrink_if_capacity_exceeds(MAX_RETAINED_PLUGIN_SCRATCH_PIXELS);
        if self.plugin_rgba.capacity() > MAX_RETAINED_PLUGIN_SCRATCH_RGBA_BYTES {
            self.plugin_rgba = Vec::new();
        }
    }

    #[cfg(test)]
    pub(crate) fn resolve_typeface_for_test(
        &mut self,
        font_mgr: &FontMgr,
        text_engine: &CosmicTextEngine,
        face_id: cosmic_text::fontdb::ID,
        weight: u16,
    ) -> Option<Typeface> {
        super::text_blob::resolve_typeface(
            font_mgr,
            text_engine,
            &mut self.base_typefaces,
            face_id,
            weight,
            &mut self.stats.typeface_hits,
            &mut self.stats.typeface_misses,
        )
    }

    #[cfg(test)]
    pub(crate) fn set_base_typefaces_cap_for_test(&mut self, max_entries: usize) {
        self.base_typefaces = SlabLru::with_initial_capacity_and_hasher(
            max_entries,
            MAX_COPIED_TYPEFACE_BYTES,
            max_entries.min(16),
            FastBuildHasher,
        );
    }

    #[cfg(test)]
    pub(crate) fn set_base_typefaces_byte_cap_for_test(&mut self, max_bytes: usize) {
        self.base_typefaces = SlabLru::with_initial_capacity_and_hasher(
            MAX_BASE_TYPEFACES,
            max_bytes,
            16,
            FastBuildHasher,
        );
    }

    #[cfg(test)]
    pub(crate) fn plugin_scratch_capacities(&self) -> (usize, usize) {
        (self.plugin_buf.pixel_capacity(), self.plugin_rgba.capacity())
    }
}
