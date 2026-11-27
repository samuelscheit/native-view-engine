//! Flat 3-word-stride [`GridTracks`] `FlatResource` and zero-copy [`GridTracksView`] with
//! `O(1)` length, `O(1)` indexing, and `ExactSizeIterator` for Taffy's zero-alloc `StyleView`.

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::FlatResource;

use crate::layout_ext::{GridSingleTrack, GridTrack, GridTrackUnit};

use super::MAX_ITEMS;

const TRACK_STRIDE: usize = 3;

#[inline]
fn pack_unit(u: GridTrackUnit) -> (u32, u32) {
    match u {
        GridTrackUnit::Auto => (0, 0),
        GridTrackUnit::Px(v) => (1, v.to_bits()),
        GridTrackUnit::Percent(v) => (2, v.to_bits()),
        GridTrackUnit::Fr(v) => (3, v.to_bits()),
        GridTrackUnit::MinContent => (4, 0),
        GridTrackUnit::MaxContent => (5, 0),
    }
}

#[inline]
fn unpack_unit(tag: u32, w: u32) -> GridTrackUnit {
    match tag {
        1 => GridTrackUnit::Px(f32::from_bits(w)),
        2 => GridTrackUnit::Percent(f32::from_bits(w)),
        3 => GridTrackUnit::Fr(f32::from_bits(w)),
        4 => GridTrackUnit::MinContent,
        5 => GridTrackUnit::MaxContent,
        _ => GridTrackUnit::Auto,
    }
}

#[inline]
const fn valid_unit(tag: u32, w: u32) -> bool {
    match tag {
        0 | 4 | 5 => w == 0,
        1..=3 => true,
        _ => false,
    }
}

#[inline]
fn pack_track(t: GridTrack) -> [u32; 3] {
    match t {
        GridTrack::Auto => [0, 0, 0],
        GridTrack::Px(v) => [1, v.to_bits(), 0],
        GridTrack::Percent(v) => [2, v.to_bits(), 0],
        GridTrack::Fr(v) => [3, v.to_bits(), 0],
        GridTrack::MinMax(min, max) => {
            let (t1, w1) = pack_unit(min);
            let (t2, w2) = pack_unit(max);
            [4 | (t1 << 7) | (t2 << 10), w1, w2]
        }
        GridTrack::Repeat(count, single) => {
            let rep = u32::from(count) << 16;
            match single {
                GridSingleTrack::Auto => [5 | rep, 0, 0],
                GridSingleTrack::Px(v) => [5 | (1 << 4) | rep, v.to_bits(), 0],
                GridSingleTrack::Percent(v) => [5 | (2 << 4) | rep, v.to_bits(), 0],
                GridSingleTrack::Fr(v) => [5 | (3 << 4) | rep, v.to_bits(), 0],
                GridSingleTrack::MinMax(min, max) => {
                    let (t1, w1) = pack_unit(min);
                    let (t2, w2) = pack_unit(max);
                    [5 | (4 << 4) | (t1 << 7) | (t2 << 10) | rep, w1, w2]
                }
            }
        }
    }
}

#[inline]
fn unpack_track(hdr: u32, w1: u32, w2: u32) -> GridTrack {
    let tag = hdr & 0xF;
    let single_tag = (hdr >> 4) & 0x7;
    let u1 = (hdr >> 7) & 0x7;
    let u2 = (hdr >> 10) & 0x7;
    let rep = (hdr >> 16) as u16;
    match tag {
        1 => GridTrack::Px(f32::from_bits(w1)),
        2 => GridTrack::Percent(f32::from_bits(w1)),
        3 => GridTrack::Fr(f32::from_bits(w1)),
        4 => GridTrack::MinMax(unpack_unit(u1, w1), unpack_unit(u2, w2)),
        5 => {
            let single = match single_tag {
                1 => GridSingleTrack::Px(f32::from_bits(w1)),
                2 => GridSingleTrack::Percent(f32::from_bits(w1)),
                3 => GridSingleTrack::Fr(f32::from_bits(w1)),
                4 => GridSingleTrack::MinMax(unpack_unit(u1, w1), unpack_unit(u2, w2)),
                _ => GridSingleTrack::Auto,
            };
            GridTrack::Repeat(rep, single)
        }
        _ => GridTrack::Auto,
    }
}

#[inline]
const fn validate_track(hdr: u32, w1: u32, w2: u32) -> bool {
    if (hdr & 0xE000) != 0 {
        return false;
    }
    let tag = hdr & 0xF;
    let single_tag = (hdr >> 4) & 0x7;
    let u1 = (hdr >> 7) & 0x7;
    let u2 = (hdr >> 10) & 0x7;
    let rep = hdr >> 16;
    match tag {
        0 => single_tag == 0 && u1 == 0 && u2 == 0 && rep == 0 && w1 == 0 && w2 == 0,
        1..=3 => single_tag == 0 && u1 == 0 && u2 == 0 && rep == 0 && w2 == 0,
        4 => single_tag == 0 && rep == 0 && valid_unit(u1, w1) && valid_unit(u2, w2),
        5 => match single_tag {
            0 => u1 == 0 && u2 == 0 && w1 == 0 && w2 == 0,
            1..=3 => u1 == 0 && u2 == 0 && w2 == 0,
            4 => valid_unit(u1, w1) && valid_unit(u2, w2),
            _ => false,
        },
        _ => false,
    }
}

/// Owned CSS Grid track template list (`FlatResource` backed by 3-word fixed-stride slab).
#[derive(Debug, Clone, PartialEq)]
pub struct GridTracks(pub Arc<[GridTrack]>);

impl GridTracks {
    pub fn new(tracks: impl Into<Arc<[GridTrack]>>) -> Self {
        Self(tracks.into())
    }
}

/// Zero-copy borrowed view over flat [`GridTracks`] words (`[count, (hdr, w1, w2)*]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridTracksView<'a>(&'a [u32]);

impl<'a> GridTracksView<'a> {
    pub const EMPTY: Self = Self(&[0]);

    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            let max_fit = (self.0.len() - 1) / TRACK_STRIDE;
            let count = self.0[0] as usize;
            if count < max_fit { count } else { max_fit }
        }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(self, index: usize) -> Option<GridTrack> {
        if index >= self.len() {
            return None;
        }
        let base = 1 + index * TRACK_STRIDE;
        Some(unpack_track(self.0[base], self.0[base + 1], self.0[base + 2]))
    }

    #[inline]
    pub fn iter(self) -> GridTracksIter<'a> {
        GridTracksIter {
            view: self,
            idx: 0,
            end: self.len(),
        }
    }

    /// Formats this track list as a space-separated CSS `grid-template-*` value directly into `out`.
    pub fn write_css_into(self, out: &mut String) {
        for (i, track) in self.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            track.write_css_into(out);
        }
    }
}

impl<'a> IntoIterator for GridTracksView<'a> {
    type Item = GridTrack;
    type IntoIter = GridTracksIter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Zero-alloc `ExactSizeIterator` over [`GridTrack`]s in a [`GridTracksView`].
#[derive(Debug, Clone)]
pub struct GridTracksIter<'a> {
    view: GridTracksView<'a>,
    idx: usize,
    end: usize,
}

impl Iterator for GridTracksIter<'_> {
    type Item = GridTrack;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.end {
            return None;
        }
        let item = self.view.get(self.idx);
        self.idx += 1;
        item
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.end - self.idx;
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for GridTracksIter<'_> {}

impl FlatResource for GridTracks {
    const KIND: Key64 = Key64::of("nve.res.grid_tracks");
    type View<'a> = GridTracksView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_ITEMS {
            return false;
        }
        out.reserve(1 + self.0.len() * TRACK_STRIDE);
        out.push(self.0.len() as u32);
        for &track in self.0.iter() {
            out.extend_from_slice(&pack_track(track));
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&count_u32) = words.first() else {
            return false;
        };
        let count = count_u32 as usize;
        if count > MAX_ITEMS {
            return false;
        }
        let Some(expected) = count.checked_mul(TRACK_STRIDE).and_then(|n| n.checked_add(1)) else {
            return false;
        };
        if words.len() != expected {
            return false;
        }
        for chunk in words[1..].as_chunks::<TRACK_STRIDE>().0 {
            if !validate_track(chunk[0], chunk[1], chunk[2]) {
                return false;
            }
        }
        true
    }

    #[inline(always)]
    fn view(words: &[u32]) -> GridTracksView<'_> {
        GridTracksView(words)
    }
}
