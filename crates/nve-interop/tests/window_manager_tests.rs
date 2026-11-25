use nve_interop::{
    FrameworkKind, InteropRuntime, NVW1_HEADER_SIZE, NVW1_MAGIC, SurfaceDescriptor,
    SurfaceEnvironment, SurfacePresentation, WindowCodecError, WindowCommand, WindowDescriptor,
    WindowError, WindowFlags, WindowLifecycleEvent, WindowLifecycleState, WindowManager,
    WindowRect, WindowState, decode_window_commands, decode_window_events, encode_window_commands,
    encode_window_events,
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
fn nvw1_binary_command_and_event_roundtrips_and_apply() {
    let cmds = vec![
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
        WindowCommand::RespondCloseRequest { handle: 5, allow: false },
        WindowCommand::CloseWindow { handle: 5 },
    ];

    let encoded = encode_window_commands(&cmds);
    assert_eq!(&encoded[..4], &NVW1_MAGIC);
    let decoded = decode_window_commands(&encoded).unwrap();
    assert_eq!(decoded, cmds);

    // Cross-kind decode must fail cleanly.
    assert_eq!(decode_window_events(&encoded), Err(WindowCodecError::InvalidPayloadKind(1)));

    // Execute subset via WindowManager and round-trip all 7 event variants.
    let mut wm = WindowManager::new();
    for cmd in &decoded[..5] {
        wm.apply_command(cmd.clone()).unwrap();
    }
    wm.apply_command(decoded[5].clone()).unwrap();
    wm.apply_command(decoded[6].clone()).unwrap();
    wm.apply_command(decoded[8].clone()).unwrap();
    wm.request_close(5).unwrap();
    wm.apply_command(decoded[12].clone()).unwrap();
    wm.apply_command(decoded[13].clone()).unwrap();

    let events = wm.drain_events();
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Opened { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::CloseRequested { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Closed { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Focused { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Moved { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::Resized { .. })));
    assert!(events.iter().any(|e| matches!(e, WindowLifecycleEvent::StateChanged { .. })));

    let ev_bytes = encode_window_events(&events);
    assert_eq!(decode_window_events(&ev_bytes).unwrap(), events);
    assert_eq!(decode_window_commands(&ev_bytes), Err(WindowCodecError::InvalidPayloadKind(2)));
}

#[test]
fn nvw1_fuzz_truncated_corrupt_bad_magic_and_malformed_inputs_never_panic() {
    let valid_cmds = encode_window_commands(&[
        WindowCommand::OpenWindow {
            surface_id: 1,
            presentation: SurfacePresentation::PrimaryWindow(
                WindowDescriptor::new("Hello", 640.0, 480.0).with_min_size(100.0, 100.0),
            ),
        },
        WindowCommand::SetBounds { handle: 1, bounds: WindowRect::new(0.0, 0.0, 800.0, 600.0) },
    ]);
    let valid_events = encode_window_events(&[
        WindowLifecycleEvent::Opened {
            handle: 1,
            surface_id: 1,
            bounds: WindowRect::new(0.0, 0.0, 640.0, 480.0),
            state: WindowState::Normal,
        },
        WindowLifecycleEvent::Resized { handle: 1, width: 800.0, height: 600.0, scale_factor: 2.0 },
    ]);

    // Every strict prefix of valid command & event buffers must fail with Truncated.
    for cut in 0..valid_cmds.len() {
        assert!(matches!(
            decode_window_commands(&valid_cmds[..cut]),
            Err(WindowCodecError::Truncated { .. })
        ));
    }
    for cut in 0..valid_events.len() {
        assert!(matches!(
            decode_window_events(&valid_events[..cut]),
            Err(WindowCodecError::Truncated { .. })
        ));
    }

    // Trailing byte rejection.
    let mut with_trailing = valid_cmds.clone();
    with_trailing.push(0xAA);
    assert_eq!(decode_window_commands(&with_trailing), Err(WindowCodecError::TrailingBytes(1)));

    let mut ev_trailing = valid_events.clone();
    ev_trailing.extend_from_slice(&[1, 2, 3]);
    assert_eq!(decode_window_events(&ev_trailing), Err(WindowCodecError::TrailingBytes(3)));

    // Bad magic & bad version.
    let mut bad_magic = valid_cmds.clone();
    bad_magic[0] = b'X';
    assert_eq!(decode_window_commands(&bad_magic), Err(WindowCodecError::BadMagic(*b"XVW1")));

    let mut bad_ver = valid_cmds.clone();
    bad_ver[4] = 99;
    assert_eq!(decode_window_commands(&bad_ver), Err(WindowCodecError::UnsupportedVersion(99)));

    // Huge record count is rejected before allocation (anti-OOM).
    let mut huge_count = vec![b'N', b'V', b'W', b'1', 1, 0, 1, 0, 0xFF, 0xFF, 0xFF, 0x7F];
    assert!(matches!(
        decode_window_commands(&huge_count),
        Err(WindowCodecError::TooManyRecords(_))
    ));
    huge_count[6] = 2;
    assert!(matches!(
        decode_window_events(&huge_count),
        Err(WindowCodecError::TooManyRecords(_))
    ));

    // Unknown opcode, invalid bool, NaN float, invalid UTF-8.
    let mut bad_op = vec![b'N', b'V', b'W', b'1', 1, 0, 1, 0, 1, 0, 0, 0, 99];
    assert_eq!(decode_window_commands(&bad_op), Err(WindowCodecError::InvalidOpcode(99)));
    bad_op[6] = 2;
    assert_eq!(decode_window_events(&bad_op), Err(WindowCodecError::InvalidOpcode(99)));

    let bad_bool = vec![b'N', b'V', b'W', b'1', 1, 0, 1, 0, 1, 0, 0, 0, 8, 1, 0, 0, 0, 2];
    assert_eq!(
        decode_window_commands(&bad_bool),
        Err(WindowCodecError::InvalidEnumValue { field: "decorated", value: 2 })
    );

    let mut nan_resize = encode_window_events(&[WindowLifecycleEvent::Resized {
        handle: 1,
        width: 100.0,
        height: 100.0,
        scale_factor: 1.0,
    }]);
    let nan_bytes = f32::NAN.to_le_bytes();
    nan_resize[NVW1_HEADER_SIZE + 5..NVW1_HEADER_SIZE + 9].copy_from_slice(&nan_bytes);
    assert_eq!(decode_window_events(&nan_resize), Err(WindowCodecError::NonFiniteFloat("width")));

    let mut bad_utf8 = encode_window_commands(&[WindowCommand::SetTitle {
        handle: 1,
        title: "abcd".into(),
    }]);
    let n = bad_utf8.len();
    bad_utf8[n - 2] = 0xFF;
    bad_utf8[n - 1] = 0xFE;
    assert_eq!(decode_window_commands(&bad_utf8), Err(WindowCodecError::InvalidUtf8));

    // Deterministic pseudo-random fuzz sweep across 512 mutated buffers: zero panics.
    let mut seed: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let mut next_u8 = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 32) as u8
    };
    for i in 0..512 {
        let len = (i % 64) + 1;
        let mut buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        if i % 3 == 0 && buf.len() >= 8 {
            buf[..4].copy_from_slice(&NVW1_MAGIC);
            buf[4..6].copy_from_slice(&1u16.to_le_bytes());
            buf[6] = if i % 2 == 0 { 1 } else { 2 };
        }
        let _ = decode_window_commands(&buf);
        let _ = decode_window_events(&buf);
    }
}
