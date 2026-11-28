//! Deep edge-case and property tests for:
//! 1. Anchored overlay positioning (`compute_overlay_position`, `point_in_safe_polygon`)
//! 2. Headless type-safe `FormState` / `FieldState<T>` validation & dirty-tracking
//! 3. Bounded transactional & burst-coalescing `UndoRedoStack<T>`

use nve_ui::form::{FieldState, FormState};
use nve_ui::history::UndoRedoStack;
use nve_ui::positioning::{
    compute_overlay_position, point_in_safe_polygon, Placement, Point, PositionOptions, Rect, Size,
    Viewport,
};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

// ---------------------------------------------------------------------------
// 1. Anchored Overlay Positioning Engine
// ---------------------------------------------------------------------------

#[test]
fn positioning_all_12_placements_exact_geometry() {
    let anchor = Rect::new(400.0, 300.0, 100.0, 40.0);
    let overlay = Size::new(160.0, 60.0);
    let vp = Viewport::new(1000.0, 800.0);
    let offset = 10.0;

    struct Case {
        p: Placement,
        x: f32,
        y: f32,
        ax: Option<f32>,
        ay: Option<f32>,
    }
    let expected = [
        Case { p: Placement::Top, x: 370.0, y: 230.0, ax: Some(80.0), ay: None },
        Case { p: Placement::TopStart, x: 400.0, y: 230.0, ax: Some(50.0), ay: None },
        Case { p: Placement::TopEnd, x: 340.0, y: 230.0, ax: Some(110.0), ay: None },
        Case { p: Placement::Bottom, x: 370.0, y: 350.0, ax: Some(80.0), ay: None },
        Case { p: Placement::BottomStart, x: 400.0, y: 350.0, ax: Some(50.0), ay: None },
        Case { p: Placement::BottomEnd, x: 340.0, y: 350.0, ax: Some(110.0), ay: None },
        Case { p: Placement::Left, x: 230.0, y: 290.0, ax: None, ay: Some(30.0) },
        Case { p: Placement::LeftStart, x: 230.0, y: 300.0, ax: None, ay: Some(20.0) },
        Case { p: Placement::LeftEnd, x: 230.0, y: 280.0, ax: None, ay: Some(40.0) },
        Case { p: Placement::Right, x: 510.0, y: 290.0, ax: None, ay: Some(30.0) },
        Case { p: Placement::RightStart, x: 510.0, y: 300.0, ax: None, ay: Some(20.0) },
        Case { p: Placement::RightEnd, x: 510.0, y: 280.0, ax: None, ay: Some(40.0) },
    ];

    for c in expected {
        let opts = PositionOptions::new(c.p).with_offset(offset);
        let pos = compute_overlay_position(anchor, overlay, vp, &opts);
        assert_eq!(pos.placement, c.p);
        assert!(!pos.flipped, "unexpected flip for {:?}", c.p);
        assert!(!pos.shifted, "unexpected shift for {:?}", c.p);
        assert!(approx(pos.x, c.x), "{:?} x: {} != {}", c.p, pos.x, c.x);
        assert!(approx(pos.y, c.y), "{:?} y: {} != {}", c.p, pos.y, c.y);
        assert_eq!(pos.arrow_x.map(|v| v.round()), c.ax);
        assert_eq!(pos.arrow_y.map(|v| v.round()), c.ay);
    }
}

#[test]
fn positioning_single_flips_and_corner_double_flips() {
    let vp = Viewport::new(800.0, 600.0);
    let overlay = Size::new(200.0, 120.0);

    // Bottom edge -> flips Bottom to Top.
    let bottom_anchor = Rect::new(300.0, 520.0, 80.0, 36.0);
    let pos = compute_overlay_position(
        bottom_anchor,
        overlay,
        vp,
        &PositionOptions::new(Placement::Bottom),
    );
    assert_eq!(pos.placement, Placement::Top);
    assert!(pos.flipped);
    assert!(approx(pos.y, 520.0 - 8.0 - 120.0));

    // Right edge -> flips Right to Left.
    let right_anchor = Rect::new(710.0, 250.0, 60.0, 40.0);
    let pos = compute_overlay_position(
        right_anchor,
        overlay,
        vp,
        &PositionOptions::new(Placement::Right),
    );
    assert_eq!(pos.placement, Placement::Left);
    assert!(pos.flipped);
    assert!(approx(pos.x, 710.0 - 8.0 - 200.0));

    // Bottom-right corner double-flip: BottomStart overflows bottom AND right -> TopEnd fits both!
    let br_anchor = Rect::new(720.0, 540.0, 60.0, 36.0);
    let pos = compute_overlay_position(
        br_anchor,
        overlay,
        vp,
        &PositionOptions::new(Placement::BottomStart),
    );
    assert_eq!(pos.placement, Placement::TopEnd);
    assert!(pos.flipped);
    assert!(!pos.shifted);
    assert!(pos.x >= 8.0 && pos.x + overlay.width <= 792.0);
    assert!(pos.y >= 8.0 && pos.y + overlay.height <= 592.0);

    // Bottom-right corner double-flip for horizontal alignment: RightStart -> LeftEnd.
    let pos_h = compute_overlay_position(
        br_anchor,
        overlay,
        vp,
        &PositionOptions::new(Placement::RightStart),
    );
    assert_eq!(pos_h.placement, Placement::LeftEnd);
    assert!(pos_h.flipped);
    assert!(!pos_h.shifted);

    // Custom fallback placement ordering is honored strictly when fallback fits cleanly.
    let mid_bottom_anchor = Rect::new(300.0, 500.0, 80.0, 36.0);
    let custom = PositionOptions::new(Placement::Bottom)
        .with_fallbacks(vec![Placement::Right, Placement::Top]);
    let pos_custom = compute_overlay_position(mid_bottom_anchor, overlay, vp, &custom);
    assert_eq!(pos_custom.placement, Placement::Right);
    assert!(pos_custom.flipped);
}

#[test]
fn positioning_shift_clamps_viewport_and_tracks_arrow_center() {
    let vp = Viewport::new(500.0, 400.0);
    let overlay = Size::new(240.0, 80.0);

    // Anchor near right edge with centered Bottom placement -> stays Bottom, shifts left, arrow
    // tracks anchor center near right end of overlay.
    let anchor = Rect::new(450.0, 40.0, 30.0, 24.0); // center_x = 465
    let pos = compute_overlay_position(anchor, overlay, vp, &PositionOptions::new(Placement::Bottom));
    assert_eq!(pos.placement, Placement::Bottom);
    assert!(!pos.flipped);
    assert!(pos.shifted);
    // Clamped to vp.width - padding - overlay.width = 500 - 8 - 240 = 252
    assert!(approx(pos.x, 252.0));
    // Arrow tracks anchor center: 465 - 252 = 213 (within [0, 240])
    assert!(approx(pos.arrow_x.unwrap(), 213.0));
    assert!(pos.arrow_y.is_none());

    // Extreme off-screen anchor -> arrow clamps cleanly to [0, overlay.width].
    let far_anchor = Rect::new(700.0, 40.0, 20.0, 20.0);
    let pos_far = compute_overlay_position(
        far_anchor,
        overlay,
        vp,
        &PositionOptions::new(Placement::Bottom).with_flip(false),
    );
    assert!(pos_far.shifted);
    assert!(approx(pos_far.arrow_x.unwrap(), overlay.width));
}

#[test]
fn positioning_tiny_viewport_picks_most_space_and_auto_sizes() {
    // Viewport 220x180, anchor at y=110 (more space above: 110-8-8=94 vs below: 172-148=24).
    let vp = Viewport::new(220.0, 180.0);
    let anchor = Rect::new(80.0, 110.0, 60.0, 30.0);
    let overlay = Size::new(300.0, 250.0);
    let opts = PositionOptions::new(Placement::Bottom).with_auto_size(true);

    let pos = compute_overlay_position(anchor, overlay, vp, &opts);
    // Neither Bottom (24px) nor Top (94px) fits 250px, nor Left (64px)/Right (64px) fits 300px,
    // so solver picks Top (94px = maximum available space).
    assert_eq!(pos.placement, Placement::Top);
    assert!(pos.flipped);
    assert!(approx(pos.max_height.unwrap(), 94.0));
    assert!(approx(pos.max_width.unwrap(), 204.0));
    // Auto-sized overlay sits flush at anchor.y - offset - eff_h = 110 - 8 - 94 = 8.0
    assert!(approx(pos.y, 8.0));
    assert!(approx(pos.x, 8.0));
}

#[test]
fn positioning_property_sweep_invariants() {
    let vp = Viewport::new(640.0, 480.0);
    let pad = 8.0;
    for &placement in Placement::ALL {
        for ax in [0.0, 40.0, 300.0, 590.0] {
            for ay in [0.0, 30.0, 220.0, 440.0] {
                let anchor = Rect::new(ax, ay, 40.0, 28.0);
                let overlay = Size::new(140.0, 90.0);
                let opts = PositionOptions::new(placement)
                    .with_viewport_padding(pad)
                    .with_auto_size(true);
                let pos = compute_overlay_position(anchor, overlay, vp, &opts);
                assert!(pos.x.is_finite() && pos.y.is_finite());
                assert!(pos.x >= pad - 1e-2);
                assert!(pos.y >= pad - 1e-2);
                if let Some(ax_off) = pos.arrow_x {
                    let eff_w = overlay.width.min(pos.max_width.unwrap_or(f32::INFINITY));
                    assert!((-1e-3..=eff_w + 1e-3).contains(&ax_off));
                }
                if let Some(ay_off) = pos.arrow_y {
                    let eff_h = overlay.height.min(pos.max_height.unwrap_or(f32::INFINITY));
                    assert!((-1e-3..=eff_h + 1e-3).contains(&ay_off));
                }
            }
        }
    }
}

#[test]
fn safe_polygon_corridor_all_directions_and_edge_cases() {
    let submenu = Rect::new(200.0, 100.0, 180.0, 200.0);
    let prev = Point::new(120.0, 150.0);

    // Diagonal movement toward bottom of submenu stays inside safe triangle.
    assert!(point_in_safe_polygon(Point::new(170.0, 210.0), prev, submenu));
    // Horizontal movement directly toward submenu stays inside.
    assert!(point_in_safe_polygon(Point::new(160.0, 150.0), prev, submenu));
    // Point already inside submenu is safe.
    assert!(point_in_safe_polygon(Point::new(240.0, 180.0), prev, submenu));
    // Moving straight down outside the triangle dismisses.
    assert!(!point_in_safe_polygon(Point::new(120.0, 250.0), prev, submenu));
    // Moving backward away from submenu dismisses.
    assert!(!point_in_safe_polygon(Point::new(90.0, 150.0), prev, submenu));

    // Submenu positioned to the left of prev_cursor.
    let prev_right = Point::new(450.0, 180.0);
    assert!(point_in_safe_polygon(
        Point::new(410.0, 220.0),
        prev_right,
        submenu
    ));
    assert!(!point_in_safe_polygon(
        Point::new(470.0, 180.0),
        prev_right,
        submenu
    ));

    // Submenu positioned below prev_cursor.
    let prev_above = Point::new(290.0, 40.0);
    assert!(point_in_safe_polygon(
        Point::new(260.0, 80.0),
        prev_above,
        submenu
    ));
    assert!(!point_in_safe_polygon(
        Point::new(180.0, 45.0),
        prev_above,
        submenu
    ));

    // Non-finite inputs return false safely.
    assert!(!point_in_safe_polygon(
        Point::new(f32::NAN, 150.0),
        prev,
        submenu
    ));
}

// ---------------------------------------------------------------------------
// 2. Headless Type-Safe Form Validation & Dirty-Tracking
// ---------------------------------------------------------------------------

#[test]
fn form_dirty_tracking_touched_visibility_cross_field_and_order() {
    let mut form = FormState::new();
    form.register_field("email", String::from("ada@nve.dev"))
        .with_validator(|v: &String| {
            if v.contains('@') {
                Ok(())
            } else {
                Err("Must contain @".into())
            }
        });
    form.register_field("age", 30u32)
        .with_validator(|&age: &u32| {
            if age >= 18 {
                Ok(())
            } else {
                Err("Must be 18+".into())
            }
        });
    form.register_field("password", String::from("secret123"))
        .with_validator(|v: &String| {
            if v.len() >= 8 {
                Ok(())
            } else {
                Err("Min 8 chars".into())
            }
        });
    form.register_field("confirm_password", String::from("secret123"));
    form.add_cross_validator("confirm_password", |f: &FormState| {
        let pw = f.value::<String>("password").map(String::as_str).unwrap_or("");
        let cf = f
            .value::<String>("confirm_password")
            .map(String::as_str)
            .unwrap_or("");
        if pw == cf {
            Ok(())
        } else {
            Err("Passwords must match".into())
        }
    });

    // Clean initial state allows Android Predictive Back pop.
    assert!(!form.is_dirty());
    assert!(form.can_pop());
    assert!(form.is_valid());
    assert_eq!(form.first_error_field(), None);

    // Typing an invalid email makes form dirty and records raw error, but hides visible_error
    // until blur (`touch`) or `submit`.
    assert!(form.set_value("email", String::from("not-an-email")));
    assert!(form.is_dirty());
    assert!(!form.can_pop());
    assert_eq!(form.dirty_fields(), vec!["email"]);
    assert_eq!(form.error("email"), Some("Must contain @"));
    assert_eq!(form.visible_error("email"), None);

    // Blur reveals visible error.
    form.touch("email");
    assert_eq!(form.visible_error("email"), Some("Must contain @"));
    assert_eq!(form.first_error_field(), Some("email"));

    // Cross-field validation fires when password changes to mismatch confirm_password.
    form.set_value("password", String::from("new-password-99"));
    assert_eq!(form.error("confirm_password"), Some("Passwords must match"));
    assert_eq!(form.visible_error("confirm_password"), None); // untouched so far

    // Fix email -> first_error_field advances in registration order to confirm_password.
    form.set_value("email", String::from("grace@nve.dev"));
    assert_eq!(form.first_error_field(), Some("confirm_password"));

    // Introduce age error -> age is registered BEFORE confirm_password, so first_error_field is age!
    form.set_value("age", 15u32);
    assert_eq!(form.first_error_field(), Some("age"));
    assert_eq!(form.first_visible_error_field(), None);

    // Submitting reveals every field error and returns false while invalid.
    assert!(!form.submit());
    assert!(form.is_submitted());
    assert_eq!(form.visible_error("age"), Some("Must be 18+"));
    assert_eq!(
        form.visible_error("confirm_password"),
        Some("Passwords must match")
    );
    assert_eq!(form.first_visible_error_field(), Some("age"));

    // Fix all errors -> submit succeeds; commit_clean_baseline clears dirty state.
    form.set_value("age", 28u32);
    form.set_value("confirm_password", String::from("new-password-99"));
    assert!(form.submit());
    assert!(form.is_dirty());
    form.commit_clean_baseline();
    assert!(!form.is_dirty());
    assert!(form.can_pop());

    // Further edits can be rolled back via reset_to_clean.
    form.set_value("email", String::from("bad"));
    assert!(form.is_dirty());
    form.reset_to_clean();
    assert!(!form.is_dirty());
    assert!(form.can_pop());
    assert_eq!(
        form.value::<String>("email").map(String::as_str),
        Some("grace@nve.dev")
    );
    assert_eq!(form.error("email"), None);
}

#[test]
fn form_stale_async_validation_results_are_dropped() {
    let mut form = FormState::new();
    form.register_field("username", String::from("ada"));
    form.touch("username");

    let tok1 = form.begin_async_validation("username").unwrap();
    assert!(form.is_validating());

    // User types a new character -> invalidates tok1.
    form.set_value("username", String::from("adalovelace"));
    let tok2 = form.begin_async_validation("username").unwrap();
    assert_ne!(tok1.generation, tok2.generation);

    // Stale tok1 arrives late with an error -> MUST be rejected!
    assert!(!form.apply_async_result(tok1, Err("Username taken".into())));
    assert_eq!(form.error("username"), None);
    assert!(form.is_validating());

    // Fresh tok2 arrives with Ok(()) -> accepted and clears validating flag.
    assert!(form.apply_async_result(tok2, Ok(())));
    assert!(!form.is_validating());
    assert_eq!(form.error("username"), None);

    // Another async check fails -> surfaces error, then reset_to_clean invalidates a pending check.
    let tok3 = form.begin_async_validation("username").unwrap();
    assert!(form.apply_async_result(tok3, Err("Reserved handle".into())));
    assert_eq!(form.visible_error("username"), Some("Reserved handle"));

    let tok4 = form.begin_async_validation("username").unwrap();
    form.reset_to_clean();
    assert!(!form.is_validating());
    assert!(!form.apply_async_result(tok4, Err("Late error".into())));
    assert_eq!(form.error("username"), None);

    // Standalone FieldState<T> sanity check.
    let mut standalone = FieldState::new("code", 10i32);
    assert!(!standalone.is_dirty());
    standalone.set_value(20);
    assert!(standalone.is_dirty());
    assert!(!standalone.validate_sync(|&v| if v < 15 { Ok(()) } else { Err("Too high".into()) }));
    assert_eq!(standalone.visible_error(), None);
    standalone.mark_touched();
    assert_eq!(standalone.visible_error(), Some("Too high"));
}

// ---------------------------------------------------------------------------
// 3. Transactional & Burst-Coalescing UndoRedoStack<T>
// ---------------------------------------------------------------------------

#[test]
fn history_bounded_eviction_and_redo_branch_clearing() {
    let mut stack = UndoRedoStack::new(0u32, 4);
    assert_eq!(stack.len(), 1);
    assert!(!stack.can_undo());
    assert!(!stack.can_redo());

    for v in 1..=10u32 {
        stack.push(v);
        assert!(stack.len() <= 4);
    }
    assert_eq!(stack.len(), 4);
    assert_eq!(*stack.current(), 10);
    assert_eq!(stack.undo_count(), 3);
    assert_eq!(stack.redo_count(), 0);

    // Oldest entries (0..=6) were evicted; stack holds [7, 8, 9, 10].
    assert_eq!(stack.undo(), Some(&9));
    assert_eq!(stack.undo(), Some(&8));
    assert_eq!(stack.undo(), Some(&7));
    assert_eq!(stack.undo(), None);
    assert_eq!(stack.redo(), Some(&8));

    // Pushing a new state truncates the redo branch [9, 10].
    stack.push(99);
    assert!(!stack.can_redo());
    assert_eq!(stack.redo(), None);
    assert_eq!(*stack.current(), 99);
    assert_eq!(stack.undo(), Some(&8));
    assert_eq!(stack.redo(), Some(&99));

    // Edge case: max_entries == 0 clamps to 1.
    let mut tiny = UndoRedoStack::new("a", 0);
    assert_eq!(tiny.max_entries(), 1);
    tiny.push("b");
    assert_eq!(tiny.len(), 1);
    assert_eq!(*tiny.current(), "b");
    assert!(!tiny.can_undo());
}

#[test]
fn history_burst_coalescing_window_and_keys() {
    let mut stack = UndoRedoStack::new(String::new(), 32);

    // Rapid typing burst within 400ms coalesces into a single undoable step.
    stack.push_coalesced("h".into(), "typing", 1000, 400);
    stack.push_coalesced("he".into(), "typing", 1120, 400);
    stack.push_coalesced("hel".into(), "typing", 1300, 400);
    stack.push_coalesced("hello".into(), "typing", 1500, 400);
    assert_eq!(stack.len(), 2);
    assert_eq!(stack.current(), "hello");

    // Pause longer than window (2100 - 1500 = 600 > 400) starts a new undo entry.
    stack.push_coalesced("hello world".into(), "typing", 2100, 400);
    assert_eq!(stack.len(), 3);

    // Different coalesce key starts a new undo entry even within window.
    stack.push_coalesced("HELLO WORLD".into(), "format", 2150, 400);
    assert_eq!(stack.len(), 4);

    // Non-monotonic timestamp starts a new entry instead of corrupting previous.
    stack.push_coalesced("HELLO WORLD!".into(), "format", 2000, 400);
    assert_eq!(stack.len(), 5);

    assert_eq!(stack.undo().map(String::as_str), Some("HELLO WORLD"));
    assert_eq!(stack.undo().map(String::as_str), Some("hello world"));
    assert_eq!(stack.undo().map(String::as_str), Some("hello"));
    assert_eq!(stack.undo().map(String::as_str), Some(""));
    assert_eq!(stack.undo(), None);
}

#[test]
fn history_nested_transaction_groups_and_abort_rollback() {
    let mut stack = UndoRedoStack::new(10i32, 16);

    // Outer commit collapses multiple pushes into one undo step.
    stack.begin_group();
    assert!(stack.in_group());
    assert!(!stack.can_undo());
    stack.push(20);
    stack.push(30);
    assert_eq!(*stack.current(), 30);
    assert_eq!(stack.len(), 1);
    assert!(stack.commit_group());
    assert!(!stack.in_group());
    assert_eq!(stack.len(), 2);
    assert_eq!(*stack.current(), 30);

    // Nested group: inner aborts cleanly back to outer working state, then outer commits!
    stack.begin_group();
    stack.push(40);
    stack.begin_group();
    assert_eq!(stack.group_depth(), 2);
    stack.push(50);
    stack.push(60);
    assert_eq!(*stack.current(), 60);
    assert!(stack.abort_group());
    assert_eq!(stack.group_depth(), 1);
    assert_eq!(*stack.current(), 40);
    assert!(stack.commit_group());
    assert_eq!(stack.len(), 3);
    assert_eq!(*stack.current(), 40);

    // Nested group: inner commits into outer, then outer aborts -> rolls back ENTIRE transaction!
    stack.begin_group();
    stack.push(70);
    stack.begin_group();
    stack.push(80);
    assert!(stack.commit_group());
    assert_eq!(*stack.current(), 80);
    assert!(stack.abort_group());
    assert!(!stack.in_group());
    assert_eq!(*stack.current(), 40);
    assert_eq!(stack.len(), 3);

    // Empty group commits as a no-op; stray commit/abort returns false.
    stack.begin_group();
    assert!(stack.commit_group());
    assert_eq!(stack.len(), 3);
    assert!(!stack.commit_group());
    assert!(!stack.abort_group());

    // Undo verifies exact history [10, 30, 40].
    assert_eq!(stack.undo(), Some(&30));
    assert_eq!(stack.undo(), Some(&10));
    assert_eq!(stack.undo(), None);
}
