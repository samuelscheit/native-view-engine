use std::collections::HashMap;

use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize};

use crate::surface::FrameworkKind;

/// Outcome of hit-testing / pointer arbitration for a surface across framework boundaries.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureDisposition {
    /// Surface owns this pointer sequence; competing parents/children must yield.
    Claim = 1,
    /// Surface does not handle this gesture (e.g. orthogonal scroll axis or transparent hit).
    PassThrough = 2,
    /// Surface could handle this gesture in general, but is yielding control to an ancestor
    /// (e.g. inner scrollable reached its scroll boundary and hands off overscroll to outer).
    DeferToParent = 3,
}

/// Phase of a pointer / touch interaction.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPhase {
    Down = 1,
    Move = 2,
    Up = 3,
    Cancel = 4,
}

/// Scroll axes a surface can actively consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScrollAxes {
    pub horizontal: bool,
    pub vertical: bool,
}

impl ScrollAxes {
    pub const NONE: Self = Self {
        horizontal: false,
        vertical: false,
    };
    pub const VERTICAL: Self = Self {
        horizontal: false,
        vertical: true,
    };
    pub const HORIZONTAL: Self = Self {
        horizontal: true,
        vertical: false,
    };
    pub const BOTH: Self = Self {
        horizontal: true,
        vertical: true,
    };

    pub fn any(self) -> bool {
        self.horizontal || self.vertical
    }
}

/// Boundary state of a scrollable surface, used to determine when to defer to a parent scrollable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScrollEdgeState {
    pub at_top: bool,
    pub at_bottom: bool,
    pub at_left: bool,
    pub at_right: bool,
}

impl ScrollEdgeState {
    /// Returns whether the scrollable can consume a finger drag `(delta_x, delta_y)`.
    ///
    /// Convention (standard touch dragging):
    /// - `delta_y > 0` (finger drags down) scrolls toward the top -> blocked if `at_top`.
    /// - `delta_y < 0` (finger drags up) scrolls toward the bottom -> blocked if `at_bottom`.
    /// - `delta_x > 0` (finger drags right) scrolls toward the left -> blocked if `at_left`.
    /// - `delta_x < 0` (finger drags left) scrolls toward the right -> blocked if `at_right`.
    pub fn can_scroll_vertical(self, delta_y: f32) -> bool {
        if delta_y > 0.0 {
            !self.at_top
        } else if delta_y < 0.0 {
            !self.at_bottom
        } else {
            true
        }
    }

    pub fn can_scroll_horizontal(self, delta_x: f32) -> bool {
        if delta_x > 0.0 {
            !self.at_left
        } else if delta_x < 0.0 {
            !self.at_right
        } else {
            true
        }
    }
}

/// Declared gesture and scroll capabilities of a registered surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GestureCapabilities {
    pub framework: FrameworkKind,
    pub handles_tap: bool,
    pub scroll_axes: ScrollAxes,
    pub scroll_edges: ScrollEdgeState,
    /// When `true`, hitting a scroll boundary defers drag to the nearest capable ancestor.
    pub nested_scroll_enabled: bool,
    /// Minimum drag distance (logical px) before classifying a pointer as a scroll/drag.
    pub touch_slop_px: f32,
}

impl Default for GestureCapabilities {
    fn default() -> Self {
        Self {
            framework: FrameworkKind::Nve,
            handles_tap: true,
            scroll_axes: ScrollAxes::NONE,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }
}

impl GestureCapabilities {
    pub fn vertical_scroll(framework: FrameworkKind) -> Self {
        Self {
            framework,
            handles_tap: true,
            scroll_axes: ScrollAxes::VERTICAL,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }

    pub fn horizontal_scroll(framework: FrameworkKind) -> Self {
        Self {
            framework,
            handles_tap: true,
            scroll_axes: ScrollAxes::HORIZONTAL,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }

    pub fn pass_through(framework: FrameworkKind) -> Self {
        Self {
            framework,
            handles_tap: false,
            scroll_axes: ScrollAxes::NONE,
            scroll_edges: ScrollEdgeState::default(),
            nested_scroll_enabled: true,
            touch_slop_px: 8.0,
        }
    }
}

/// Incoming pointer sample submitted to [`GestureArbitrator`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerSample {
    pub pointer_id: u32,
    pub phase: PointerPhase,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub timestamp_us: u64,
}

/// Why the arbitrator selected a disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArbitrationReason {
    InitialTouchDown,
    WithinTouchSlop,
    ExplicitOverride,
    ScrollAxisClaimed,
    OrthogonalAxisPassThrough,
    ScrollBoundaryDeferredToParent,
    ScrollBoundaryConsumedLocally,
    LockedWinnerContinued,
    PointerEnded,
    NoEligibleSurface,
}

/// Result of arbitrating a pointer sample across an innermost-to-outermost surface chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitrationDecision {
    /// Disposition from the perspective of the innermost hit surface.
    pub innermost_disposition: GestureDisposition,
    /// The surface currently owning the pointer gesture, if any.
    pub winner: Option<SurfaceId>,
    /// Surfaces that previously owned or were tentatively tracking this pointer and must receive `Cancel`.
    pub cancelled_surfaces: Vec<SurfaceId>,
    pub reason: ArbitrationReason,
}

#[derive(Debug, Clone)]
struct ActivePointer {
    chain: Vec<SurfaceId>,
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
    capabilities: HashMap<SurfaceId, GestureCapabilities>,
    overrides: HashMap<SurfaceId, GestureDisposition>,
    active_pointers: HashMap<u32, ActivePointer>,
}

impl GestureArbitrator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_capabilities(&mut self, surface_id: SurfaceId, caps: GestureCapabilities) {
        self.capabilities.insert(surface_id, caps);
    }

    pub fn update_scroll_edges(&mut self, surface_id: SurfaceId, edges: ScrollEdgeState) {
        if let Some(caps) = self.capabilities.get_mut(&surface_id) {
            caps.scroll_edges = edges;
        }
    }

    /// Forces an explicit disposition for a surface (e.g. from a custom native hit-test hook).
    pub fn set_override(&mut self, surface_id: SurfaceId, disposition: Option<GestureDisposition>) {
        match disposition {
            Some(d) => {
                self.overrides.insert(surface_id, d);
            }
            None => {
                self.overrides.remove(&surface_id);
            }
        }
    }

    /// Removes all gesture state for a destroyed surface.
    pub fn unregister_surface(&mut self, surface_id: SurfaceId) {
        self.capabilities.remove(&surface_id);
        self.overrides.remove(&surface_id);
        for track in self.active_pointers.values_mut() {
            track.chain.retain(|&id| id != surface_id);
            if track.winner == Some(surface_id) {
                track.winner = None;
                track.locked_scroll = false;
            }
        }
        self.active_pointers.retain(|_, t| !t.chain.is_empty());
    }

    pub fn active_pointer_count(&self) -> usize {
        self.active_pointers.len()
    }

    /// Arbitrates a pointer sample along `hit_chain` (ordered **innermost -> outermost**).
    pub fn arbitrate(
        &mut self,
        hit_chain: &[SurfaceId],
        sample: PointerSample,
    ) -> ArbitrationDecision {
        if hit_chain.is_empty() {
            if matches!(sample.phase, PointerPhase::Up | PointerPhase::Cancel) {
                self.active_pointers.remove(&sample.pointer_id);
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

    fn handle_down(
        &mut self,
        hit_chain: &[SurfaceId],
        innermost: SurfaceId,
        sample: PointerSample,
    ) -> ArbitrationDecision {
        if let Some(&forced) = self.overrides.get(&innermost) {
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
                    chain: hit_chain.to_vec(),
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
                chain: hit_chain.to_vec(),
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

    fn handle_move(
        &mut self,
        hit_chain: &[SurfaceId],
        innermost: SurfaceId,
        sample: PointerSample,
    ) -> ArbitrationDecision {
        let (chain, down_x, down_y, prev_winner, prev_locked_scroll) =
            if let Some(track) = self.active_pointers.get(&sample.pointer_id) {
                (
                    track.chain.clone(),
                    track.down_x,
                    track.down_y,
                    track.winner,
                    track.locked_scroll,
                )
            } else {
                // Synthetic Move without prior Down: treat hit_chain as active.
                (
                    hit_chain.to_vec(),
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

        let slop = self
            .capabilities
            .get(&innermost)
            .map(|c| c.touch_slop_px)
            .unwrap_or(8.0);
        let distance = (total_dx * total_dx + total_dy * total_dy).sqrt();

        if !prev_locked_scroll && distance < slop {
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
            let Some(caps) = self.capabilities.get(&surface_id) else {
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
        if let Some(prev) = prev_winner {
            if Some(prev) != chosen_winner {
                cancelled_surfaces.push(prev);
            }
        }

        if let Some(track) = self.active_pointers.get_mut(&sample.pointer_id) {
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

    fn handle_end(&mut self, innermost: SurfaceId, sample: PointerSample) -> ArbitrationDecision {
        let removed = self.active_pointers.remove(&sample.pointer_id);
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

    fn is_interactive(&self, surface_id: SurfaceId) -> bool {
        self.capabilities
            .get(&surface_id)
            .map(|c| c.handles_tap || c.scroll_axes.any())
            .unwrap_or(true)
    }
}
