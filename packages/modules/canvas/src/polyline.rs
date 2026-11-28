//! Zero-copy borrowed polyline vertex view supporting both sealed flat word streams
//! (`DrawListView`) and direct in-memory caller slices (`&[(f32, f32)]`).

/// Private backing store for [`PolylinePoints`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum PolylineBacking<'a> {
    /// Sealed-valid flat words from `DrawListView` (already proven finite and
    /// `<= MAX_POLYLINE_POINTS` at `ValidatedWords` ingress).
    Words(&'a [u32]),
    /// Direct in-memory caller slice (`&[(f32, f32)]`).
    Slice(&'a [(f32, f32)]),
}

/// Zero-copy borrowed view over `(x, y)` `f32` vertex pairs for `StrokePolyline`.
///
/// Allows [`crate::CanvasPainter`] to stay strictly object-safe (`dyn CanvasPainter`) while
/// replaying flat [`crate::DrawListView`] word streams and direct caller slices with zero heap
/// allocations.
#[derive(Debug, Clone, Copy)]
pub struct PolylinePoints<'a>(PolylineBacking<'a>);

impl<'a> PolylinePoints<'a> {
    /// Wraps a direct in-memory `(x, y)` vertex slice with zero copies.
    #[inline(always)]
    pub const fn from_slice(points: &'a [(f32, f32)]) -> Self {
        Self(PolylineBacking::Slice(points))
    }

    /// Wraps a sealed-valid flat `u32` word slice (`[x0_bits, y0_bits, ...]`) from `DrawListView`.
    #[inline(always)]
    pub(crate) const fn from_words(words: &'a [u32]) -> Self {
        Self(PolylineBacking::Words(words))
    }

    #[inline(always)]
    pub(crate) const fn backing(self) -> PolylineBacking<'a> {
        self.0
    }

    /// Returns `true` when backed by sealed-valid `ValidatedWords` from `DrawListView`.
    #[inline(always)]
    pub const fn is_validated_words(self) -> bool {
        matches!(self.0, PolylineBacking::Words(_))
    }

    #[inline(always)]
    pub const fn len(self) -> usize {
        match self.0 {
            PolylineBacking::Words(w) => w.len() / 2,
            PolylineBacking::Slice(s) => s.len(),
        }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Returns a prefix view containing at most `max_points` vertices.
    #[inline]
    pub fn truncate(self, max_points: usize) -> Self {
        match self.0 {
            PolylineBacking::Words(w) => {
                let max_words = max_points.saturating_mul(2).min(w.len() & !1);
                Self(PolylineBacking::Words(&w[..max_words]))
            }
            PolylineBacking::Slice(s) => {
                Self(PolylineBacking::Slice(&s[..s.len().min(max_points)]))
            }
        }
    }

    #[inline]
    pub fn get(self, index: usize) -> Option<(f32, f32)> {
        match self.0 {
            PolylineBacking::Words(w) => {
                let b = index.checked_mul(2)?;
                let x = *w.get(b)?;
                let y = *w.get(b + 1)?;
                Some((f32::from_bits(x), f32::from_bits(y)))
            }
            PolylineBacking::Slice(s) => s.get(index).copied(),
        }
    }

    #[inline]
    pub fn iter(self) -> PolylinePointsIter<'a> {
        let inner = match self.0 {
            PolylineBacking::Words(w) => PolylineIterInner::Words(w.as_chunks::<2>().0.iter()),
            PolylineBacking::Slice(s) => PolylineIterInner::Slice(s.iter()),
        };
        PolylinePointsIter { inner }
    }
}

impl PartialEq for PolylinePoints<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self.0, other.0) {
            (PolylineBacking::Words(a), PolylineBacking::Words(b)) => {
                a[..(a.len() & !1)] == b[..(b.len() & !1)]
            }
            _ => {
                self.len() == other.len()
                    && self
                        .iter()
                        .zip(other.iter())
                        .all(|((ax, ay), (bx, by))| {
                            ax.to_bits() == bx.to_bits() && ay.to_bits() == by.to_bits()
                        })
            }
        }
    }
}

impl Eq for PolylinePoints<'_> {}

#[derive(Debug, Clone)]
enum PolylineIterInner<'a> {
    Words(core::slice::Iter<'a, [u32; 2]>),
    Slice(core::slice::Iter<'a, (f32, f32)>),
}

/// Zero-allocation iterator over `(x, y)` `f32` vertex pairs in a [`PolylinePoints`] view.
#[derive(Debug, Clone)]
pub struct PolylinePointsIter<'a> {
    inner: PolylineIterInner<'a>,
}

impl Iterator for PolylinePointsIter<'_> {
    type Item = (f32, f32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            PolylineIterInner::Words(iter) => {
                let &[x, y] = iter.next()?;
                Some((f32::from_bits(x), f32::from_bits(y)))
            }
            PolylineIterInner::Slice(iter) => iter.next().copied(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.inner {
            PolylineIterInner::Words(iter) => iter.size_hint(),
            PolylineIterInner::Slice(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for PolylinePointsIter<'_> {}
