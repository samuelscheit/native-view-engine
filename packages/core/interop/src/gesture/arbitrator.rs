//! Cross-framework touch and nested-scroll arbitration engine (`GestureArbitrator`).

use nve_abi::SurfaceId;

use super::types::{
    ArbitrationDecision, ArbitrationReason, FastU32Map, GestureCapabilities, GestureDisposition,
    PointerPhase, PointerSample, ScrollEdgeState, SurfaceChain,
};

#[derive(Debug, Clone)]
struct ActivePointer {
    chain: SurfaceChain,
    down_x: f32,
    down_y: f32,
    winner: Option<SurfaceId>,
    locked_scroll: bool,
}

/// Cross-framework touch and nested-scroll arbitration engine.
///
/// Solves inner-vs-outer scroll conflicts across framework boundaries (e.g. an NVE vertical
/// `ScrollView` embedded inside a Flutter `ListView`, or a React Native horizontal carousel
/// inside a SwiftUI vertical `ScrollView`).
#[derive(Debug, Default)]
pub struct GestureArbitrator {
    capabilities: FastU32Map<GestureCapabilities, 16>,
    overrides: FastU32Map<GestureDisposition, 8>,
    active_pointers: FastU32Map<ActivePointer, 8>,
}

impl GestureArbitrator {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn set_capabilities(&mut self, surface_id: SurfaceId, caps: GestureCapabilities) {
        self.capabilities.insert(surface_id, caps);
    }

    #[inline]
    pub fn update_scroll_edges(&mut self, surface_id: SurfaceId, edges: ScrollEdgeState) {
        if let Some(caps) = self.capabilities.get_mut(surface_id) {
            caps.scroll_edges = edges;
        }
    }

    /// Forces an explicit disposition for a surface (e.g. from a custom native hit-test hook).
    #[inline]
    pub fn set_override(&mut self, surface_id: SurfaceId, disposition: Option<GestureDisposition>) {
        match disposition {
            Some(d) => {
                self.overrides.insert(surface_id, d);
            }
            None => {
                self.overrides.remove(surface_id);
            }
        }
    }

    /// Removes all gesture state for a destroyed surface.
    pub fn unregister_surface(&mut self, surface_id: SurfaceId) {
        self.capabilities.remove(surface_id);
        self.overrides.remove(surface_id);
        self.active_pointers.retain(|_, track| {
            track.chain.retain(|id| id != surface_id);
            if track.winner == Some(surface_id) {
                track.winner = None;
                track.locked_scroll = false;
            }
            !track.chain.is_empty()
        });
    }

    #[inline]
    pub fn active_pointer_count(&self) -> usize {
        self.active_pointers.len()
    }

    /// Arbitrates a pointer sample along `hit_chain` (ordered **innermost -> outermost**).
    #[inline]
    pub fn arbitrate(
        &mut self,
        hit_chain: &[SurfaceId],
        sample: PointerSample,
    ) -> ArbitrationDecision {
        if hit_chain.is_empty() {
            if matches!(sample.phase, PointerPhase::Up | PointerPhase::Cancel) {
                self.active_pointers.remove(sample.pointer_id);
            }
            return ArbitrationDecision {
                innermost_disposition: GestureDisposition::PassThrough,
                winner: None,
                cancelled_surfaces: Vec::new(),
                reason: ArbitrationReason::NoEligibleSurface,
            };
        }

        let innermost = hit_chain[0];

        match sample.phase {
            PointerPhase::Down => self.handle_down(hit_chain, innermost, sample),
            PointerPhase::Move => self.handle_move(hit_chain, innermost, sample),
            PointerPhase::Up | PointerPhase::Cancel => self.handle_end(innermost, sample),
        }
    }

    #[inline]
    fn handle_down(
        &mut self,
        hit_chain: &[SurfaceId],
        innermost: SurfaceId,
        sample: PointerSample,
    ) -> ArbitrationDecision {
        if !self.overrides.is_empty()
            && let Some(&forced) = self.overrides.get(innermost)
        {
            let winner = match forced {
                GestureDisposition::Claim => Some(innermost),
                GestureDisposition::DeferToParent => hit_chain.get(1).copied(),
                GestureDisposition::PassThrough => hit_chain
                    .iter()
                    .skip(1)
                    .copied()
                    .find(|id| self.is_interactive(*id)),
            };
            self.active_pointers.insert(
                sample.pointer_id,
                ActivePointer {
                    chain: SurfaceChain::from_slice(hit_chain),
                    down_x: sample.x,
                    down_y: sample.y,
                    winner,
                    locked_scroll: false,
                },
            );
            return ArbitrationDecision {
                innermost_disposition: forced,
                winner,
                cancelled_surfaces: Vec::new(),
                reason: ArbitrationReason::ExplicitOverride,
            };
        }

        let tentative_winner = hit_chain.iter().copied().find(|id| self.is_interactive(*id));
        let innermost_disposition = match tentative_winner {
            Some(id) if id == innermost => GestureDisposition::Claim,
            Some(_) => GestureDisposition::PassThrough,
            None => GestureDisposition::PassThrough,
        };

        self.active_pointers.insert(
            sample.pointer_id,
            ActivePointer {
                chain: SurfaceChain::from_slice(hit_chain),
                down_x: sample.x,
                down_y: sample.y,
                winner: tentative_winner,
                locked_scroll: false,
            },
        );

        ArbitrationDecision {
            innermost_disposition,
            winner: tentative_winner,
            cancelled_surfaces: Vec::new(),
            reason: ArbitrationReason::InitialTouchDown,
        }
    }

    #[inline]
    fn handle_move(
        &mut self,
        hit_chain: &[SurfaceId],
        innermost: SurfaceId,
        sample: PointerSample,
    ) -> ArbitrationDecision {
        let capabilities = &self.capabilities;
        let track_opt = self.active_pointers.get_mut(sample.pointer_id);

        let (chain, down_x, down_y, prev_winner, prev_locked_scroll): (
            &[SurfaceId],
            f32,
            f32,
            Option<SurfaceId>,
            bool,
        ) = if let Some(track) = track_opt.as_deref() {
            (
                track.chain.as_slice(),
                track.down_x,
                track.down_y,
                track.winner,
                track.locked_scroll,
            )
        } else {
            // Synthetic Move without prior Down: borrow hit_chain directly with zero allocation.
            (
                hit_chain,
                sample.x - sample.delta_x,
                sample.y - sample.delta_y,
                None,
                false,
            )
        };

        let total_dx = sample.x - down_x;
        let total_dy = sample.y - down_y;
        let step_dx = if sample.delta_x != 0.0 {
            sample.delta_x
        } else {
            total_dx
        };
        let step_dy = if sample.delta_y != 0.0 {
            sample.delta_y
        } else {
            total_dy
        };

        if !prev_locked_scroll {
            let slop = capabilities
                .get(innermost)
                .map(|c| c.touch_slop_px)
                .unwrap_or(8.0);
            let dist_sq = total_dx * total_dx + total_dy * total_dy;
            if slop > 0.0 && dist_sq < slop * slop {
                let disp = if prev_winner == Some(innermost) {
                    GestureDisposition::Claim
                } else {
                    GestureDisposition::PassThrough
                };
                return ArbitrationDecision {
                    innermost_disposition: disp,
                    winner: prev_winner,
                    cancelled_surfaces: Vec::new(),
                    reason: ArbitrationReason::WithinTouchSlop,
                };
            }
        }

        // Determine dominant scroll axis from step (or total if step is tiny).
        let dominant_horizontal = if step_dx.abs() != step_dy.abs() {
            step_dx.abs() > step_dy.abs()
        } else {
            total_dx.abs() > total_dy.abs()
        };

        // Evaluate innermost-to-outermost for scroll ownership.
        let mut chosen_winner: Option<SurfaceId> = None;
        let mut deferred_inner = false;
        let mut consumed_boundary_locally = false;

        for (idx, &surface_id) in chain.iter().enumerate() {
            let Some(caps) = capabilities.get(surface_id) else {
                continue;
            };
            let supports_axis = if dominant_horizontal {
                caps.scroll_axes.horizontal
            } else {
                caps.scroll_axes.vertical
            };
            if !supports_axis {
                continue;
            }

            let can_scroll = if dominant_horizontal {
                caps.scroll_edges.can_scroll_horizontal(step_dx)
            } else {
                caps.scroll_edges.can_scroll_vertical(step_dy)
            };

            if can_scroll {
                chosen_winner = Some(surface_id);
                break;
            }

            // At boundary: either defer to parent or consume locally (overscroll bounce).
            if caps.nested_scroll_enabled {
                if idx == 0 {
                    deferred_inner = true;
                }
                continue;
            } else {
                chosen_winner = Some(surface_id);
                consumed_boundary_locally = true;
                break;
            }
        }

        let mut cancelled_surfaces = Vec::new();
        if let Some(prev) = prev_winner
            && Some(prev) != chosen_winner
        {
            cancelled_surfaces.push(prev);
        }

        if let Some(track) = track_opt {
            track.winner = chosen_winner;
            track.locked_scroll = chosen_winner.is_some();
        }

        let (innermost_disposition, reason) = match chosen_winner {
            Some(w) if w == innermost => {
                if consumed_boundary_locally {
                    (
                        GestureDisposition::Claim,
                        ArbitrationReason::ScrollBoundaryConsumedLocally,
                    )
                } else if prev_locked_scroll && prev_winner == Some(innermost) {
                    (
                        GestureDisposition::Claim,
                        ArbitrationReason::LockedWinnerContinued,
                    )
                } else {
                    (
                        GestureDisposition::Claim,
                        ArbitrationReason::ScrollAxisClaimed,
                    )
                }
            }
            Some(_) => {
                if deferred_inner {
                    (
                        GestureDisposition::DeferToParent,
                        ArbitrationReason::ScrollBoundaryDeferredToParent,
                    )
                } else {
                    (
                        GestureDisposition::PassThrough,
                        ArbitrationReason::OrthogonalAxisPassThrough,
                    )
                }
            }
            None => {
                if deferred_inner {
                    (
                        GestureDisposition::DeferToParent,
                        ArbitrationReason::ScrollBoundaryDeferredToParent,
                    )
                } else {
                    (
                        GestureDisposition::PassThrough,
                        ArbitrationReason::NoEligibleSurface,
                    )
                }
            }
        };

        ArbitrationDecision {
            innermost_disposition,
            winner: chosen_winner,
            cancelled_surfaces,
            reason,
        }
    }

    #[inline]
    fn handle_end(&mut self, innermost: SurfaceId, sample: PointerSample) -> ArbitrationDecision {
        let removed = self.active_pointers.remove(sample.pointer_id);
        let winner = removed.as_ref().and_then(|t| t.winner);
        let cancelled = if sample.phase == PointerPhase::Cancel {
            winner.into_iter().collect()
        } else {
            Vec::new()
        };
        let innermost_disposition = if winner == Some(innermost) {
            GestureDisposition::Claim
        } else {
            GestureDisposition::PassThrough
        };
        ArbitrationDecision {
            innermost_disposition,
            winner,
            cancelled_surfaces: cancelled,
            reason: ArbitrationReason::PointerEnded,
        }
    }

    #[inline(always)]
    fn is_interactive(&self, surface_id: SurfaceId) -> bool {
        self.capabilities
            .get(surface_id)
            .map(|c| c.handles_tap || c.scroll_axes.any())
            .unwrap_or(true)
    }
}
