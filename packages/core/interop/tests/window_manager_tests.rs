use nve_interop::{
    FrameworkKind, InteropRuntime, SurfaceDescriptor, SurfaceEnvironment, SurfacePresentation,
    WindowCommand, WindowDescriptor, WindowError, WindowFlags, WindowLifecycleEvent,
    WindowLifecycleState, WindowManager, WindowRect, WindowState,
};

#[test]
fn surface_descriptor_defaults_to_embedded_and_supports_window_presentations() {
    let mut rt = InteropRuntime::new();
    let embedded = rt
        .surfaces
        .register_root(FrameworkKind::Nve, "EmbeddedCard", SurfaceEnvironment::default())
        .unwrap();
    assert_eq!(rt.surfaces.get(embedded).unwrap().presentation, SurfacePresentation::Embedded);
    assert!(rt.surfaces.get(embedded).unwrap().presentation.is_embedded());

    let primary_desc = WindowDescriptor::new("Main Studio", 1280.0, 800.0)
        .with_bounds(WindowRect::new(100.0, 80.0, 1280.0, 800.0))
        .with_min_size(640.0, 480.0);
    let primary_surface = rt
        .surfaces
        .register_surface(
            SurfaceDescriptor::new(FrameworkKind::Nve, "PrimaryWindow")
                .with_presentation(SurfacePresentation::PrimaryWindow(primary_desc.clone())),
        )
        .unwrap();
    assert_eq!(
        rt.surfaces.get(primary_surface).unwrap().presentation,
        SurfacePresentation::PrimaryWindow(primary_desc.clone())
    );

    let handle = rt
        .windows
        .open_window(primary_surface, SurfacePresentation::PrimaryWindow(primary_desc))
        .unwrap();
    assert_eq!(rt.windows.open_window_count(), 1);
    assert_eq!(rt.windows.handle_for_surface(primary_surface), Some(handle));

    // Cascading surface destruction closes the associated OS window cleanly.
    rt.destroy_surface_cascade(primary_surface).unwrap();
    assert!(rt.windows.is_empty());
}

#[test]
fn multi_window_open_focus_resize_move_title_state_and_flags() {
    let mut wm = WindowManager::new();
    let primary = wm
        .open_window(
            1,
            SurfacePresentation::PrimaryWindow(
                WindowDescriptor::new("Primary", 1024.0, 768.0)
                    .with_bounds(WindowRect::new(20.0, 30.0, 1024.0, 768.0))
                    .with_min_size(320.0, 240.0)
                    .with_max_size(1920.0, 1080.0),
            ),
        )
        .unwrap();
    let secondary = wm
        .open_window(
            2,
            SurfacePresentation::SecondaryWindow {
                parent: Some(1),
                modal: false,
                descriptor: WindowDescriptor::new("Inspector", 420.0, 600.0),
            },
        )
        .unwrap();

    assert_eq!(wm.primary_window(), Some(primary));
    assert_eq!(wm.focused_window(), Some(secondary));
    assert_eq!(wm.open_window_count(), 2);

    // Reject duplicate surface window and Embedded presentation.
    assert_eq!(
        wm.open_window(1, SurfacePresentation::PrimaryWindow(WindowDescriptor::default())),
        Err(WindowError::SurfaceAlreadyHasWindow(1))
    );
    assert!(matches!(
        wm.open_window(99, SurfacePresentation::Embedded),
        Err(WindowError::InvalidDescriptor(_))
    ));

    // Focus back to primary, update title, bounds (clamped to min/max), state, decorations, always-on-top.
    wm.drain_events();
    wm.focus_window(primary).unwrap();
    assert_eq!(wm.focused_window(), Some(primary));
    assert!(wm.get(primary).unwrap().focused);
    assert!(!wm.get(secondary).unwrap().focused);

    wm.set_title(primary, "Primary — Edited".into()).unwrap();
    assert_eq!(wm.get(primary).unwrap().descriptor.title, "Primary — Edited");

    // Bounds below min_size clamp to (320, 240) and emit both Moved and Resized.
    wm.set_bounds_with_scale(primary, WindowRect::new(50.0, 60.0, 200.0, 100.0), 2.0)
        .unwrap();
    let clamped = wm.get(primary).unwrap().descriptor.bounds;
    assert_eq!(clamped, WindowRect::new(50.0, 60.0, 320.0, 240.0));

    wm.set_window_state(primary, WindowState::Maximized).unwrap();
    wm.set_decorations(primary, false).unwrap();
    wm.set_always_on_top(secondary, true).unwrap();
    assert!(!wm.get(primary).unwrap().descriptor.flags.contains(WindowFlags::DECORATED));
    assert!(wm.get(secondary).unwrap().descriptor.flags.contains(WindowFlags::ALWAYS_ON_TOP));

    let events = wm.drain_events();
    assert_eq!(
        events,
        vec![
            WindowLifecycleEvent::Focused { handle: secondary, focused: false },
            WindowLifecycleEvent::Focused { handle: primary, focused: true },
            WindowLifecycleEvent::Moved { handle: primary, x: 50.0, y: 60.0 },
            WindowLifecycleEvent::Resized {
                handle: primary,
                width: 320.0,
                height: 240.0,
                scale_factor: 2.0,
            },
            WindowLifecycleEvent::StateChanged { handle: primary, state: WindowState::Maximized },
        ]
    );
}

#[test]
fn modal_parent_focus_blocking_and_release_after_modal_close() {
    let mut wm = WindowManager::new();
    let parent = wm
        .open_window(1, SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Main", 800.0, 600.0)))
        .unwrap();
    let modal = wm
        .open_window(
            2,
            SurfacePresentation::SecondaryWindow {
                parent: Some(1),
                modal: true,
                descriptor: WindowDescriptor::new("Confirm Save", 400.0, 220.0),
            },
        )
        .unwrap();
    let nested_modal = wm
        .open_window(
            3,
            SurfacePresentation::SecondaryWindow {
                parent: Some(2),
                modal: true,
                descriptor: WindowDescriptor::new("Nested Alert", 300.0, 160.0),
            },
        )
        .unwrap();

    // Parent and intermediate modal are both blocked by the deepest modal descendant.
    assert_eq!(
        wm.focus_window(parent),
        Err(WindowError::BlockedByModalChild {
            target: parent,
            modal_child: nested_modal,
        })
    );
    assert_eq!(
        wm.focus_window(modal),
        Err(WindowError::BlockedByModalChild {
            target: modal,
            modal_child: nested_modal,
        })
    );
    assert!(wm.focus_window(nested_modal).is_ok());

    // Closing nested modal unblocks intermediate modal, still blocks root parent.
    wm.close_window(nested_modal).unwrap();
    assert_eq!(wm.focused_window(), Some(modal));
    assert_eq!(
        wm.focus_window(parent),
        Err(WindowError::BlockedByModalChild {
            target: parent,
            modal_child: modal,
        })
    );

    // Closing modal restores focusability on root parent.
    wm.close_window(modal).unwrap();
    assert_eq!(wm.focused_window(), Some(parent));
    assert!(wm.focus_window(parent).is_ok());
}

#[test]
fn cascading_close_of_owned_sheets_popovers_and_utility_panels_preserves_independent_secondary() {
    let mut wm = WindowManager::new();
    let parent = wm
        .open_window(10, SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Workspace", 1000.0, 700.0)))
        .unwrap();
    let sheet = wm
        .open_window(
            11,
            SurfacePresentation::Sheet {
                parent: 10,
                descriptor: WindowDescriptor::new("Export Sheet", 480.0, 360.0),
            },
        )
        .unwrap();
    let popover_on_sheet = wm
        .open_window(
            12,
            SurfacePresentation::Popover {
                anchor_surface: 11,
                anchor_rect: WindowRect::new(20.0, 40.0, 120.0, 32.0),
                descriptor: WindowDescriptor::new("Format Picker", 220.0, 180.0),
            },
        )
        .unwrap();
    let panel = wm
        .open_window(
            13,
            SurfacePresentation::UtilityPanel {
                parent: Some(10),
                always_on_top: true,
                descriptor: WindowDescriptor::new("Color Palette", 240.0, 320.0),
            },
        )
        .unwrap();
    let independent = wm
        .open_window(
            14,
            SurfacePresentation::SecondaryWindow {
                parent: Some(10),
                modal: false,
                descriptor: WindowDescriptor::new("Detached Docs", 640.0, 480.0),
            },
        )
        .unwrap();

    wm.drain_events();
    let closed = wm.close_window(parent).unwrap();
    // Bottom-up order: popover_on_sheet before sheet, plus panel, then parent; independent stays open.
    assert_eq!(closed, vec![popover_on_sheet, sheet, panel, parent]);
    assert_eq!(wm.open_window_count(), 1);
    assert!(wm.get(independent).is_some());
    assert_eq!(wm.get(independent).unwrap().parent_window, None);
    assert_eq!(wm.focused_window(), Some(independent));

    let closed_events: Vec<u32> = wm
        .drain_events()
        .into_iter()
        .filter_map(|e| match e {
            WindowLifecycleEvent::Closed { handle, .. } => Some(handle),
            _ => None,
        })
        .collect();
    assert_eq!(closed_events, vec![popover_on_sheet, sheet, panel, parent]);
}

#[test]
fn close_request_veto_allow_and_deny_flows() {
    let mut wm = WindowManager::new();
    let win = wm
        .open_window(1, SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Doc", 640.0, 480.0)))
        .unwrap();
    let popover = wm
        .open_window(
            2,
            SurfacePresentation::Popover {
                anchor_surface: 1,
                anchor_rect: WindowRect::new(10.0, 10.0, 80.0, 24.0),
                descriptor: WindowDescriptor::new("Menu", 180.0, 120.0),
            },
        )
        .unwrap();

    // Responding before CloseRequested fails cleanly.
    assert_eq!(wm.respond_close_request(win, true), Err(WindowError::NotClosePending(win)));

    wm.drain_events();
    assert!(wm.request_close(win).unwrap());
    // Repeated request_close while already pending is idempotent.
    assert!(!wm.request_close(win).unwrap());
    assert_eq!(wm.get(win).unwrap().lifecycle, WindowLifecycleState::ClosePending);
    assert_eq!(
        wm.drain_events(),
        vec![WindowLifecycleEvent::CloseRequested { handle: win, surface_id: 1 }]
    );

    // Deny (veto) returns window to Open with no windows closed.
    let vetoed = wm.respond_close_request(win, false).unwrap();
    assert!(vetoed.is_empty());
    assert_eq!(wm.get(win).unwrap().lifecycle, WindowLifecycleState::Open);
    assert_eq!(wm.open_window_count(), 2);

    // Re-request and allow -> cascades popover and closes primary.
    assert!(wm.request_close(win).unwrap());
    let allowed = wm.respond_close_request(win, true).unwrap();
    assert_eq!(allowed, vec![popover, win]);
    assert!(wm.is_empty());
}

#[test]
fn direct_typed_window_commands_and_lifecycle_event_flows() {
    let open_cmds = [
        WindowCommand::OpenWindow {
            surface_id: 1,
            presentation: SurfacePresentation::PrimaryWindow(
                WindowDescriptor::new("Main Window — 日本語", 1280.0, 720.0)
                    .with_bounds(WindowRect::new(40.0, 50.0, 1280.0, 720.0))
                    .with_min_size(400.0, 300.0)
                    .with_max_size(3840.0, 2160.0)
                    .with_state(WindowState::Normal),
            ),
        },
        WindowCommand::OpenWindow {
            surface_id: 2,
            presentation: SurfacePresentation::SecondaryWindow {
                parent: Some(1),
                modal: true,
                descriptor: WindowDescriptor::new("Modal", 400.0, 250.0),
            },
        },
        WindowCommand::OpenWindow {
            surface_id: 3,
            presentation: SurfacePresentation::UtilityPanel {
                parent: Some(1),
                always_on_top: true,
                descriptor: WindowDescriptor::new("Tools", 220.0, 480.0),
            },
        },
        WindowCommand::OpenWindow {
            surface_id: 4,
            presentation: SurfacePresentation::Popover {
                anchor_surface: 1,
                anchor_rect: WindowRect::new(120.0, 64.0, 88.0, 32.0),
                descriptor: WindowDescriptor::new("Quick Actions", 260.0, 180.0),
            },
        },
        WindowCommand::OpenWindow {
            surface_id: 5,
            presentation: SurfacePresentation::Sheet {
                parent: 1,
                descriptor: WindowDescriptor::new("Settings Sheet", 540.0, 400.0),
            },
        },
    ];

    let mut wm = WindowManager::new();
    let handles = wm.apply_commands(&open_cmds).unwrap();
    assert_eq!(handles, vec![Some(1), Some(2), Some(3), Some(4), Some(5)]);

    let mutate_cmds = [
        WindowCommand::SetTitle { handle: 1, title: "Renamed".into() },
        WindowCommand::SetBounds { handle: 1, bounds: WindowRect::new(10.0, 20.0, 900.0, 600.0) },
        WindowCommand::SetSizeConstraints {
            handle: 1,
            min_size: Some((300.0, 200.0)),
            max_size: None,
        },
        WindowCommand::SetWindowState { handle: 1, state: WindowState::Fullscreen },
        WindowCommand::FocusWindow { handle: 2 },
        WindowCommand::SetDecorations { handle: 1, decorated: false },
        WindowCommand::SetAlwaysOnTop { handle: 3, always_on_top: false },
    ];
    let mutate_res = wm.apply_commands(&mutate_cmds).unwrap();
    assert!(mutate_res.iter().all(Option::is_none));
    assert_eq!(wm.get(1).unwrap().descriptor.title, "Renamed");
    assert_eq!(wm.get(1).unwrap().descriptor.state, WindowState::Fullscreen);
    assert!(!wm.get(1).unwrap().descriptor.flags.contains(WindowFlags::DECORATED));
    assert!(!wm.get(3).unwrap().descriptor.flags.contains(WindowFlags::ALWAYS_ON_TOP));
    assert_eq!(wm.focused_window(), Some(2));

    wm.request_close(5).unwrap();
    wm.apply_commands(&[
        WindowCommand::RespondCloseRequest { handle: 5, allow: false },
        WindowCommand::CloseWindow { handle: 5 },
    ])
    .unwrap();
    assert!(wm.get(5).is_none());

    let events = wm.drain_events();
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Opened { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::CloseRequested { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Closed { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Focused { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Moved { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Resized { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::StateChanged { .. })));
}

#[test]
fn window_manager_validation_rejects_invalid_descriptors_bounds_and_constraints() {
    let mut wm = WindowManager::new();
    let h = wm
        .open_window(1, SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Main", 640.0, 480.0)))
        .unwrap();

    // Non-finite or zero bounds rejected cleanly.
    assert!(matches!(
        wm.set_bounds(h, WindowRect::new(0.0, 0.0, f32::NAN, 400.0)),
        Err(WindowError::InvalidDescriptor(_))
    ));
    assert!(matches!(
        wm.set_bounds(h, WindowRect::new(0.0, 0.0, 0.0, 400.0)),
        Err(WindowError::InvalidDescriptor(_))
    ));
    assert!(matches!(
        wm.set_bounds_with_scale(h, WindowRect::new(0.0, 0.0, 800.0, 600.0), 0.0),
        Err(WindowError::InvalidDescriptor(_))
    ));

    // Invalid size constraints (min > max, negative min, non-positive max) rejected cleanly.
    assert!(matches!(
        wm.set_size_constraints(h, Some((500.0, 500.0)), Some((400.0, 400.0))),
        Err(WindowError::InvalidDescriptor(_))
    ));
    assert!(matches!(
        wm.set_size_constraints(h, Some((-1.0, 0.0)), None),
        Err(WindowError::InvalidDescriptor(_))
    ));
    assert!(matches!(
        wm.set_size_constraints(h, None, Some((0.0, 100.0))),
        Err(WindowError::InvalidDescriptor(_))
    ));

    // Oversized title rejected cleanly.
    assert!(matches!(
        wm.set_title(h, "x".repeat(20_000)),
        Err(WindowError::InvalidDescriptor(_))
    ));

    // Unknown window handle fails cleanly.
    assert_eq!(wm.close_window(999), Err(WindowError::WindowNotFound(999)));
}
