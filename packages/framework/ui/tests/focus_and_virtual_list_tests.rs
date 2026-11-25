//! Deep unit & edge-case tests for `FocusScope`, `FocusTrap`, `RovingFocus`, `FenwickHeightTree`,
//! `VirtualListState`, `UiTheme::from_theme_context`, `ToastQueue`, and `StyleBuf`.

use std::collections::BTreeMap;

use nve_interop::{ColorScheme, ThemeContext};
use nve_ui::prelude::*;

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
    // 1. Empty list.
    let mut empty = VirtualListState::new(RowHeights::variable(vec![]), 400.0, 3);
    assert_eq!(
        empty.slice(),
        VirtualSlice {
            start: 0,
            end: 0,
            top_spacer_px: 0.0,
            bottom_spacer_px: 0.0,
            total_height_px: 0.0,
            visible_count: 0,
        }
    );
    assert_eq!(empty.scroll_to_index(0, ScrollAlign::Start), 0.0);
    assert_eq!(empty.handle_key("ArrowDown"), None);

    // 2. Single item.
    let mut single = VirtualListState::new(RowHeights::variable(vec![48]), 300.0, 2);
    let s1 = single.slice();
    assert_eq!((s1.start, s1.end, s1.top_spacer_px, s1.bottom_spacer_px), (0, 1, 0.0, 0.0));
    assert_eq!(single.scroll_to_index(99, ScrollAlign::Center), 0.0);
    assert_eq!(single.handle_key("ArrowDown"), Some(0));
    assert_eq!(single.handle_key("ArrowUp"), Some(0));

    // 3. Variable heights + NaN / negative / +inf / overscrolled offsets.
    let raw: Vec<u32> = (0..1_000).map(|i| 30 + ((i % 5) as u32) * 10).collect();
    let mut var = VirtualListState::new(RowHeights::variable(raw.clone()), 250.0, 2);

    assert_eq!(var.set_scroll_offset(f32::NAN), 0.0);
    assert_eq!(var.set_scroll_offset(-500.0), 0.0);
    assert_eq!(var.set_scroll_offset(f32::INFINITY), 0.0);

    let max_scroll = var.max_scroll_offset();
    assert_eq!(var.set_scroll_offset(99_999_999.0), max_scroll);
    let end_slice = var.slice();
    assert_eq!(end_slice.end, 1_000);
    assert_eq!(end_slice.bottom_spacer_px, 0.0);

    // Exact spacer conservation across offsets and scroll_to_index alignments.
    for (idx, align) in [
        (0, ScrollAlign::Start),
        (500, ScrollAlign::Center),
        (999, ScrollAlign::End),
        (250, ScrollAlign::Nearest),
    ] {
        var.scroll_to_index(idx, align);
        let sl = var.slice();
        let rendered_h: f32 = raw[sl.start..sl.end].iter().map(|&h| h as f32).sum();
        assert!(
            ((sl.top_spacer_px + rendered_h + sl.bottom_spacer_px) - sl.total_height_px).abs()
                < 1e-2,
            "spacer conservation violated at idx={idx}"
        );
        assert!(sl.visible_count <= 20, "slice must stay O(viewport)");
    }

    // Uniform 100k list stays strictly O(1) with exact bounds.
    let mut uniform = VirtualListState::new(RowHeights::uniform(100_000, 40.0), 400.0, 3);
    uniform.scroll_to_index(50_000, ScrollAlign::Start);
    assert_eq!(uniform.scroll_offset, 2_000_000.0);
    let u_sl = uniform.slice();
    assert_eq!(u_sl.start, 49_997);
    assert_eq!(u_sl.end, 50_013);
    assert_eq!(u_sl.top_spacer_px, 49_997.0 * 40.0);
}

#[test]
fn theme_context_bridge_toast_queue_and_style_buf_invariants() {
    // ThemeContext with bright yellow accent picks dark foreground; dark purple picks white.
    let mut bright_tokens = BTreeMap::new();
    bright_tokens.insert("surface.bg".into(), "#fafafa".into());
    let bright = UiTheme::from_theme_context(&ThemeContext {
        color_scheme: ColorScheme::HighContrastLight,
        accent_rgba: 0xFACC_15FF,
        tokens: bright_tokens,
    });
    assert_eq!(bright.palette.surface.as_ref(), "#fafafa");
    assert_eq!(bright.palette.accent.as_ref(), "#facc15");
    assert_eq!(bright.palette.accent_fg.as_ref(), "#0f172a");
    assert_eq!(rgba_to_hex(0x1122_3380), "#11223380");

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

    // StyleBuf sanitizes non-finite numbers and always emits full visual property sets.
    let mut sb = StyleBuf::new();
    sb.width_px(f32::NAN)
        .height_px(f32::INFINITY)
        .surface(SurfaceStyle {
            background: "#ffffff",
            border_width: 0.0,
            border_color: "transparent",
            border_radius: 8.0,
            opacity: 1.0,
            box_shadow: "none",
        });
    let css = sb.finish();
    assert!(!css.contains("NaN"));
    assert!(!css.contains("inf"));
    assert!(css.contains("background-color: #ffffff;"));
    assert!(css.contains("border-width: 0px;"));
    assert!(css.contains("border-color: transparent;"));
    assert!(css.contains("opacity: 1;"));
    assert!(css.contains("box-shadow: none;"));
}
