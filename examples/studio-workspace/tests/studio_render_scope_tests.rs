//! Re-render scope guards: an interaction re-renders only the components that read what it
//! changed (leaf subscriptions), and host events update the mounted surface in place instead of
//! rebuilding it.

mod common;

use std::collections::BTreeMap;

use common::{MultiBackendHarness, find_in_store, register_studio_tables};
use nve_abi::{
    EventBatchBuf, FrameId, NveMapEntry, NvePropsRef, NveValue, SurfaceId, TimestampUs,
    ROOT_NODE_ID,
};
use nve_dioxus::render_probe::RenderProbe;
use nve_engine::NveApp;
use nve_system_env::{ColorScheme, ThemeSnapshot};

/// Asserts the components rendered by one interaction are exactly `expected` (name, count).
fn assert_renders(label: &str, renders: BTreeMap<&'static str, u32>, expected: &[(&str, u32)]) {
    let expected: BTreeMap<&str, u32> = expected.iter().copied().collect();
    let actual: BTreeMap<&str, u32> = renders.into_iter().collect();
    assert_eq!(actual, expected, "{label}: re-rendered components");
}

#[test]
fn typing_creating_and_panel_toggles_rerender_only_their_leaves() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);
    let probe = RenderProbe::start();

    // Every interaction bumps the frame counter shown by the header pill and the time-travel card.
    h.type_text("board-new-task-input", "Ship");
    assert_renders(
        "type board draft",
        probe.take(),
        &[
            ("BoardComposer", 1),
            ("TimeTravelCard", 1),
            ("WorkspaceHeader", 1),
        ],
    );

    // A new task: the board list, its one new card, the board tab's count, and the history.
    h.press("btn-new-task");
    assert_renders(
        "create task",
        probe.take(),
        &[
            ("BoardView", 1),
            ("NavTab", 1),
            ("NavTabList", 1),
            ("TaskCard", 1),
            ("TimeTravelCard", 1),
            ("WorkspaceHeader", 1),
        ],
    );

    // Closing the docked panel: the shell drops it, the toggle repaints; no pane re-renders.
    h.press("btn-toggle-studio");
    assert!(h.find_by_test_id("studio-panel").is_none());
    assert_renders(
        "close studio panel",
        probe.take(),
        &[
            ("StudioPanelToggle", 1),
            ("StudioShell", 1),
            ("WorkspaceHeader", 1),
        ],
    );

    h.press("nav-engineering");
    let _ = probe.take();
    h.type_text("chat-composer-input", "Hello");
    assert_renders(
        "type channel draft",
        probe.take(),
        &[("ChannelComposer", 1), ("WorkspaceHeader", 1)],
    );

    h.press("nav-docs");
    h.press("doc-block-edit-1");
    let _ = probe.take();
    h.type_text("doc-block-input-1", "Edited");
    assert_renders(
        "type doc block edit",
        probe.take(),
        &[("DocBlockEditor", 1), ("WorkspaceHeader", 1)],
    );
    h.type_text("doc-new-block-input", "New");
    assert_renders(
        "type new doc block",
        probe.take(),
        &[("DocComposer", 1), ("WorkspaceHeader", 1)],
    );
}

#[test]
fn host_scroll_and_theme_events_update_the_mounted_surface_in_place() {
    register_studio_tables();
    let mut module = common::headless_module(SurfaceId::ROOT);
    let entries = [NveMapEntry::new("view", NveValue::from_str("stream"))];
    // SAFETY: `entries` outlives `props`.
    let props = unsafe { NvePropsRef::from_slice(&entries) };
    assert!(module.set_props(props));
    let _ = module.render(FrameId(1));
    let root = find_in_store(module.surface().store(), "workspace-root").expect("workspace-root");
    let sidebar =
        find_in_store(module.surface().store(), "desktop-sidebar").expect("desktop-sidebar");
    let probe = RenderProbe::start();

    let mut scroll = EventBatchBuf::new();
    scroll.push_scroll(
        SurfaceId::ROOT,
        ROOT_NODE_ID,
        TimestampUs(1_000),
        0.0,
        4_000.0,
        960.0,
        2_000_000.0,
    );
    assert!(module.dispatch_events(scroll.as_validated()));
    let _ = module.render(FrameId(2));
    assert_eq!(
        module.state_snapshot().current.stream.scroll_offset_y,
        4_000
    );
    assert_renders(
        "host scroll",
        probe.take(),
        &[
            ("Stream100kPanel", 1),
            ("TimeTravelCard", 1),
            ("WorkspaceHeader", 1),
        ],
    );

    let os = module.state_snapshot().system_env.theme;
    let flipped = if os.color_scheme == Some(ColorScheme::Dark) {
        ColorScheme::Light
    } else {
        ColorScheme::Dark
    };
    let theme = ThemeSnapshot {
        color_scheme: Some(flipped),
        ..os
    };
    assert!(module.dispatch_host_events(&theme.to_repr_c_bytes()));
    let _ = module.render(FrameId(3));
    let renders = probe.take();
    assert!(
        renders.contains_key("StudioShell"),
        "the theme change must repaint the shell: {renders:?}"
    );
    assert!(
        !renders.contains_key("StudioWorkspaceApp"),
        "host events must not remount the surface root: {renders:?}"
    );

    let store = module.surface().store();
    assert_eq!(
        find_in_store(store, "workspace-root"),
        Some(root),
        "root node survives host events"
    );
    assert_eq!(
        find_in_store(store, "desktop-sidebar"),
        Some(sidebar),
        "pane nodes survive host events"
    );
    assert_eq!(module.state_snapshot().color_scheme(), flipped);
}
