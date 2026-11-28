//! Declarative transitions: [`TransitionProperty`], [`TransitionSpec`], the flat
//! [`Transitions`] resource (7 words/spec) and the [`Motion`] (`nve.motion`) word table.

use std::sync::Arc;

use nve_abi::Key64;
use nve_store::{FlatResource, NodeId, NveTable, Res, TxCore, ValidatedWords, WordEnum};

use crate::easing::EasingCurve;
use crate::spring::SpringSpec;

/// Strictly typed animatable style property (`#[repr(u8)]`, zero arbitrary strings).
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, WordEnum)]
pub enum TransitionProperty {
    #[default]
    All = 0,
    BackgroundColor = 1,
    Color = 2,
    BorderColor = 3,
    Opacity = 4,
    BorderRadius = 5,
    Transform = 6,
    Width = 7,
    Height = 8,
    Padding = 9,
    Gap = 10,
}

impl TransitionProperty {
    /// Returns `true` when `self` governs `target` (`All` matches every property).
    #[inline]
    pub const fn matches(self, target: Self) -> bool {
        matches!(self, Self::All) || (self as u8) == (target as u8)
    }
}

/// Declarative style transition: compiled to native CSS `transition` on Web and driven by
/// [`crate::MotionClock`]-style interpolation on native backends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionSpec {
    pub property: TransitionProperty,
    pub duration_ms: u32,
    pub delay_ms: u32,
    pub easing: EasingCurve,
    pub reduced_motion_instant: bool,
}

impl Default for TransitionSpec {
    #[inline]
    fn default() -> Self {
        Self::all(180)
    }
}

impl TransitionSpec {
    /// Transition for `property` lasting `duration_ms` with `EaseOut`, snapping under reduced
    /// motion.
    #[inline]
    pub const fn new(property: TransitionProperty, duration_ms: u32) -> Self {
        Self {
            property,
            duration_ms,
            delay_ms: 0,
            easing: EasingCurve::EaseOut,
            reduced_motion_instant: true,
        }
    }

    /// Shorthand for `TransitionSpec::new(TransitionProperty::All, duration_ms)`.
    #[inline]
    pub const fn all(duration_ms: u32) -> Self {
        Self::new(TransitionProperty::All, duration_ms)
    }

    /// Spring-driven transition whose duration is the spring's settle time.
    pub fn spring(property: TransitionProperty, spring: SpringSpec) -> Self {
        let duration_ms = (spring.settle_duration_s() * 1000.0).round() as u32;
        Self::new(property, duration_ms).easing(EasingCurve::Spring(spring))
    }

    #[inline]
    pub const fn delay_ms(mut self, delay_ms: u32) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    #[inline]
    pub const fn easing(mut self, easing: EasingCurve) -> Self {
        self.easing = easing;
        self
    }

    /// Controls whether reduced motion snaps this transition instantly (`true` by default).
    #[inline]
    pub const fn reduced_motion_instant(mut self, instant: bool) -> Self {
        self.reduced_motion_instant = instant;
        self
    }

    #[inline]
    pub fn duration_s(&self) -> f32 {
        self.duration_ms as f32 / 1000.0
    }

    #[inline]
    pub fn delay_s(&self) -> f32 {
        self.delay_ms as f32 / 1000.0
    }
}

/// Resolves the winning [`TransitionSpec`] for `prop` using CSS last-match-wins order.
pub fn resolve_transition_for(
    specs: impl DoubleEndedIterator<Item = TransitionSpec>,
    prop: TransitionProperty,
) -> Option<TransitionSpec> {
    specs.rev().find(|s| s.property.matches(prop))
}

/// Attaches `specs` as `node`'s [`Motion`] transitions, or detaches [`Motion`] when empty
/// (e.g. from Dioxus: `nve_table: table_attr::<Motion, _>(specs, |tx, n, s| write_transitions(tx, n, s))`).
pub fn write_transitions(tx: &mut TxCore<'_>, node: NodeId, specs: &[TransitionSpec]) {
    if specs.is_empty() {
        tx.detach::<Motion>(node);
    } else {
        tx.edit::<Motion>(node)
            .transitions(Some(Transitions::new(specs)));
    }
}

/// Declarative transitions attached to a node.
#[derive(NveTable)]
#[nve_table(name = "nve.motion", storage = "sparse", flags = "publish")]
pub struct Motion {
    #[effect(composite, paint)]
    pub transitions: Res<Transitions>,
}

const TRANSITION_STRIDE: usize = 7;
const MAX_TRANSITIONS: usize = 4096;
const EASING_CUBIC: u32 = 5;
const EASING_SPRING: u32 = 6;
const PRESET_EASINGS: [EasingCurve; 5] = [
    EasingCurve::Linear,
    EasingCurve::Ease,
    EasingCurve::EaseIn,
    EasingCurve::EaseOut,
    EasingCurve::EaseInOut,
];

/// Owned transition list (`nve.res.transitions`).
#[derive(Debug, Clone, PartialEq)]
pub struct Transitions(pub Arc<[TransitionSpec]>);

impl Transitions {
    pub fn new(specs: impl Into<Arc<[TransitionSpec]>>) -> Self {
        Self(specs.into())
    }
}

/// Zero-copy view over validated [`Transitions`] words: `[count, (hdr, duration_ms, delay_ms,
/// c0..c3)*]` with `hdr = property | easing_tag << 8 | reduced_motion_instant << 16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionsView<'a>(&'a [u32]);

impl<'a> TransitionsView<'a> {
    #[inline(always)]
    pub fn len(self) -> usize {
        self.0.len().saturating_sub(1) / TRANSITION_STRIDE
    }

    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Validated flat words (a content key for caches).
    #[inline(always)]
    pub fn words(self) -> &'a [u32] {
        self.0
    }

    pub fn get(self, index: usize) -> Option<TransitionSpec> {
        let b = 1 + index.checked_mul(TRANSITION_STRIDE)?;
        let w = self.0.get(b..b + TRANSITION_STRIDE)?;
        let hdr = w[0];
        let c = |i: usize| f32::from_bits(w[3 + i]);
        let tag = (hdr >> 8) & 0xFF;
        let easing = if let Some(&preset) = PRESET_EASINGS.get(tag as usize) {
            preset
        } else if tag == EASING_CUBIC {
            EasingCurve::CubicBezier(c(0), c(1), c(2), c(3))
        } else if tag == EASING_SPRING {
            EasingCurve::Spring(SpringSpec::new(c(0), c(1), c(2)))
        } else {
            return None;
        };
        Some(TransitionSpec {
            property: TransitionProperty::try_from_word(u32::from(hdr as u8))?,
            duration_ms: w[1],
            delay_ms: w[2],
            easing,
            reduced_motion_instant: ((hdr >> 16) & 0xFF) != 0,
        })
    }

    #[inline]
    pub fn iter(self) -> impl DoubleEndedIterator<Item = TransitionSpec> + Clone + 'a {
        (0..self.len()).filter_map(move |i| self.get(i))
    }
}

impl FlatResource for Transitions {
    const KIND: Key64 = Key64::of("nve.res.transitions");
    type View<'a> = TransitionsView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_TRANSITIONS {
            return false;
        }
        let start = out.len();
        out.reserve(1 + self.0.len() * TRANSITION_STRIDE);
        out.push(self.0.len() as u32);
        for t in self.0.iter() {
            let (tag, c) = match t.easing.sanitized() {
                EasingCurve::Linear => (0, [0.0; 4]),
                EasingCurve::Ease => (1, [0.0; 4]),
                EasingCurve::EaseIn => (2, [0.0; 4]),
                EasingCurve::EaseOut => (3, [0.0; 4]),
                EasingCurve::EaseInOut => (4, [0.0; 4]),
                EasingCurve::CubicBezier(x1, y1, x2, y2) => (EASING_CUBIC, [x1, y1, x2, y2]),
                EasingCurve::Spring(s) => (EASING_SPRING, [s.stiffness, s.damping, s.mass, 0.0]),
            };
            let hdr = u32::from(t.property as u8)
                | (tag << 8)
                | (u32::from(t.reduced_motion_instant) << 16);
            out.extend_from_slice(&[hdr, t.duration_ms, t.delay_ms]);
            out.extend(c.map(f32::to_bits));
        }
        if !Self::validate(&out[start..]) {
            out.truncate(start);
            return false;
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&count) = words.first() else {
            return false;
        };
        let count = count as usize;
        if count > MAX_TRANSITIONS || words.len() != 1 + count * TRANSITION_STRIDE {
            return false;
        }
        words[1..].as_chunks::<TRANSITION_STRIDE>().0.iter().all(|w| {
            let hdr = w[0];
            let c = |i: usize| f32::from_bits(w[3 + i]);
            let params_ok = match (hdr >> 8) & 0xFF {
                0..=4 => w[3..].iter().all(|&x| x == 0),
                EASING_CUBIC => {
                    (0..4).all(|i| c(i).is_finite())
                        && (0.0..=1.0).contains(&c(0))
                        && (0.0..=1.0).contains(&c(2))
                }
                // Positive finite stiffness, damping (an undamped spring never settles), mass.
                EASING_SPRING => (0..3).all(|i| c(i).is_finite() && c(i) > 0.0) && w[6] == 0,
                _ => false,
            };
            params_ok
                && TransitionProperty::try_from_word(u32::from(hdr as u8)).is_some()
                && ((hdr >> 16) & 0xFF) <= 1
                && (hdr >> 24) == 0
        })
    }

    #[inline(always)]
    fn view(words: ValidatedWords<'_, Self>) -> TransitionsView<'_> {
        TransitionsView(words.as_slice())
    }
}
