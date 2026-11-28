//! Unified `CosmicTextEngine` -> Skia `TextBlob` assembly and zero-copy mmap base + inline
//! variable-font `wght` variant typeface caching (`--features skia`).
//!
//! Eliminates `PREFERRED_SANS_FAMILIES` and `canvas.draw_str`: every text op (and HUD banner)
//! uses the exact `cosmic-text` HarfBuzz glyph IDs and logical `(x, y)` coordinates, materialized
//! once into a position- and color-independent `skia_safe::TextBlob` cached in `SlabLru`.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use nve_abi::Xxh64Hasher;
use skia_safe::font::Edging;
use skia_safe::font_arguments::{variation_position::Coordinate, VariationPosition};
use skia_safe::{
    Data, Font, FontArguments, FontHinting, FontMgr, Point, TextBlob, TextBlobBuilder, Typeface,
};

use crate::slab_lru::{FastBuildHasher, IdentityU64BuildHasher, SlabLru};
use crate::text::{CosmicTextEngine, SkiaShapedRuns, TextAlign};
use crate::text_cache::{FontFaceSource, TextLayoutParams};

/// Maximum number of variable-weight `wght` variants retained inline per base [`Typeface`].
pub(crate) const MAX_VARIANTS_PER_FACE: usize = 16;
/// Byte budget charged to `base_typefaces` for a cached negative (failed) typeface entry.
pub(crate) const FAILED_TYPEFACE_BYTES: usize = 64;

/// Thin `AsRef<[u8]> + Send + 'static` owner passed to `skia_safe::Data::new_with_owner` so Skia
/// borrows `fontdb`'s existing `Arc<Mmap>` (`SharedFile`) or `Arc<Vec<u8>>` (`Binary`) with zero
/// byte copies, zero double-mmap, and zero TOCTOU path races.
struct SharedFontBytes(Arc<dyn AsRef<[u8]> + Send + Sync>);

impl AsRef<[u8]> for SharedFontBytes {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

/// How a cached base [`Typeface`] was backed when decoded into Skia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypefaceBacking {
    /// Zero-copy OS `mmap` via `Data::new_with_owner` (`fontdb::Source::SharedFile`) or
    /// `SkData::MakeFromFileName` (`fontdb::Source::File`).
    Mmap,
    /// In-memory heap font buffer (`fontdb::Source::Binary`, zero-copy pinned via
    /// `Data::new_with_owner` and bounded by `MAX_COPIED_TYPEFACE_BYTES`).
    Heap,
    /// Cached negative entry after font decoding failed (prevents per-frame retry storms).
    Failed,
}

/// Base font-face entry cached once per `fontdb::ID` with inline bounded variable-`wght` variants
/// so evicting a base face drops its variants simultaneously without pinning evicted `SkData`.
pub(crate) struct BaseTypefaceEntry {
    pub(crate) typeface: Option<Typeface>,
    pub(crate) wght_range: Option<(f32, f32)>,
    pub(crate) variants: Vec<(u32, Typeface)>,
    pub(crate) backing: TypefaceBacking,
    /// Exact font data byte length reported in `RendererCacheStats::typeface_bytes` (even when
    /// mmap faces charge `0` bytes toward `MAX_COPIED_TYPEFACE_BYTES`).
    pub(crate) raw_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextBlobKey {
    pub(crate) hash: u64,
    pub(crate) text_len: u32,
    pub(crate) font_size_bits: u32,
    pub(crate) font_weight: u16,
    pub(crate) text_align: TextAlign,
    pub(crate) wrap_width_bits: u32,
    pub(crate) align_width_bits: u32,
}

impl Hash for TextBlobKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

#[derive(Debug)]
pub(crate) struct CachedTextBlob {
    pub(crate) text: String,
    pub(crate) blob: Option<TextBlob>,
}

#[inline]
pub(crate) fn make_text_blob_key(
    text: &str,
    font_size: f32,
    font_weight: u16,
    text_align: TextAlign,
    wrap_width: Option<f32>,
    align_width: f32,
) -> (TextBlobKey, TextLayoutParams) {
    let params = TextLayoutParams::new(font_size, font_weight, text_align, wrap_width, align_width);
    let fs_bits = params.font_size_bits();
    let ww_bits = params.wrap_width_bits();
    let aw_bits = params.align_width_bits();
    let mut h = Xxh64Hasher::new(0);
    h.update(text.as_bytes());
    h.update(&[0xFD, params.text_align as u8]);
    h.update(&params.font_weight.to_le_bytes());
    h.update_u32(fs_bits);
    h.update_u32(ww_bits);
    h.update_u32(aw_bits);
    let key = TextBlobKey {
        hash: h.digest(),
        text_len: text.len() as u32,
        font_size_bits: fs_bits,
        font_weight: params.font_weight,
        text_align: params.text_align,
        wrap_width_bits: ww_bits,
        align_width_bits: aw_bits,
    };
    (key, params)
}

#[inline]
fn derive_wght_variant(base_tf: &Typeface, clamped_wght: f32) -> Typeface {
    let coords = [Coordinate {
        axis: Coordinate::wght,
        value: clamped_wght,
    }];
    let pos = VariationPosition {
        coordinates: &coords,
    };
    let args = FontArguments::new().set_variation_design_position(pos);
    base_tf
        .clone_with_arguments(&args)
        .unwrap_or_else(|| base_tf.clone())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_typeface(
    font_mgr: &FontMgr,
    text_engine: &CosmicTextEngine,
    base_typefaces: &mut SlabLru<cosmic_text::fontdb::ID, BaseTypefaceEntry, FastBuildHasher>,
    face_id: cosmic_text::fontdb::ID,
    weight: u16,
    hits: &mut u64,
    misses: &mut u64,
) -> Option<Typeface> {
    // 1. Fast path: base face (and its inline variable-font variants) is already cached.
    if let Some(entry) = base_typefaces.get_mut(&face_id) {
        let Some(base_tf) = entry.typeface.as_ref() else {
            // Cached failed face load: never retry mmap/copy every frame.
            *hits = hits.saturating_add(1);
            return None;
        };
        let Some((min_w, max_w)) = entry.wght_range else {
            *hits = hits.saturating_add(1);
            return Some(base_tf.clone());
        };
        let clamped = (weight as f32).clamp(min_w, max_w);
        let wght_bits = clamped.to_bits();
        if let Some(pos) = entry.variants.iter().position(|(b, _)| *b == wght_bits) {
            // Rotate hit to the back (MRU) so `remove(0)` evicts the least-recently-used variant.
            entry.variants[pos..].rotate_left(1);
            *hits = hits.saturating_add(1);
            return entry.variants.last().map(|(_, tf)| tf.clone());
        }
        *misses = misses.saturating_add(1);
        let varied = derive_wght_variant(base_tf, clamped);
        if entry.variants.len() >= MAX_VARIANTS_PER_FACE {
            entry.variants.remove(0);
        }
        entry.variants.push((wght_bits, varied.clone()));
        return Some(varied);
    }

    // 2. Cold miss: resolve source metadata WITHOUT holding the `FontSystem` mutex during decode.
    *misses = misses.saturating_add(1);
    let loaded = text_engine.resolve_face_source(face_id).and_then(|src| match src {
        FontFaceSource::File { path, face_index } => {
            let data = Data::from_filename(&path)?;
            let actual_len = data.len();
            font_mgr
                .new_from_data(data, Some(face_index))
                .map(|tf| (tf, actual_len, true))
        }
        FontFaceSource::Shared {
            data,
            face_index,
            is_mmap,
        } => {
            let sk_data = Data::new_with_owner(SharedFontBytes(data));
            let actual_len = sk_data.len();
            font_mgr
                .new_from_data(sk_data, Some(face_index))
                .map(|tf| (tf, actual_len, is_mmap))
        }
    });

    let Some((base_tf, raw_bytes_len, is_mmap)) = loaded else {
        // Clamp charge to `max_bytes()` so even tiny byte caps admit failed faces instead of
        // retrying decode on every frame.
        let failed_charge = FAILED_TYPEFACE_BYTES.min(base_typefaces.max_bytes());
        base_typefaces.insert(
            face_id,
            BaseTypefaceEntry {
                typeface: None,
                wght_range: None,
                variants: Vec::new(),
                backing: TypefaceBacking::Failed,
                raw_bytes: FAILED_TYPEFACE_BYTES,
            },
            failed_charge,
        );
        return None;
    };

    let wght_range = base_tf.variation_design_parameters().and_then(|params| {
        params
            .iter()
            .find(|p| p.tag == Coordinate::wght)
            .filter(|p| p.min.is_finite() && p.max.is_finite() && p.min < p.max)
            .map(|p| (p.min, p.max))
    });

    let (result_tf, variants) = if let Some((min_w, max_w)) = wght_range {
        let clamped = (weight as f32).clamp(min_w, max_w);
        let varied = derive_wght_variant(&base_tf, clamped);
        (varied.clone(), vec![(clamped.to_bits(), varied)])
    } else {
        (base_tf.clone(), Vec::new())
    };

    let raw_bytes = raw_bytes_len.max(1);
    // Mmap-backed faces share OS file pages (0 heap copy), so they charge `0` against the heap
    // byte cap while `MAX_BASE_TYPEFACES` bounds entry count and `raw_bytes` reports exact size.
    // Clamp heap face weight to `base_typefaces.max_bytes()` so a single oversized binary face
    // (> 32 MiB) is still admitted (evicting older heap entries) instead of re-parsing per frame.
    let (backing, heap_budget_bytes) = if is_mmap {
        (TypefaceBacking::Mmap, 0)
    } else {
        (
            TypefaceBacking::Heap,
            raw_bytes.min(base_typefaces.max_bytes()),
        )
    };
    base_typefaces.insert(
        face_id,
        BaseTypefaceEntry {
            typeface: Some(base_tf),
            wght_range,
            variants,
            backing,
            raw_bytes,
        },
        heap_budget_bytes,
    );
    Some(result_tf)
}

#[allow(clippy::too_many_arguments)]
fn assemble_text_blob(
    font_mgr: &FontMgr,
    text_engine: &CosmicTextEngine,
    base_typefaces: &mut SlabLru<cosmic_text::fontdb::ID, BaseTypefaceEntry, FastBuildHasher>,
    shaped_scratch: &mut SkiaShapedRuns,
    text: &str,
    params: TextLayoutParams,
    typeface_hits: &mut u64,
    typeface_misses: &mut u64,
) -> Option<TextBlob> {
    let has_fonts = text_engine.shape_for_skia_runs(
        text,
        params.font_size,
        params.font_weight,
        params.text_align,
        params.wrap_width,
        params.align_width,
        shaped_scratch,
    );
    if !has_fonts || shaped_scratch.glyphs.is_empty() {
        return None;
    }

    let mut builder = TextBlobBuilder::new();
    let glyphs = &shaped_scratch.glyphs;
    let mut i = 0usize;
    while i < glyphs.len() {
        let run_font_id = glyphs[i].font_id;
        let run_fs_bits = glyphs[i].font_size.to_bits();
        let mut j = i + 1;
        while j < glyphs.len()
            && glyphs[j].font_id == run_font_id
            && glyphs[j].font_size.to_bits() == run_fs_bits
        {
            j += 1;
        }
        let run_slice = &glyphs[i..j];
        let tf = resolve_typeface(
            font_mgr,
            text_engine,
            base_typefaces,
            run_font_id,
            params.font_weight,
            typeface_hits,
            typeface_misses,
        )?;
        let mut font = Font::from_typeface(tf, glyphs[i].font_size);
        font.set_subpixel(true);
        // Parity trade-off: unhinted grayscale AA (`Edging::AntiAlias` + `FontHinting::None`)
        // trades platform-specific LCD subpixel sharpening / grid-fitting for exact cross-OS
        // and Software-vs-Skia glyph metrics, ink centroids, and fractional-HiDPI parity.
        font.set_edging(Edging::AntiAlias);
        font.set_hinting(FontHinting::None);
        let (dst_glyphs, dst_pos) = builder.alloc_run_pos(&font, run_slice.len(), None);
        for (k, g) in run_slice.iter().enumerate() {
            dst_glyphs[k] = g.glyph_id;
            dst_pos[k] = Point::new(g.x, g.y);
        }
        i = j;
    }
    builder.make()
}

/// Builds a transient `TextBlob` without inserting into `text_blobs` or altering text-blob cache
/// telemetry (used for per-frame changing developer HUD banners).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_transient_text_blob(
    font_mgr: &FontMgr,
    text_engine: &CosmicTextEngine,
    base_typefaces: &mut SlabLru<cosmic_text::fontdb::ID, BaseTypefaceEntry, FastBuildHasher>,
    shaped_scratch: &mut SkiaShapedRuns,
    text: &str,
    font_size: f32,
    font_weight: u16,
    text_align: TextAlign,
    wrap_width: Option<f32>,
    align_width: f32,
    typeface_hits: &mut u64,
    typeface_misses: &mut u64,
) -> Option<TextBlob> {
    if text.is_empty() {
        return None;
    }
    let params = TextLayoutParams::new(font_size, font_weight, text_align, wrap_width, align_width);
    assemble_text_blob(
        font_mgr,
        text_engine,
        base_typefaces,
        shaped_scratch,
        text,
        params,
        typeface_hits,
        typeface_misses,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn get_or_build_text_blob(
    font_mgr: &FontMgr,
    text_engine: &CosmicTextEngine,
    base_typefaces: &mut SlabLru<cosmic_text::fontdb::ID, BaseTypefaceEntry, FastBuildHasher>,
    text_blobs: &mut SlabLru<TextBlobKey, CachedTextBlob, IdentityU64BuildHasher>,
    shaped_scratch: &mut SkiaShapedRuns,
    text: &str,
    font_size: f32,
    font_weight: u16,
    text_align: TextAlign,
    wrap_width: Option<f32>,
    align_width: f32,
    blob_hits: &mut u64,
    blob_misses: &mut u64,
    typeface_hits: &mut u64,
    typeface_misses: &mut u64,
) -> Option<TextBlob> {
    if text.is_empty() {
        return None;
    }
    let (key, params) =
        make_text_blob_key(text, font_size, font_weight, text_align, wrap_width, align_width);

    if let Some(entry) = text_blobs.get_mut(&key)
        && entry.text == text
    {
        *blob_hits = blob_hits.saturating_add(1);
        return entry.blob.clone();
    }
    *blob_misses = blob_misses.saturating_add(1);

    let blob = assemble_text_blob(
        font_mgr,
        text_engine,
        base_typefaces,
        shaped_scratch,
        text,
        params,
        typeface_hits,
        typeface_misses,
    );

    // Always cache the resulting entry (including `None` when any run's face failed) so broken
    // fonts never trigger per-frame HarfBuzz shaping or mmap retry storms.
    let est_bytes = 128 + text.len() + shaped_scratch.glyphs.len() * 16;
    text_blobs.insert(
        key,
        CachedTextBlob {
            text: text.to_owned(),
            blob: blob.clone(),
        },
        est_bytes,
    );
    blob
}
