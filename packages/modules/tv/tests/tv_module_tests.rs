use nve_tv::{
    FocusDirection, NodeId, RemoteAction, ScrollRegion, SpatialFocusEngine, SpatialFocusGroupId,
    SpatialFocusNode, SpatialRect, TvOverscanInsets,
};

fn n(index: u32) -> NodeId {
    NodeId::new(index, 0)
}

#[test]
fn spatial_focus_navigates_grid_skips_disabled_and_respects_cone_overlap() {
    let mut engine = SpatialFocusEngine::new();
    // 2x3 grid where node 2 (top-middle) is disabled.
    for row in 0..2u32 {
        for col in 0..3u32 {
            let id = row * 3 + col + 1;
            let rect = SpatialRect::new(col as f32 * 220.0, row as f32 * 140.0, 200.0, 120.0);
            engine.register(SpatialFocusNode::new(n(id), rect).with_disabled(id == 2));
        }
    }

    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(1)));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(3)));
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(n(6)));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(n(5)));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(n(4)));
    assert_eq!(engine.move_focus(FocusDirection::Up), Some(n(1)));
}

#[test]
fn group_memory_and_modal_focus_trap_work_deterministically() {
    let sidebar = SpatialFocusGroupId(1);
    let shelf = SpatialFocusGroupId(2);
    let modal = SpatialFocusGroupId(99);

    let mut engine = SpatialFocusEngine::new();
    engine.register(
        SpatialFocusNode::new(n(10), SpatialRect::new(20.0, 40.0, 180.0, 60.0)).with_group(sidebar),
    );
    engine.register(
        SpatialFocusNode::new(n(11), SpatialRect::new(20.0, 120.0, 180.0, 60.0))
            .with_group(sidebar),
    );
    engine.register(
        SpatialFocusNode::new(n(20), SpatialRect::new(240.0, 40.0, 200.0, 100.0)).with_group(shelf),
    );
    engine.register(
        SpatialFocusNode::new(n(21), SpatialRect::new(460.0, 40.0, 200.0, 100.0)).with_group(shelf),
    );

    assert!(engine.set_focus(n(21)));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(n(20)));
    assert!(engine.set_focus(n(21)));
    // Jump into sidebar, then jump back into shelf -> restores remembered card 21!
    assert!(engine.set_focus(n(11)));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(21)));
    assert_eq!(engine.last_focused_in_group(shelf), Some(n(21)));

    // Register modal trap and ensure focus stays trapped inside modal.
    engine.register(
        SpatialFocusNode::new(n(90), SpatialRect::new(300.0, 300.0, 140.0, 60.0))
            .with_group(modal)
            .with_traps_focus(true),
    );
    engine.register(
        SpatialFocusNode::new(n(91), SpatialRect::new(460.0, 300.0, 140.0, 60.0))
            .with_group(modal)
            .with_traps_focus(true),
    );
    assert!(engine.set_focus(n(90)));
    assert_eq!(engine.move_focus(FocusDirection::Up), None);
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(91)));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(n(90)));
}

#[test]
fn wrap_axis_and_scroll_into_view_handle_boundaries() {
    let mut engine = SpatialFocusEngine::new().with_wrap_axis(true);
    engine.register(SpatialFocusNode::new(
        n(1),
        SpatialRect::new(0.0, 0.0, 100.0, 80.0),
    ));
    engine.register(SpatialFocusNode::new(
        n(2),
        SpatialRect::new(120.0, 0.0, 100.0, 80.0),
    ));
    engine.register(SpatialFocusNode::new(
        n(3),
        SpatialRect::new(240.0, 0.0, 100.0, 80.0),
    ));

    assert!(engine.set_focus(n(3)));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(1)));
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(n(3)));

    let adj = engine
        .compute_scroll_into_view(
            n(3),
            ScrollRegion {
                viewport_origin_x: 0.0,
                viewport_origin_y: 0.0,
                viewport_width: 200.0,
                viewport_height: 100.0,
                content_width: 400.0,
                content_height: 100.0,
                margin_px: 20.0,
            },
        )
        .expect("scroll adjustment");
    assert!(adj.changed);
    assert!((adj.target_offset_x - 160.0).abs() < 1e-3);
}

#[test]
fn overscan_insets_and_remote_action_keys_match_cross_platform_spec() {
    let std1080 = TvOverscanInsets::for_viewport_px(
        1920,
        1080,
        2.0,
        TvOverscanInsets::DEFAULT_OVERSCAN_RATIO,
    );
    assert_eq!(std1080, TvOverscanInsets::STANDARD_1080P_DP);
    assert!((std1080.safe_width(960.0) - 864.0).abs() < 1e-4);
    assert!((std1080.safe_height(540.0) - 486.0).abs() < 1e-4);

    let safe = TvOverscanInsets::STANDARD_TVOS_1080P.safe_rect(1920.0, 1080.0);
    assert_eq!(safe, SpatialRect::new(96.0, 54.0, 1728.0, 972.0));

    let nan_insets = TvOverscanInsets::for_viewport(f32::NAN, -50.0, f32::INFINITY);
    assert_eq!(
        nan_insets,
        TvOverscanInsets {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0
        }
    );

    assert_eq!(
        RemoteAction::Move(FocusDirection::Right).key_name(),
        "ArrowRight"
    );
    assert_eq!(
        RemoteAction::from_key_name("ArrowRight"),
        Some(RemoteAction::Move(FocusDirection::Right))
    );
    assert_eq!(
        RemoteAction::from_key_name("ChannelUp"),
        Some(RemoteAction::ChannelUp)
    );
    assert_eq!(
        RemoteAction::from_key_name("ChannelDown"),
        Some(RemoteAction::ChannelDown)
    );
    assert_eq!(RemoteAction::from_key_name("UnknownKey"), None);
}
