//! Deep edge-case unit tests for 2D geometric `SpatialFocusEngine`.

use nve_ui::{
    FocusDirection, RemoteAction, ScrollRegion, SpatialFocusEngine, SpatialFocusGroupId,
    SpatialFocusNode, SpatialRect,
};

#[test]
fn spatial_rect_geometry_and_validation() {
    let r = SpatialRect::new(10.0, 20.0, 100.0, 50.0);
    assert!(r.is_valid());
    assert_eq!(r.right(), 110.0);
    assert_eq!(r.bottom(), 70.0);
    assert_eq!(r.center(), (60.0, 45.0));
    assert!(r.contains_point(10.0, 20.0));
    assert!(r.contains_point(110.0, 70.0));
    assert!(r.contains_point(60.0, 45.0));
    assert!(!r.contains_point(9.9, 45.0));
    assert!(!r.contains_point(f32::NAN, 45.0));

    let overlap = SpatialRect::new(100.0, 60.0, 40.0, 40.0);
    let touching = SpatialRect::new(110.0, 20.0, 40.0, 50.0);
    assert!(r.intersects(&overlap));
    assert!(!r.intersects(&touching));

    // Zero-area and non-finite rects are invalid
    assert!(!SpatialRect::new(0.0, 0.0, 0.0, 50.0).is_valid());
    assert!(!SpatialRect::new(0.0, 0.0, 50.0, -10.0).is_valid());
    assert!(!SpatialRect::new(f32::NAN, 0.0, 50.0, 50.0).is_valid());
    assert!(!SpatialRect::new(0.0, 0.0, f32::INFINITY, 50.0).is_valid());
    assert!(!SpatialRect::new(0.0, 0.0, 0.0, 50.0).intersects(&r));
    assert!(!SpatialRect::new(0.0, 0.0, f32::NAN, 50.0).contains_point(0.0, 0.0));
}

#[test]
fn uniform_grid_navigation_all_four_directions_and_skipping_disabled() {
    let mut engine = SpatialFocusEngine::new();
    // 3x3 grid: id = row * 3 + col + 1
    // Row 0: 1, 2(disabled), 3
    // Row 1: 4, 5, 6
    // Row 2: 7, 8, 9
    for row in 0..3u64 {
        for col in 0..3u64 {
            let id = row * 3 + col + 1;
            let rect = SpatialRect::new(col as f32 * 220.0, row as f32 * 140.0, 200.0, 120.0);
            engine.register(SpatialFocusNode::new(id, rect).with_disabled(id == 2));
        }
    }

    assert!(!engine.set_focus(2), "disabled node cannot receive focus");
    assert!(engine.set_focus(1));

    // Moving Right from 1 skips disabled 2 and lands on 3 (direct-line cone beats diagonal 5)
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(3));
    // Edge of grid with wrap_axis=false stays at 3
    assert_eq!(engine.move_focus(FocusDirection::Right), None);
    assert_eq!(engine.focused(), Some(3));

    assert_eq!(engine.move_focus(FocusDirection::Down), Some(6));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(5));
    // Moving Up from 5 skips disabled 2 (no node directly above in col 1 -> falls back to nearest)
    // First test normal Down -> 8, Left -> 7, Up -> 4, Up -> 1
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(8));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(7));
    assert_eq!(engine.move_focus(FocusDirection::Up), Some(4));
    assert_eq!(engine.move_focus(FocusDirection::Up), Some(1));
    assert_eq!(engine.move_focus(FocusDirection::Up), None);

    // Enable wrap_axis: moving Left from 1 wraps to 3; moving Up from 1 wraps to 7
    engine.set_wrap_axis(true);
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(3));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(1));
    assert_eq!(engine.move_focus(FocusDirection::Up), Some(7));
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(1));
}

#[test]
fn staggered_masonry_and_wide_hero_above_multi_column_shelf() {
    let mut engine = SpatialFocusEngine::new();
    // Wide hero banner spanning full 940px width
    engine.register(SpatialFocusNode::new(
        100,
        SpatialRect::new(0.0, 0.0, 940.0, 260.0),
    ));
    // 3-column shelf directly below hero
    engine.register(SpatialFocusNode::new(
        201,
        SpatialRect::new(0.0, 290.0, 300.0, 180.0),
    ));
    engine.register(SpatialFocusNode::new(
        202,
        SpatialRect::new(320.0, 290.0, 300.0, 180.0),
    ));
    engine.register(SpatialFocusNode::new(
        203,
        SpatialRect::new(640.0, 290.0, 300.0, 180.0),
    ));
    // Staggered masonry row below the shelf (offset horizontally and varying heights)
    engine.register(SpatialFocusNode::new(
        301,
        SpatialRect::new(0.0, 490.0, 300.0, 260.0),
    ));
    engine.register(SpatialFocusNode::new(
        302,
        SpatialRect::new(320.0, 540.0, 300.0, 160.0),
    ));
    engine.register(SpatialFocusNode::new(
        303,
        SpatialRect::new(640.0, 500.0, 300.0, 220.0),
    ));

    // From wide hero center (470, 130), moving Down lands on center card 202 (center x=470)
    assert!(engine.set_focus(100));
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(202));
    // Move left to 201, then Up back to wide hero 100
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(201));
    assert_eq!(engine.move_focus(FocusDirection::Up), Some(100));

    // Navigate staggered masonry right -> 301 -> 302 -> 303
    assert!(engine.set_focus(301));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(302));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(303));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(302));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(301));
}

#[test]
fn empty_single_node_and_nan_zero_size_guards() {
    let mut engine = SpatialFocusEngine::new();
    assert_eq!(engine.move_focus(FocusDirection::Right), None);
    assert_eq!(engine.focused(), None);

    // Register invalid NaN / zero-size / negative nodes only
    engine.register(SpatialFocusNode::new(
        1,
        SpatialRect::new(f32::NAN, 0.0, 100.0, 100.0),
    ));
    engine.register(SpatialFocusNode::new(
        2,
        SpatialRect::new(0.0, 0.0, 0.0, 100.0),
    ));
    engine.register(SpatialFocusNode::new(
        3,
        SpatialRect::new(0.0, 0.0, 100.0, -5.0),
    ));
    assert!(!engine.set_focus(1));
    assert!(!engine.set_focus(2));
    assert!(!engine.set_focus(3));
    assert_eq!(engine.move_focus(FocusDirection::Down), None);

    // Single valid node: first move_focus focuses it; subsequent moves return None even with wrap
    engine.register(SpatialFocusNode::new(
        10,
        SpatialRect::new(50.0, 50.0, 120.0, 80.0),
    ));
    engine.set_wrap_axis(true);
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(10));
    assert_eq!(engine.move_focus(FocusDirection::Right), None);
    assert_eq!(engine.move_focus(FocusDirection::Left), None);
    assert_eq!(engine.focused(), Some(10));
}

#[test]
fn focus_group_memory_restores_exact_card_between_sidebar_and_shelf() {
    let sidebar = SpatialFocusGroupId(1);
    let shelf = SpatialFocusGroupId(2);
    let mut engine = SpatialFocusEngine::new();

    // Vertical sidebar on left (x = 0..180)
    for idx in 0..3u64 {
        engine.register(
            SpatialFocusNode::new(
                10 + idx,
                SpatialRect::new(0.0, 100.0 + idx as f32 * 90.0, 180.0, 72.0),
            )
            .with_group(sidebar),
        );
    }

    // Horizontal content shelf on right (x = 240..)
    for idx in 0..4u64 {
        engine.register(
            SpatialFocusNode::new(
                100 + idx,
                SpatialRect::new(240.0 + idx as f32 * 240.0, 100.0, 210.0, 280.0),
            )
            .with_group(shelf),
        );
    }

    // Focus third card (102) in shelf and sidebar item 11
    assert!(engine.set_focus(11));
    assert_eq!(engine.last_focused_in_group(sidebar), Some(11));
    assert!(engine.set_focus(102));
    assert_eq!(engine.last_focused_in_group(shelf), Some(102));

    // Move within shelf left: 102 -> 101 -> 100, wait: let's test direct cross-group jump!
    // Start at card 102, move up/down? Put a second row or jump to sidebar via set_focus / Up.
    // Specifically test moving Left from a vertical rail (where card 102 is remembered) into sidebar
    // and back Right restores 102!
    let shelf_row2 = SpatialFocusGroupId(3);
    for idx in 0..4u64 {
        engine.register(
            SpatialFocusNode::new(
                200 + idx,
                SpatialRect::new(240.0 + idx as f32 * 240.0, 420.0, 210.0, 280.0),
            )
            .with_group(shelf_row2),
        );
    }

    // Focus 102 in shelf, move Down into shelf_row2 (202), walk left to sidebar (12), move Up to 10,
    // then move Right back into shelf -> geometrically hits 100, memory restores exact card 102!
    assert!(engine.set_focus(102));
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(202));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(201));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(200));
    // Moving Left from 200 enters sidebar -> restores sidebar's remembered item 11!
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(11));
    // Moving Right from sidebar 11 enters shelf -> geometrically hits 100, restores remembered 102!
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(102));

    // If remembered card 102 becomes disabled, fallback gracefully to geometric nearest (100)
    engine.register(
        SpatialFocusNode::new(
            102,
            SpatialRect::new(240.0 + 2.0 * 240.0, 100.0, 210.0, 280.0),
        )
        .with_group(shelf)
        .with_disabled(true),
    );
    assert!(engine.set_focus(10));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(100));
}

#[test]
fn modal_focus_trap_prevents_escaping_to_background_cards() {
    let bg_group = SpatialFocusGroupId(1);
    let modal_group = SpatialFocusGroupId(99);
    let mut engine = SpatialFocusEngine::new();

    // Background cards surrounding the modal on all 4 sides
    engine.register(
        SpatialFocusNode::new(1, SpatialRect::new(400.0, 40.0, 200.0, 100.0)).with_group(bg_group),
    );
    engine.register(
        SpatialFocusNode::new(2, SpatialRect::new(400.0, 700.0, 200.0, 100.0))
            .with_group(bg_group),
    );
    engine.register(
        SpatialFocusNode::new(3, SpatialRect::new(40.0, 350.0, 200.0, 100.0)).with_group(bg_group),
    );
    engine.register(
        SpatialFocusNode::new(4, SpatialRect::new(900.0, 350.0, 200.0, 100.0))
            .with_group(bg_group),
    );

    // Modal dialog with 2 buttons trapping focus
    engine.register(
        SpatialFocusNode::new(50, SpatialRect::new(360.0, 360.0, 140.0, 64.0))
            .with_group(modal_group)
            .with_traps_focus(true),
    );
    engine.register(
        SpatialFocusNode::new(51, SpatialRect::new(520.0, 360.0, 140.0, 64.0))
            .with_group(modal_group)
            .with_traps_focus(true),
    );

    assert!(engine.set_focus(50));
    // Can move between modal buttons
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(51));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(50));

    // Cannot escape to background cards in any direction
    assert_eq!(engine.move_focus(FocusDirection::Up), None);
    assert_eq!(engine.move_focus(FocusDirection::Down), None);
    assert_eq!(engine.move_focus(FocusDirection::Left), None);
    assert_eq!(engine.focused(), Some(50));

    // Even with wrap_axis=true, wrapping stays strictly inside the modal group
    engine.set_wrap_axis(true);
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(51));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(50));
}

#[test]
fn compute_scroll_into_view_clamping_margins_and_oversized_items() {
    let mut engine = SpatialFocusEngine::new();
    // Node near top
    engine.register(SpatialFocusNode::new(
        1,
        SpatialRect::new(20.0, 10.0, 200.0, 120.0),
    ));
    // Node in middle (already visible)
    engine.register(SpatialFocusNode::new(
        2,
        SpatialRect::new(300.0, 350.0, 200.0, 120.0),
    ));
    // Node near bottom-right edge
    engine.register(SpatialFocusNode::new(
        3,
        SpatialRect::new(2350.0, 1900.0, 200.0, 120.0),
    ));
    // Node just past bottom margin
    engine.register(SpatialFocusNode::new(
        4,
        SpatialRect::new(300.0, 940.0, 200.0, 120.0),
    ));
    // Oversized hero node taller & wider than viewport
    engine.register(SpatialFocusNode::new(
        5,
        SpatialRect::new(120.0, 600.0, 1400.0, 900.0),
    ));

    let region = ScrollRegion {
        viewport_origin_x: 200.0,
        viewport_origin_y: 200.0,
        viewport_width: 1000.0,
        viewport_height: 800.0,
        content_width: 2500.0,
        content_height: 2000.0,
        margin_px: 60.0,
    };

    // Already inside viewport with full 60px margin -> no change
    let mid = engine.compute_scroll_into_view(2, region).unwrap();
    assert!(!mid.changed);
    assert_eq!(mid.target_offset_x, 200.0);
    assert_eq!(mid.target_offset_y, 200.0);

    // Near top-left -> clamps to (0.0, 0.0)
    let top = engine.compute_scroll_into_view(1, region).unwrap();
    assert!(top.changed);
    assert_eq!(top.target_offset_x, 0.0);
    assert_eq!(top.target_offset_y, 0.0);

    // Near bottom-right -> clamps to max_x=1500, max_y=1200
    let bot = engine.compute_scroll_into_view(3, region).unwrap();
    assert!(bot.changed);
    assert_eq!(bot.target_offset_x, 1500.0);
    assert_eq!(bot.target_offset_y, 1200.0);

    // Just past bottom safe margin: node bottom=1060 + margin 60 = 1120 -> target_y = 1120 - 800 = 320
    let margin_push = engine.compute_scroll_into_view(4, region).unwrap();
    assert!(margin_push.changed);
    assert_eq!(margin_push.target_offset_x, 200.0);
    assert_eq!(margin_push.target_offset_y, 320.0);

    // Oversized hero node taller & wider than viewport:
    // X=[120, 1520] already spans viewport [200, 1200] -> stays at 200.0 without jitter;
    // Y=[600, 1500] does not span [200, 1000] -> aligns leading edge to 600.0.
    let oversized = engine.compute_scroll_into_view(5, region).unwrap();
    assert!(oversized.changed);
    assert_eq!(oversized.target_offset_x, 200.0);
    assert_eq!(oversized.target_offset_y, 600.0);

    // Oversized node starting beyond viewport aligns leading edge cleanly clamped to max_x
    engine.register(SpatialFocusNode::new(
        6,
        SpatialRect::new(1300.0, 1300.0, 1400.0, 900.0),
    ));
    let oversized_far = engine.compute_scroll_into_view(6, region).unwrap();
    assert!(oversized_far.changed);
    assert_eq!(oversized_far.target_offset_x, 1300.0);
    assert_eq!(oversized_far.target_offset_y, 1200.0);

    // Unknown node & invalid viewport return None
    assert!(engine.compute_scroll_into_view(999, region).is_none());
    assert!(engine
        .compute_scroll_into_view(
            2,
            ScrollRegion {
                viewport_width: 0.0,
                ..region
            }
        )
        .is_none());
}

#[test]
fn key_name_to_remote_action_mapping_and_handle_action() {
    assert_eq!(
        SpatialFocusEngine::from_key_name("ArrowUp"),
        Some(RemoteAction::Move(FocusDirection::Up))
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("ArrowDown"),
        Some(RemoteAction::Move(FocusDirection::Down))
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("ArrowLeft"),
        Some(RemoteAction::Move(FocusDirection::Left))
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("ArrowRight"),
        Some(RemoteAction::Move(FocusDirection::Right))
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("Enter"),
        Some(RemoteAction::Select)
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("Select"),
        Some(RemoteAction::Select)
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("Escape"),
        Some(RemoteAction::Back)
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("Back"),
        Some(RemoteAction::Back)
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("GoBack"),
        Some(RemoteAction::Back)
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("MediaPlayPause"),
        Some(RemoteAction::PlayPause)
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("ContextMenu"),
        Some(RemoteAction::ContextMenu)
    );
    assert_eq!(
        SpatialFocusEngine::from_key_name("Menu"),
        Some(RemoteAction::Menu)
    );
    assert_eq!(SpatialFocusEngine::from_key_name("KeyA"), None);
    assert_eq!(SpatialFocusEngine::from_key_name(""), None);

    let mut engine = SpatialFocusEngine::new();
    engine.register(SpatialFocusNode::new(
        1,
        SpatialRect::new(0.0, 0.0, 100.0, 100.0),
    ));
    engine.register(SpatialFocusNode::new(
        2,
        SpatialRect::new(120.0, 0.0, 100.0, 100.0),
    ));
    assert!(engine.set_focus(1));
    assert_eq!(
        engine.handle_action(RemoteAction::from_key_name("ArrowRight").unwrap()),
        Some(2)
    );
    assert_eq!(engine.handle_action(RemoteAction::Select), Some(2));
}
