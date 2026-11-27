//! Flat `FlatResource` implementations for [`ResponsiveRules`] and [`StateRules`] embedding
//! table-generic [`RowPatch`] payloads and propagating transitive child [`ResourceId`] edges.

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::{FlatResource, ResourceId, RowPatch, RowPatchView};

use crate::interaction::{InteractionState, StateStyleRule};
use crate::media::{
    MediaQuery, ResponsiveRule, MEDIA_FLAG_HAS_MAX_WIDTH, MEDIA_FLAG_HAS_MIN_WIDTH,
    MEDIA_FLAG_HAS_REDUCED_MOTION, MEDIA_FLAG_REDUCED_MOTION_VAL,
};

use super::MAX_ITEMS;

const VALID_MEDIA_FLAGS: u8 = MEDIA_FLAG_HAS_MIN_WIDTH
    | MEDIA_FLAG_HAS_MAX_WIDTH
    | MEDIA_FLAG_HAS_REDUCED_MOTION
    | MEDIA_FLAG_REDUCED_MOTION_VAL;

#[inline]
const fn pack_media_query(q: MediaQuery) -> (u32, u32) {
    let w0 = (q.min_width_px as u32) | ((q.max_width_px as u32) << 16);
    let w1 = (q.flags as u32) | ((q.color_scheme as u32) << 8) | ((q.pointer as u32) << 16);
    (w0, w1)
}

#[inline]
const fn unpack_media_query(w0: u32, w1: u32) -> MediaQuery {
    MediaQuery {
        min_width_px: (w0 & 0xFFFF) as u16,
        max_width_px: (w0 >> 16) as u16,
        flags: (w1 & 0xFF) as u8,
        color_scheme: ((w1 >> 8) & 0xFF) as u8,
        pointer: ((w1 >> 16) & 0xFF) as u8,
        _pad: ((w1 >> 24) & 0xFF) as u8,
    }
}

// ---------------------------------------------------------------------------------------------
// ResponsiveRules: [count, (mq_w0, mq_w1, patch_words_len, patch_words...)*]
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ResponsiveRules(pub Arc<[ResponsiveRule]>);

impl ResponsiveRules {
    pub fn new(rules: impl Into<Arc<[ResponsiveRule]>>) -> Self {
        Self(rules.into())
    }
}

/// Zero-copy borrowed view over flat [`ResponsiveRules`] words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponsiveRulesView<'a>(&'a [u32]);

impl<'a> ResponsiveRulesView<'a> {
    pub const EMPTY: Self = Self(&[0]);

    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() { 0 } else { self.0[0] as usize }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn iter(self) -> ResponsiveRulesIter<'a> {
        ResponsiveRulesIter {
            words: if self.0.len() > 1 { &self.0[1..] } else { &[] },
            pos: 0,
            remaining: self.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResponsiveRulesIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for ResponsiveRulesIter<'a> {
    type Item = (MediaQuery, RowPatchView<'a>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let w0 = *self.words.get(self.pos)?;
        let w1 = *self.words.get(self.pos + 1)?;
        let plen = *self.words.get(self.pos + 2)? as usize;
        let start = self.pos + 3;
        let end = start.checked_add(plen)?;
        let patch_words = self.words.get(start..end)?;
        self.pos = end;
        self.remaining -= 1;
        Some((
            unpack_media_query(w0, w1),
            RowPatchView::from_validated_words(patch_words),
        ))
    }
}

impl FlatResource for ResponsiveRules {
    const KIND: Key64 = Key64::of("nve.res.responsive_rules");
    type View<'a> = ResponsiveRulesView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_ITEMS {
            return false;
        }
        let start = out.len();
        out.push(self.0.len() as u32);
        for rule in self.0.iter() {
            let (w0, w1) = pack_media_query(rule.query);
            out.push(w0);
            out.push(w1);
            let len_idx = out.len();
            out.push(0);
            let patch_start = out.len();
            if !rule.patch.write_words(out) {
                out.truncate(start);
                return false;
            }
            let plen = out.len() - patch_start;
            let Ok(plen_u32) = u32::try_from(plen) else {
                out.truncate(start);
                return false;
            };
            out[len_idx] = plen_u32;
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
        let mut pos = 1usize;
        for _ in 0..count {
            let Some(&w0) = words.get(pos) else {
                return false;
            };
            let Some(&w1) = words.get(pos + 1) else {
                return false;
            };
            let Some(&plen_u32) = words.get(pos + 2) else {
                return false;
            };
            let q = unpack_media_query(w0, w1);
            if q._pad != 0
                || (q.flags & !VALID_MEDIA_FLAGS) != 0
                || q.color_scheme > 2
                || q.pointer > 2
            {
                return false;
            }
            pos += 3;
            let Some(end) = pos.checked_add(plen_u32 as usize) else {
                return false;
            };
            let Some(patch_words) = words.get(pos..end) else {
                return false;
            };
            if !RowPatch::validate(patch_words) {
                return false;
            }
            pos = end;
        }
        pos == words.len()
    }

    #[inline(always)]
    fn view(words: &[u32]) -> ResponsiveRulesView<'_> {
        ResponsiveRulesView(words)
    }

    fn child_resources(words: &[u32], visit: &mut dyn FnMut(ResourceId)) {
        for (_, patch) in ResponsiveRulesView(words).iter() {
            patch.visit_resources(visit);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// StateRules: [count, (state_bits_u32, patch_words_len, patch_words...)*]
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct StateRules(pub Arc<[StateStyleRule]>);

impl StateRules {
    pub fn new(rules: impl Into<Arc<[StateStyleRule]>>) -> Self {
        Self(rules.into())
    }
}

/// Zero-copy borrowed view over flat [`StateRules`] words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRulesView<'a>(&'a [u32]);

impl<'a> StateRulesView<'a> {
    pub const EMPTY: Self = Self(&[0]);

    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() { 0 } else { self.0[0] as usize }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn iter(self) -> StateRulesIter<'a> {
        StateRulesIter {
            words: if self.0.len() > 1 { &self.0[1..] } else { &[] },
            pos: 0,
            remaining: self.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StateRulesIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for StateRulesIter<'a> {
    type Item = (InteractionState, RowPatchView<'a>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let state_bits = *self.words.get(self.pos)?;
        let plen = *self.words.get(self.pos + 1)? as usize;
        let start = self.pos + 2;
        let end = start.checked_add(plen)?;
        let patch_words = self.words.get(start..end)?;
        self.pos = end;
        self.remaining -= 1;
        Some((
            InteractionState::from_bits_truncate(state_bits as u8),
            RowPatchView::from_validated_words(patch_words),
        ))
    }
}

impl FlatResource for StateRules {
    const KIND: Key64 = Key64::of("nve.res.state_rules");
    type View<'a> = StateRulesView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_ITEMS {
            return false;
        }
        let start = out.len();
        out.push(self.0.len() as u32);
        for rule in self.0.iter() {
            out.push(u32::from(rule.state.bits()));
            let len_idx = out.len();
            out.push(0);
            let patch_start = out.len();
            if !rule.patch.write_words(out) {
                out.truncate(start);
                return false;
            }
            let plen = out.len() - patch_start;
            let Ok(plen_u32) = u32::try_from(plen) else {
                out.truncate(start);
                return false;
            };
            out[len_idx] = plen_u32;
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
        let max_bits = u32::from(InteractionState::ALL.bits());
        let mut pos = 1usize;
        for _ in 0..count {
            let Some(&state_bits) = words.get(pos) else {
                return false;
            };
            let Some(&plen_u32) = words.get(pos + 1) else {
                return false;
            };
            if state_bits > max_bits {
                return false;
            }
            pos += 2;
            let Some(end) = pos.checked_add(plen_u32 as usize) else {
                return false;
            };
            let Some(patch_words) = words.get(pos..end) else {
                return false;
            };
            if !RowPatch::validate(patch_words) {
                return false;
            }
            pos = end;
        }
        pos == words.len()
    }

    #[inline(always)]
    fn view(words: &[u32]) -> StateRulesView<'_> {
        StateRulesView(words)
    }

    fn child_resources(words: &[u32], visit: &mut dyn FnMut(ResourceId)) {
        for (_, patch) in StateRulesView(words).iter() {
            patch.visit_resources(visit);
        }
    }
}
