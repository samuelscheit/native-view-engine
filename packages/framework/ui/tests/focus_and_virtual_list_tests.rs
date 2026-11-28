//! Deep unit & edge-case tests for `FocusScope`, `FocusTrap`, `RovingFocus`, `FenwickHeightTree`,
//! `VirtualListState`, `Theme::from_theme_context`, `ToastQueue`, and `TypedStyle`/`SurfaceStyleExt`.

use std::collections::BTreeMap;

use nve_abi::ColorRgba;
use nve_interop::{ColorScheme, ContrastLevel, ThemeContext};
use nve_ui::prelude::*;
use nve_ui::{wcag_contrast, Hex, Oklch};

#[test]
fn focus_trap_wrap_around_disabled_skipping_and_restore_edge_cases() {
    let mut trap = FocusTrap::new([
        FocusableEntry::disabled("a", true),
        FocusableEntry::new("b"),
        FocusableEntry::disabled("c", true),
        FocusableEntry::new("d"),
    ]);

    // Ignored when closed.
    assert_eq!(trap.handle_key("Tab", false), FocusTrapAction::Ignored);
    assert_eq!(trap.handle_key("Escape", false), FocusTrapAction::Ignored);

    // Activates on first enabled ("b"), remembers previous focus.
    assert_eq!(trap.activate(Some("trigger-1".into())), Some("b"));
    assert_eq!(trap.previous_focus(), Some("trigger-1"));

    // Tab skips disabled "c" -> "d", wraps around skipping disabled "a" -> "b".
    assert_eq!(
        trap.handle_key("Tab", false),
        FocusTrapAction::FocusChanged("d".into())
    );
    assert_eq!(
        trap.handle_key("Tab", false),
        FocusTrapAction::FocusChanged("b".into())
    );
    // Shift+Tab wraps backward skipping disabled "a" -> "d".
    assert_eq!(
        trap.handle_key("Tab", true),
        FocusTrapAction::FocusChanged("d".into())
    );

    // Disabling active item moves focus to remaining enabled item.
    trap.scope_mut().set_disabled("d", true);
    assert_eq!(trap.active(), Some("b"));

    // Escape dismisses and returns previous focus.
    assert_eq!(
        trap.handle_key("Escape", false),
        FocusTrapAction::Dismiss {
            restore_focus: Some("trigger-1".into())
        }
    );
    assert!(!trap.is_open());
    assert_eq!(trap.active(), None);

    // Empty and all-disabled traps never panic.
    let mut empty_trap = FocusTrap::new(Vec::<FocusableEntry>::new());
    assert_eq!(empty_trap.activate(None), None);
    assert_eq!(empty_trap.handle_key("Tab", false), FocusTrapAction::Ignored);
    assert_eq!(empty_trap.handle_key("Tab", true), FocusTrapAction::Ignored);

    // close_on_escape = false ignores Escape.
    let mut sticky = FocusTrap::from_ids(["only"]).with_close_on_escape(false);
    assert_eq!(sticky.activate(Some("prev".into())), Some("only"));
    assert_eq!(sticky.handle_key("Escape", false), FocusTrapAction::Ignored);
    assert_eq!(sticky.deactivate(), Some("prev".into()));
}

#[test]
fn roving_focus_horizontal_vertical_disabled_and_non_looping() {
    let mut h = RovingFocus::with_items(
        Orientation::Horizontal,
        [
            FocusableEntry::disabled("t0", true),
            FocusableEntry::new("t1"),
            FocusableEntry::disabled("t2", true),
            FocusableEntry::new("t3"),
            FocusableEntry::disabled("t4", true),
        ],
    );
    assert_eq!(h.active_index(), Some(1));
    assert_eq!(h.tab_index_for(0), -1);
    assert_eq!(h.tab_index_for(1), 0);
    assert_eq!(h.tab_index_for(3), -1);

    // Vertical keys ignored in horizontal mode.
    assert_eq!(h.handle_key("ArrowDown"), None);
    assert_eq!(h.handle_key("ArrowRight"), Some(3));
    assert_eq!(h.handle_key("ArrowRight"), Some(1)); // wraps skipping t4 and t0
    assert_eq!(h.handle_key("End"), Some(3));
    assert_eq!(h.handle_key("Home"), Some(1));

    // Non-looping vertical menu clamps at enabled ends.
    let mut v = RovingFocus::with_items(
        Orientation::Vertical,
        [
            FocusableEntry::new("m0"),
            FocusableEntry::disabled("m1", true),
            FocusableEntry::new("m2"),
        ],
    )
    .with_loop(false);
    assert_eq!(v.handle_key("ArrowUp"), Some(0));
    assert_eq!(v.handle_key("ArrowDown"), Some(2));
    assert_eq!(v.handle_key("ArrowDown"), Some(2));

    // All-disabled roving focus returns None gracefully.
    let mut all_dis = RovingFocus::with_items(
        Orientation::Vertical,
        [FocusableEntry::disabled("x", true)],
    );
    assert_eq!(all_dis.active_index(), None);
    assert_eq!(all_dis.handle_key("ArrowDown"), None);
    assert_eq!(all_dis.tab_index_for(0), -1);
}

#[cfg(feature = "virtual-list")]
#[test]
fn fenwick_height_tree_100k_prefix_sums_lower_bound_and_live_updates() {
    let n = 100_000usize;
    let mut heights: Vec<u32> = (0..n).map(|i| 20 + ((i * 7) % 45) as u32).collect();
    let mut tree = FenwickHeightTree::from_heights(heights.clone());

    let expected_total: u64 = heights.iter().map(|&h| u64::from(h)).sum();
    assert_eq!(tree.len(), n);
    assert_eq!(tree.total_height(), expected_total);

    // Verify prefix sums & lower_bound across checkpoints.
    let mut running = 0u64;
    for (idx, &h) in heights.iter().enumerate() {
        if idx % 9_973 == 0 || idx == n - 1 {
            assert_eq!(tree.prefix_sum(idx), running);
            assert_eq!(tree.lower_bound(running), idx);
            assert_eq!(tree.lower_bound(running + u64::from(h) - 1), idx);
        }
        running += u64::from(h);
    }
    assert_eq!(tree.lower_bound(expected_total), n);
    assert_eq!(tree.lower_bound(expected_total + 50_000), n);

    // Live O(log N) row height updates (grow and shrink) maintain exact sums.
    assert!(tree.set_height(12_345, 180));
    heights[12_345] = 180;
    assert!(tree.set_height(50_000, 12));
    heights[50_000] = 12;
    assert!(!tree.set_height(n, 50));

    let new_total: u64 = heights.iter().map(|&h| u64::from(h)).sum();
    assert_eq!(tree.total_height(), new_total);
    let prefix_50001: u64 = heights[..50_001].iter().map(|&h| u64::from(h)).sum();
    assert_eq!(tree.prefix_sum(50_001), prefix_50001);
    assert_eq!(tree.lower_bound(prefix_50001), 50_001);
}

#[cfg(feature = "virtual-list")]
#[test]
fn virtual_list_edge_cases_empty_single_nan_overscroll_and_scroll_to_index() {
    // 1. Empty list (including pre-allocated empty Vec -> 0 heap bytes!).
    let empty_tree = FenwickHeightTree::from_heights(Vec::with_capacity(64));
    assert_eq!(empty_tree.memory_bytes(), std::mem::size_of::<FenwickHeightTree>());
    let mut empty = VirtualListState::new(RowHeights::variable(vec![]), 400.0, 3);
    assert_eq!(
        empty.slice(),
        VirtualSlice {
            start: 0,
            end: 0,
            first_visible: 0,
            end_visible: 0,
            clamped_scroll_offset: 0.0,
            top_spacer_px: 0,
            rendered_height_px: 0,
            bottom_spacer_px: 0,
            total_height_px: 0,
            visible_count: 0,
        }
    );
    assert_eq!(empty.scroll_to_index(0, ScrollAlign::Start), 0.0);
    assert_eq!(empty.handle_key("ArrowDown"), None);

    // 2. Single item.
    let mut single = VirtualListState::new(RowHeights::variable(vec![48]), 300.0, 2);
    let s1 = single.slice();
    assert_eq!((s1.start, s1.end, s1.top_spacer_px, s1.bottom_spacer_px), (0, 1, 0, 0));
    assert_eq!(single.scroll_to_index(99, ScrollAlign::Center), 0.0);
    assert_eq!(single.handle_key("ArrowDown"), Some(0));
    assert_eq!(single.handle_key("ArrowUp"), Some(0));

    // 3. Variable heights + NaN / negative / +inf / overscrolled offsets + direct NaN viewport_height.
    let raw: Vec<u32> = (0..1_000).map(|i| 30 + ((i % 5) as u32) * 10).collect();
    let mut var = VirtualListState::new(RowHeights::variable(raw.clone()), 250.0, 2);

    assert_eq!(var.set_scroll_offset(f64::NAN), 0.0);
    assert_eq!(var.set_scroll_offset(-500.0), 0.0);
    assert_eq!(var.set_scroll_offset(f64::INFINITY), 0.0);

    var.viewport_height = f64::NAN;
    assert_eq!(var.max_scroll_offset(), var.heights.total_height() as f64);
    var.viewport_height = 250.0;

    let max_scroll = var.max_scroll_offset();
    assert_eq!(var.set_scroll_offset(99_999_999.0), max_scroll);
    let end_slice = var.slice();
    assert_eq!(end_slice.end, 1_000);
    assert_eq!(end_slice.bottom_spacer_px, 0);

    // Exact u64 spacer conservation across offsets and scroll_to_index alignments.
    for (idx, align) in [
        (0, ScrollAlign::Start),
        (500, ScrollAlign::Center),
        (999, ScrollAlign::End),
        (250, ScrollAlign::Nearest),
    ] {
        var.scroll_to_index(idx, align);
        let sl = var.slice();
        let rendered_h: u64 = raw[sl.start..sl.end].iter().map(|&h| u64::from(h)).sum();
        assert_eq!(sl.rendered_height_px, rendered_h);
        assert_eq!(
            sl.top_spacer_px + sl.rendered_height_px + sl.bottom_spacer_px,
            sl.total_height_px,
            "spacer conservation violated at idx={idx}"
        );
        assert!(sl.visible_count <= 20, "slice must stay O(viewport)");
    }

    // Uniform 100k list stays strictly O(1) with exact bounds.
    let mut uniform = VirtualListState::new(RowHeights::uniform(100_000, 40), 400.0, 3);
    uniform.scroll_to_index(50_000, ScrollAlign::Start);
    assert_eq!(uniform.scroll_offset, 2_000_000.0);
    let u_sl = uniform.slice();
    assert_eq!(u_sl.start, 49_997);
    assert_eq!(u_sl.end, 50_013);
    assert_eq!(u_sl.first_visible, 50_000);
    assert_eq!(u_sl.end_visible, 50_010);
    assert_eq!(u_sl.top_spacer_px, 49_997 * 40);
    assert_eq!(u_sl.rendered_height_px, 16 * 40);
    assert_eq!(
        u_sl.top_spacer_px + u_sl.rendered_height_px + u_sl.bottom_spacer_px,
        u_sl.total_height_px
    );
}

#[cfg(feature = "virtual-list")]
#[test]
fn row_heights_uniform_to_variable_promotion_revisions_and_cow() {
    let mut heights = RowHeights::uniform(1_000, 40);
    assert!(!heights.is_variable());
    let rev_initial = heights.revision();

    // 1. Out-of-bounds update returns false, keeps Uniform mode, and keeps revision unchanged.
    assert!(!heights.set_row_height(1_000, 80));
    assert!(!heights.set_row_height(usize::MAX, 80));
    assert!(!heights.is_variable());
    assert_eq!(heights.revision(), rev_initial);

    // 2. No-op update matching uniform height stays Uniform with unchanged revision.
    assert!(heights.set_row_height(500, 40));
    assert!(!heights.is_variable());
    assert_eq!(heights.revision(), rev_initial);

    // 3. First heterogeneous update performs one-time O(N) lossless promotion to Variable.
    assert!(heights.set_row_height(500, 96));
    assert!(heights.is_variable());
    let rev_promoted = heights.revision();
    assert_ne!(rev_promoted, rev_initial);
    assert_eq!(heights.total_height(), 999 * 40 + 96);
    assert_eq!(heights.row_height(499), 40);
    assert_eq!(heights.row_height(500), 96);
    assert_eq!(heights.row_height(501), 40);
    assert_eq!(heights.offset_of(500), 500 * 40);
    assert_eq!(heights.offset_of(501), 500 * 40 + 96);
    assert_eq!(heights.index_and_offset_at((500 * 40 + 95) as f64), (500, 500 * 40));
    assert_eq!(heights.index_and_offset_at((500 * 40 + 96) as f64), (501, 500 * 40 + 96));

    // 4. Subsequent no-op & OOB updates in Variable mode keep revision unchanged.
    assert!(heights.set_row_height(500, 96));
    assert!(!heights.set_row_height(1_000, 50));
    assert_eq!(heights.revision(), rev_promoted);

    // 5. Subsequent mutating update bumps revision and updates exact offsets & slices.
    assert!(heights.set_row_height(10, 120));
    let rev_mutated = heights.revision();
    assert_ne!(rev_mutated, rev_promoted);
    assert_eq!(heights.total_height(), 998 * 40 + 96 + 120);
    assert_eq!(heights.offset_of(501), 499 * 40 + 120 + 96);

    // 6. O(1) Clone + PartialEq/Eq + copy-on-write isolation + explicit content_eq.
    let snapshot = heights.clone();
    assert_eq!(heights, snapshot);
    assert_eq!(heights.revision(), snapshot.revision());
    assert!(heights.content_eq(&snapshot));

    // Mutating shared handle COW-clones without affecting `snapshot`.
    assert!(heights.set_row_height(10, 40));
    assert_ne!(heights, snapshot);
    assert_ne!(heights.revision(), snapshot.revision());
    assert!(!heights.content_eq(&snapshot));
    assert_eq!(snapshot.row_height(10), 120);
    assert_eq!(heights.row_height(10), 40);

    // Two independently built identical Variable tables compare != in O(1) PartialEq,
    // while content_eq compares true (including Uniform <-> Variable equivalence).
    let indep_a = RowHeights::variable(vec![40; 16]);
    let indep_b = RowHeights::variable(vec![40; 16]);
    let uniform_16 = RowHeights::uniform(16, 40);
    assert_ne!(indep_a, indep_b);
    assert!(indep_a.content_eq(&indep_b));
    assert!(indep_a.content_eq(&uniform_16));
    assert!(uniform_16.content_eq(&indep_a));
    assert_eq!(RowHeights::uniform(16, 40), uniform_16);
    assert_eq!(RowHeights::uniform(16, 40).revision(), uniform_16.revision());
    assert_ne!(RowHeights::uniform(16, 40).revision(), RowHeights::uniform(17, 40).revision());
    assert_ne!(RowHeights::uniform(16, 40).revision(), indep_a.revision());

    // Exact Uniform <-> Variable slice parity on identical heights.
    for &vp in &[0.0, 1.0, 37.5, 250.0, 1_500.0] {
        for &off in &[-10.0, 0.0, 39.5, 40.0, 120.25, 500.0, 9999.0] {
            for &ov in &[0usize, 3, 33, usize::MAX] {
                assert_eq!(uniform_16.slice(off, vp, ov), indep_a.slice(off, vp, ov));
            }
        }
    }
}

#[cfg(feature = "virtual-list")]
struct XorShift64(u64);

#[cfg(feature = "virtual-list")]
impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn range(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() as usize) % bound
        }
    }
}

#[cfg(feature = "virtual-list")]
fn naive_prefix(heights: &[u32]) -> Vec<u64> {
    let mut prefix = Vec::with_capacity(heights.len() + 1);
    prefix.push(0);
    let mut running = 0u64;
    for &h in heights {
        running = running.saturating_add(u64::from(h.max(1)));
        prefix.push(running);
    }
    prefix
}

#[cfg(feature = "virtual-list")]
fn naive_lower_bound_with_offset(prefix: &[u64], offset_px: u64) -> (usize, u64) {
    let n = prefix.len().saturating_sub(1);
    if n == 0 {
        return (0, 0);
    }
    for i in 0..n {
        if prefix[i + 1] > offset_px {
            return (i, prefix[i]);
        }
    }
    (n, prefix[n])
}

/// Independent geometric interval-intersection reference slicer (directly checks row intervals
/// `[top, bottom)` against `[clamped_offset, clamped_offset + vp)` and sums row heights directly
/// rather than mirroring the implementation's prefix arithmetic).
#[cfg(feature = "virtual-list")]
fn independent_reference_slice(
    heights: &[u32],
    scroll_offset: f64,
    viewport_height: f64,
    overscan: usize,
) -> VirtualSlice {
    let n = heights.len();
    if n == 0 {
        return VirtualSlice::EMPTY;
    }

    let vp = if viewport_height.is_finite() && viewport_height > 0.0 {
        viewport_height
    } else {
        0.0
    };
    let total: u64 = heights.iter().map(|&h| u64::from(h.max(1))).sum();
    let max_scroll = ((total as f64) - vp).max(0.0);
    let offset = if scroll_offset.is_finite() && scroll_offset > 0.0 {
        scroll_offset.min(max_scroll)
    } else {
        0.0
    };
    let offset_floor = offset.floor() as u64;
    let vp_bottom = offset + vp;

    // Scan all row intervals [top, bottom) to determine exact intersecting visibility range.
    let mut top = 0u64;
    let mut first_visible = n - 1;
    let mut found_first = false;
    let mut end_visible = 0usize;
    for (i, &raw_h) in heights.iter().enumerate() {
        let h = u64::from(raw_h.max(1));
        let bottom = top + h;
        if !found_first && bottom > offset_floor {
            first_visible = i;
            found_first = true;
        }
        if (top as f64) < vp_bottom {
            end_visible = i + 1;
        }
        top = bottom;
    }
    end_visible = end_visible.max(first_visible + 1).min(n);

    let start = first_visible.saturating_sub(overscan);
    let end = end_visible.saturating_add(overscan).min(n);
    let top_spacer_px: u64 = heights[..start].iter().map(|&h| u64::from(h.max(1))).sum();
    let rendered_height_px: u64 = heights[start..end].iter().map(|&h| u64::from(h.max(1))).sum();
    let bottom_spacer_px: u64 = heights[end..].iter().map(|&h| u64::from(h.max(1))).sum();

    VirtualSlice {
        start,
        end,
        first_visible,
        end_visible,
        clamped_scroll_offset: offset,
        top_spacer_px,
        rendered_height_px,
        bottom_spacer_px,
        total_height_px: total,
        visible_count: end - start,
    }
}

#[cfg(feature = "virtual-list")]
#[test]
fn property_500_random_trees_match_independent_reference_and_navigation() {
    let mut rng = XorShift64::new(0x5EED_CAFE_BEEF_1234);
    let edge_sizes: [usize; 16] = [
        0, 1, 2, 3, 7, 8, 15, 16, 17, 63, 64, 65, 255, 256, 1024, 10_000,
    ];
    let overscans = [0usize, 1, 3, 31, 32, 33, 64, usize::MAX];
    let viewports = [
        -10.0f64,
        -0.0,
        0.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1.0,
        37.5,
        250.0,
        1_500.0,
    ];

    // First 3 * 16 = 48 cases systematically cover EVERY edge size across all 3 constructors
    // (from_heights, variable_from_fn, uniform->variable promotion) with zero modulo aliasing;
    // remaining 500 cases use independent RNG draws across sizes, viewports, and overscans.
    let total_cases = edge_sizes.len() * 3 + 500;
    for case_idx in 0..total_cases {
        let (n, mode) = if case_idx < edge_sizes.len() * 3 {
            (edge_sizes[case_idx % edge_sizes.len()], case_idx / edge_sizes.len())
        } else {
            (rng.range(300), rng.range(3))
        };

        let (mut expected_heights, mut row_heights) = if mode == 2 && n > 0 {
            let base = (1 + rng.range(80)) as u32;
            (vec![base; n], RowHeights::uniform(n, base))
        } else {
            let mut raw = Vec::with_capacity(n);
            for _ in 0..n {
                let h = match rng.range(10) {
                    0 => 0,
                    1 => 1,
                    2 => 70_000 + (rng.range(50_000) as u32), // > u16::MAX tall row
                    _ => (1 + rng.range(120)) as u32,
                };
                raw.push(h);
            }
            let normalized: Vec<u32> = raw.iter().map(|&h| h.max(1)).collect();
            let rh = if mode == 0 {
                RowHeights::variable(raw)
            } else {
                RowHeights::variable_from_fn(n, |i| raw[i])
            };
            (normalized, rh)
        };

        // Apply random updates (triggers uniform->variable promotion on first differing height).
        let update_count = if n == 0 { 2 } else { (1 + rng.range(12)).min(n + 2) };
        for _ in 0..update_count {
            let idx = if rng.range(8) == 0 {
                n + rng.range(3)
            } else {
                rng.range(n.max(1))
            };
            let new_h = match rng.range(6) {
                0 => 0,
                1 => 85_000,
                _ => (1 + rng.range(140)) as u32,
            };
            let ok = row_heights.set_row_height(idx, new_h);
            assert_eq!(ok, idx < n, "case {case_idx}: OOB return mismatch");
            if idx < n {
                expected_heights[idx] = new_h.max(1);
            }
        }

        let prefix = naive_prefix(&expected_heights);
        let expected_total = *prefix.last().unwrap();
        assert_eq!(
            row_heights.total_height(),
            expected_total,
            "case {case_idx} (n={n}): total_height mismatch"
        );

        // Verify prefix_sum & lower_bound_with_offset at exact boundaries.
        if let Some(tree) = row_heights.as_tree() {
            assert_eq!(tree.total_height(), expected_total);
            let stride = (n / 32).max(1);
            for i in (0..n).step_by(stride).chain(n.checked_sub(1)) {
                assert_eq!(tree.prefix_sum(i), prefix[i], "case {case_idx} i={i}");
                for probe in [
                    prefix[i].saturating_sub(1),
                    prefix[i],
                    prefix[i].saturating_add(1),
                ] {
                    assert_eq!(
                        tree.lower_bound_with_offset(probe),
                        naive_lower_bound_with_offset(&prefix, probe),
                        "case {case_idx} n={n} probe={probe}"
                    );
                }
            }
            for probe in [
                0,
                expected_total.saturating_sub(1),
                expected_total,
                expected_total.saturating_add(100),
            ] {
                assert_eq!(
                    tree.lower_bound_with_offset(probe),
                    naive_lower_bound_with_offset(&prefix, probe),
                    "case {case_idx} n={n} tail probe={probe}"
                );
            }
        }

        // Probe slices at general offsets + exact row boundary prefix[i] offsets.
        let overscan = overscans[rng.range(overscans.len())];
        let vp = viewports[rng.range(viewports.len())];
        let boundary_a = if n > 0 { prefix[rng.range(n)] } else { 0 };
        let boundary_b = if n > 1 { prefix[n - 1] } else { 0 };
        let probe_offsets = [
            f64::NAN,
            f64::NEG_INFINITY,
            f64::INFINITY,
            -25.0,
            0.0,
            0.5,
            boundary_a.saturating_sub(1) as f64,
            boundary_a as f64,
            (boundary_a as f64) + 0.5,
            boundary_b as f64,
            (expected_total as f64) * 0.5,
            expected_total as f64,
            (expected_total as f64) + 5_000.0,
        ];

        for &off in &probe_offsets {
            let actual = row_heights.slice(off, vp, overscan);
            let expected = independent_reference_slice(&expected_heights, off, vp, overscan);
            assert_eq!(
                actual, expected,
                "case {case_idx} (n={n}, mode={mode}, var={}, off={off}, vp={vp}, overscan={overscan})",
                row_heights.is_variable()
            );
            // Exact u64 conservation ALWAYS holds (no 2^24 float threshold!).
            assert_eq!(
                actual.top_spacer_px + actual.rendered_height_px + actual.bottom_spacer_px,
                actual.total_height_px,
                "case {case_idx}: exact u64 spacer conservation"
            );
        }

        // Randomized scroll_to_index postconditions & handle_key invariants for non-empty trees.
        if n > 0 && n <= 1_024 {
            let test_vp = [50.0, 240.0, 800.0][rng.range(3)];
            let mut st = VirtualListState::new(row_heights.clone(), test_vp, 2);
            for &align in &[
                ScrollAlign::Start,
                ScrollAlign::Center,
                ScrollAlign::End,
                ScrollAlign::Nearest,
            ] {
                let target = rng.range(n);
                let prev = st.scroll_offset;
                let next = st.scroll_to_index(target, align);
                assert!(next >= 0.0 && next <= st.max_scroll_offset());
                let row_top = st.heights.offset_of(target) as f64;
                let row_h = f64::from(st.heights.row_height(target));
                let row_bottom = row_top + row_h;
                if row_h <= test_vp {
                    assert!(
                        row_top >= next - 1e-6 && row_bottom <= next + test_vp + 1e-6,
                        "case {case_idx}: target row {target} not visible after {align:?}"
                    );
                }
                if align == ScrollAlign::Nearest
                    && row_top >= prev
                    && row_bottom <= prev + test_vp
                {
                    assert_eq!(next, prev, "Nearest must not move when row already visible");
                }
            }

            // Keyboard navigation sequence postconditions.
            let mut kst = VirtualListState::new(row_heights.clone(), test_vp, 2);
            assert_eq!(kst.handle_key("UnknownKey"), None);
            assert_eq!(kst.selected_index, None);
            assert_eq!(kst.handle_key("ArrowDown"), Some(0));
            assert_eq!(kst.handle_key("End"), Some(n - 1));
            assert_eq!(kst.handle_key("Home"), Some(0));
            let keys = ["ArrowDown", "ArrowUp", "PageDown", "PageUp", "Home", "End"];
            for _ in 0..16 {
                let key = keys[rng.range(keys.len())];
                let idx = kst.handle_key(key).expect("non-empty handle_key");
                assert!(idx < n);
                assert_eq!(kst.selected_index, Some(idx));
                assert!(kst.scroll_offset >= 0.0 && kst.scroll_offset <= kst.max_scroll_offset());
            }
        }
    }
}

#[cfg(feature = "virtual-list")]
#[test]
fn variable_height_page_up_down_jumps_by_viewport_pixels_and_1m_rows_conserve_u64() {
    // Variable-height PageDown/PageUp: row 0 is tiny (20px) while rows 1..10 are tall (200px).
    // With viewport=400px, PageDown from row 0 must jump ~400px (landing at row 2, offset 220..420),
    // NOT 400/20 = 20 rows!
    let mut heights = vec![20u32];
    heights.extend(std::iter::repeat_n(200u32, 20));
    let mut st = VirtualListState::new(RowHeights::variable(heights), 400.0, 1);
    assert_eq!(st.handle_key("ArrowDown"), Some(0));
    assert_eq!(st.handle_key("PageDown"), Some(2));
    assert_eq!(st.handle_key("PageDown"), Some(4));
    assert_eq!(st.handle_key("PageUp"), Some(2));
    assert_eq!(st.handle_key("PageUp"), Some(0));

    // Tall row (> viewport) still makes forward/backward progress without getting stuck.
    let mut tall = VirtualListState::new(RowHeights::variable(vec![1_000, 1_000, 1_000]), 300.0, 0);
    assert_eq!(tall.handle_key("ArrowDown"), Some(0));
    assert_eq!(tall.handle_key("PageDown"), Some(1));
    assert_eq!(tall.handle_key("PageDown"), Some(2));
    assert_eq!(tall.handle_key("PageUp"), Some(1));
    assert_eq!(tall.handle_key("PageUp"), Some(0));

    // 1,000,000-row exact u64 pixel conservation (> 44,000,000 px, well beyond f32 2^24 precision!).
    const ONE_MILLION: usize = 1_000_000;
    let million = RowHeights::variable_from_fn(ONE_MILLION, |i| 32 + ((i * 17) % 33) as u32);
    let total = million.total_height();
    assert!(total > 40_000_000, "expected >40M px total, got {total}");
    for &frac in &[0.0f64, 0.25, 0.5, 0.75, 0.999, 1.0] {
        let sl = million.slice(frac * (total as f64), 600.0, 3);
        assert_eq!(
            sl.top_spacer_px + sl.rendered_height_px + sl.bottom_spacer_px,
            sl.total_height_px,
            "1M-row exact u64 conservation failed at frac={frac}"
        );
        assert_eq!(sl.total_height_px, total);
        assert!(sl.visible_count <= 30, "1M slice must stay O(viewport)");
    }
}

#[test]
fn theme_context_bridge_toast_queue_and_style_buf_invariants() {
    // ThemeContext accent override keeps the brand hue but forces a maximum-contrast
    // high-contrast solid with an AAA foreground.
    let bright = Theme::from_theme_context(
        &ThemeContext {
            color_scheme: Some(ColorScheme::Light),
            contrast: ContrastLevel::High,
            accent_rgba: 0xFACC_15FF,
            tokens: BTreeMap::new(),
        },
        ColorScheme::Dark,
        AppBrand::Default,
    );
    assert!(!bright.is_dark() && bright.is_high_contrast());
    let hue = Oklch::from_srgb(bright.status.accent.solid).h;
    assert!((80.0..115.0).contains(&hue), "yellow hue preserved, got {hue}");
    assert!(wcag_contrast(bright.status.accent.on_solid, bright.status.accent.solid) >= 7.0);
    assert_eq!(Hex(ColorRgba::rgba(0x11, 0x22, 0x33, 0x80)).to_string(), "#11223380");
    assert_eq!(ColorRgba::rgb(0x09, 0x09, 0x0b).hex().to_string(), "#09090b");

    // ToastQueue max_visible eviction + tick expiration.
    let mut q = ToastQueue::new(2);
    let id1 = q.push(ToastOptions::new("t1").with_duration_ms(Some(500)));
    let id2 = q.push(ToastOptions::new("t2").with_duration_ms(Some(1000)));
    let id3 = q.push(ToastOptions::new("t3").with_duration_ms(None));
    assert_eq!(q.len(), 2);
    assert!(!q.dismiss(id1)); // evicted by max_visible=2
    assert_eq!(q.tick(1000), vec![id2]);
    assert_eq!(q.items().len(), 1);
    assert_eq!(q.items()[0].id, id3);

    // TypedStyle + SurfaceStyleExt sanitizes non-finite numbers and emits full visual property sets.
    let st = nve_dioxus::TypedStyle::new()
        .width_px(f32::NAN)
        .height_px(f32::INFINITY)
        .surface(TypedSurfaceStyle {
            background: ColorRgba::WHITE,
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: 8.0,
            opacity: 1.0,
            box_shadow: None,
        });
    assert_eq!(st.layout().width, nve_abi::Dimension::Points(0.0));
    assert_eq!(st.layout().height, nve_abi::Dimension::Points(0.0));
    assert_eq!(st.style().background, ColorRgba::WHITE);
    assert_eq!(st.style().border_width, 0.0);
    assert_eq!(st.style().border_color, ColorRgba::TRANSPARENT);
    assert_eq!(st.style().border_radius, 8.0);
    assert_eq!(st.style().opacity, 1.0);
    assert_eq!(st.box_shadows.as_deref(), Some(&[][..]));
}
