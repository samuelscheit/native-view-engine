//! Flat-buffer `nve_store::FlatResource` blobs referenced by core word tables.
//!
//! Every resource lives directly in `ResourceStore`'s contiguous `SpanPool<u32>` slab with
//! zero-copy `View<'a>` handles, zero `Arc<dyn Any>`, zero `downcast_ref`, and strict adversarial
//! validation at wire ingress.

mod grid_tracks;
mod paint_res;
mod rules_res;

pub use grid_tracks::{GridTracks, GridTracksIter, GridTracksView};
pub use paint_res::{
    BackgroundFillRes, BackgroundFillView, BoxShadows, BoxShadowsIter, BoxShadowsView,
    GradientStopsIter, GradientStopsView, Transitions, TransitionsIter, TransitionsView,
};
pub use rules_res::{
    ResponsiveRules, ResponsiveRulesIter, ResponsiveRulesView, StateRules, StateRulesIter,
    StateRulesView,
};

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::{
    read_bytes_words, read_fixed_str_pack, read_str_words, validate_bytes_words,
    validate_fixed_str_pack, validate_str_words, write_bytes_words, write_fixed_str_pack,
    write_str_words, FlatResource,
};

pub(super) const MAX_ITEMS: usize = 4096;
const MAX_STRING_BYTES: usize = 256 * 1024;
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

// ---------------------------------------------------------------------------------------------
// AriaRelations
// ---------------------------------------------------------------------------------------------

/// ARIA ID-reference relations (`aria-controls`, `aria-labelledby`, `aria-describedby`,
/// `aria-activedescendant`). Empty strings are omitted on emission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AriaRelations {
    pub controls: String,
    pub labelled_by: String,
    pub described_by: String,
    pub active_descendant: String,
}

/// Zero-copy borrowed view over flat [`AriaRelations`] words (`[len0..len3, utf8_bytes...]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AriaRelationsView<'a>(&'a [u32]);

impl<'a> AriaRelationsView<'a> {
    #[inline]
    pub fn controls(self) -> &'a str {
        read_fixed_str_pack::<4>(self.0, 0)
    }

    #[inline]
    pub fn labelled_by(self) -> &'a str {
        read_fixed_str_pack::<4>(self.0, 1)
    }

    #[inline]
    pub fn described_by(self) -> &'a str {
        read_fixed_str_pack::<4>(self.0, 2)
    }

    #[inline]
    pub fn active_descendant(self) -> &'a str {
        read_fixed_str_pack::<4>(self.0, 3)
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.controls().is_empty()
            && self.labelled_by().is_empty()
            && self.described_by().is_empty()
            && self.active_descendant().is_empty()
    }

    pub fn to_owned(self) -> AriaRelations {
        AriaRelations {
            controls: self.controls().to_owned(),
            labelled_by: self.labelled_by().to_owned(),
            described_by: self.described_by().to_owned(),
            active_descendant: self.active_descendant().to_owned(),
        }
    }
}

impl FlatResource for AriaRelations {
    const KIND: Key64 = Key64::of("nve.res.aria_relations");
    type View<'a> = AriaRelationsView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        write_fixed_str_pack(
            out,
            [
                self.controls.as_str(),
                self.labelled_by.as_str(),
                self.described_by.as_str(),
                self.active_descendant.as_str(),
            ],
            MAX_STRING_BYTES,
        )
    }

    fn validate(words: &[u32]) -> bool {
        validate_fixed_str_pack::<4>(words, MAX_STRING_BYTES)
    }

    #[inline(always)]
    fn view(words: &[u32]) -> AriaRelationsView<'_> {
        AriaRelationsView(words)
    }
}

// ---------------------------------------------------------------------------------------------
// ImageSrc
// ---------------------------------------------------------------------------------------------

/// Image source payload: either a URL/URI string or inline encoded image bytes (PNG/WebP/JPEG).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSrc {
    Url(String),
    EncodedBytes(Arc<[u8]>),
}

/// Zero-copy borrowed view over flat [`ImageSrc`] words (`[tag, byte_len, payload_words...]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSrcView<'a> {
    Url(&'a str),
    EncodedBytes(&'a [u8]),
}

impl ImageSrcView<'_> {
    pub fn to_owned(self) -> ImageSrc {
        match self {
            Self::Url(u) => ImageSrc::Url(u.to_owned()),
            Self::EncodedBytes(b) => ImageSrc::EncodedBytes(Arc::from(b)),
        }
    }
}

impl FlatResource for ImageSrc {
    const KIND: Key64 = Key64::of("nve.res.image_src");
    type View<'a> = ImageSrcView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        let start = out.len();
        let ok = match self {
            Self::Url(u) => {
                out.push(0);
                write_str_words(out, u, MAX_STRING_BYTES)
            }
            Self::EncodedBytes(b) => {
                out.push(1);
                write_bytes_words(out, b, MAX_IMAGE_BYTES)
            }
        };
        if !ok {
            out.truncate(start);
        }
        ok
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&tag) = words.first() else {
            return false;
        };
        let mut pos = 1usize;
        let ok = match tag {
            0 => validate_str_words(words, &mut pos, MAX_STRING_BYTES),
            1 => validate_bytes_words(words, &mut pos, MAX_IMAGE_BYTES),
            _ => false,
        };
        ok && pos == words.len()
    }

    #[inline]
    fn view(words: &[u32]) -> ImageSrcView<'_> {
        let mut pos = 1usize;
        match words.first().copied() {
            Some(0) => ImageSrcView::Url(read_str_words(words, &mut pos).unwrap_or("")),
            Some(1) => ImageSrcView::EncodedBytes(read_bytes_words(words, &mut pos).unwrap_or(&[])),
            _ => ImageSrcView::Url(""),
        }
    }
}
