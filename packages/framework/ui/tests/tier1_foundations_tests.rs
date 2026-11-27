//! Deep cross-backend (`HeadlessBackend` + `WebBackend`) integration tests for the three Tier 1
//! UI foundations:
//! 1. Subtree `ErrorBoundary` (structured `Result` errors, `throw_ui_error`, real panics,
//!    sibling isolation, telemetry deduplication, and `reset()` recovery).
//! 2. Cross-platform `AutofillHint` / `InputMode` (`<input>`, `<text_input>`, `nve_ui::Input`,
//!    SSR + live DOM attribute emission, and zero-keydown programmatic autofill sync).
//! 3. Cross-platform `KeyboardInsets` trait + `KeyboardAvoidingView` + Web `visualViewport`
//!    avoidance geometry (zero double-padding & `max(safe_area, keyboard)`).

mod common;

use std::cell::{Cell, RefCell};

use nve_abi::{EventPayload, FrameId, NativeEvent, NativeEventKind, NodeId, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_backend_web::{InputSyncTracker, WebBackend};
use nve_components::tables::{DebugId, TextContent, TextInput};
use nve_dioxus::elements::backend::{StoreBackend, Viewport};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_ui::prelude::*;

use common::UiHarness;

/// Dual-backend harness feeding every frame's `CommandBuffer` into BOTH `HeadlessBackend` and
/// `WebBackend` simultaneously to prove cross-backend parity.
struct DualHarness {
    surface: DioxusSurface,
    headless: HeadlessBackend,
    web: WebBackend,
    frame_id: FrameId,
    timestamp_us: u64,
}

impl DualHarness {
    fn new(root: fn() -> Element) -> Self {
        let vp = Viewport {
            width: 390.0,
            height: 844.0,
            scale_factor: 3.0,
        };
        let mut headless = HeadlessBackend::new();
        headless.set_viewport(SurfaceId::ROOT, vp);
        let mut web = WebBackend::new();
        web.set_viewport(SurfaceId::ROOT, vp);
        let mut h = Self {
            surface: DioxusSurface::new(SurfaceId::ROOT, root),
            headless,
            web,
            frame_id: FrameId::ZERO,
            timestamp_us: 0,
        };
        h.render();
        h
    }

    fn render(&mut self) {
        self.frame_id = self.frame_id.next();
        self.surface.reconcile(self.frame_id);
        let store = self.surface.store();
        let changes = self.surface.last_changes();
        self.headless
            .commit(store, changes)
            .expect("headless commit succeeded");
        self.web
            .commit(store, changes)
            .expect("web commit succeeded");
    }

    fn maybe_find(&self, tid: &str) -> Option<NodeId> {
        let store = self.surface.store();
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            if id != NodeId::ROOT
                && let Some(dbg) = store.try_row::<DebugId>(id)
                && dbg.test_id() == tid
            {
                return Some(id);
            }
            stack.extend_from_slice(store.children(id));
        }
        None
    }

    fn find(&self, tid: &str) -> NodeId {
        let found = self.maybe_find(tid);
        assert!(found.is_some(), "expected node with testID='{tid}'");
        found.expect("checked above")
    }

    fn collect_text(&self, id: NodeId) -> String {
        let store = self.surface.store();
        let mut out = String::new();
        if let Some(tc) = store.try_row::<TextContent>(id) {
            out.push_str(tc.text());
        }
        for &cid in store.children(id) {
            out.push_str(&self.collect_text(cid));
        }
        out
    }

    fn text_of(&self, tid: &str) -> String {
        self.collect_text(self.find(tid))
    }

    fn next_ts(&mut self) -> u64 {
        self.timestamp_us += 16_000;
        self.timestamp_us
    }

    fn press(&mut self, tid: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 8.0, y: 8.0 },
        }]));
        self.render();
    }

    fn text_change(&mut self, tid: &str, value: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(value.to_owned()),
        }]));
        self.render();
    }
}

thread_local! {
    static TELEMETRY_LOG: RefCell<Vec<UiErrorReport>> = const { RefCell::new(Vec::new()) };
    static RESET_LOG: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    static UNRELATED_RENDERS: Cell<u32> = const { Cell::new(0) };
    static TEST_TRAIT_STORE: RefCell<Option<ReactiveTraitStore>> = const { RefCell::new(None) };
}

fn clear_logs() {
    TELEMETRY_LOG.with(|l| l.borrow_mut().clear());
    RESET_LOG.with(|l| l.borrow_mut().clear());
}

#[component]
fn ResultFailingChild(should_fail: Signal<bool>) -> Element {
    if should_fail() {
        return Err(ui_error("checkout cart parse failed").into());
    }
    rsx! {
        view {
            test_id: "healthy-child",
            text { "Healthy child mounted" }
            pressable {
                test_id: "throw-event-err",
                onpress: move |_| {
                    throw_ui_error("async submit rejected");
                },
                text { "Trigger event error" }
            }
        }
    }
}

#[component]
fn ErrorBoundaryResultApp() -> Element {
    let mut sibling_count = use_signal(|| 0u32);
    let mut should_fail = use_signal(|| true);

    rsx! {
        ThemeProvider {
            view {
                test_id: "app-root",
                view {
                    test_id: "sibling-pane",
                    text { test_id: "sibling-count", "{sibling_count}" }
                    pressable {
                        test_id: "sibling-btn",
                        onpress: move |_| sibling_count += 1,
                        text { "Increment sibling" }
                    }
                }
                ErrorBoundary {
                    on_error: move |report: UiErrorReport| {
                        TELEMETRY_LOG.with(|l| l.borrow_mut().push(report));
                    },
                    on_reset: move |count: u32| {
                        RESET_LOG.with(|l| l.borrow_mut().push(count));
                    },
                    fallback: move |report: UiErrorReport, reset: ErrorBoundaryReset| {
                        let loc = report.source_location.clone().unwrap_or_default();
                        rsx! {
                            view {
                                test_id: "custom-fallback",
                                text { test_id: "fb-msg", "{report.message}" }
                                text { test_id: "fb-loc", "{loc}" }
                                text { test_id: "fb-resets", "{report.reset_count}" }
                                pressable {
                                    test_id: "fb-retry",
                                    onpress: move |_| {
                                        should_fail.set(false);
                                        reset.reset();
                                    },
                                    text { "Retry" }
                                }
                            }
                        }
                    },
                    ResultFailingChild { should_fail }
                }
            }
        }
    }
}

#[test]
fn error_boundary_isolates_result_and_throw_errors_preserves_sibling_and_resets_on_both_backends() {
    clear_logs();
    let mut h = DualHarness::new(ErrorBoundaryResultApp);

    // Frame 1 immediately renders fallback on BOTH HeadlessBackend and WebBackend.
    assert!(h.maybe_find("healthy-child").is_none());
    assert_eq!(h.text_of("fb-msg"), "checkout cart parse failed");
    assert!(h.text_of("fb-loc").contains("tier1_foundations_tests.rs:"));
    assert_eq!(h.text_of("fb-resets"), "0");

    let ssr_err = h.web.render_ssr(h.surface.store()).html;
    assert!(ssr_err.contains("checkout cart parse failed"));
    assert!(h.web.render_html(h.surface.store()).contains("checkout cart parse failed"));

    // Telemetry callback fired exactly once with structured report.
    TELEMETRY_LOG.with(|l| {
        let log = l.borrow();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].message, "checkout cart parse failed");
        assert!(log[0].source_location.as_deref().is_some_and(|s| s.contains("tier1_foundations_tests.rs:")));
        assert_eq!(log[0].reset_count, 0);
        assert!(!log[0].is_panic);
    });

    // Sibling subtree remains fully interactive while boundary is tripped; no duplicate telemetry.
    h.press("sibling-btn");
    assert_eq!(h.text_of("sibling-count"), "1");
    assert!(h.maybe_find("custom-fallback").is_some());
    TELEMETRY_LOG.with(|l| assert_eq!(l.borrow().len(), 1));

    // Clicking Retry invokes reset(), cleanly remounting healthy child while keeping sibling state.
    h.press("fb-retry");
    assert!(h.maybe_find("custom-fallback").is_none());
    assert_eq!(h.text_of("healthy-child"), "Healthy child mountedTrigger event error");
    assert_eq!(h.text_of("sibling-count"), "1");
    RESET_LOG.with(|l| assert_eq!(*l.borrow(), vec![1]));
    let ssr_recovered = h.web.render_ssr(h.surface.store()).html;
    assert!(ssr_recovered.contains("Healthy child mounted"));
    assert!(!ssr_recovered.contains("checkout cart parse failed"));

    // Imperative `throw_ui_error` from an event handler inside the child trips the boundary again
    // with incremented `reset_count == 1`.
    h.press("throw-event-err");
    assert!(h.maybe_find("healthy-child").is_none());
    assert_eq!(h.text_of("fb-msg"), "async submit rejected");
    assert_eq!(h.text_of("fb-resets"), "1");
    TELEMETRY_LOG.with(|l| {
        let log = l.borrow();
        assert_eq!(log.len(), 2);
        assert_eq!(log[1].message, "async submit rejected");
        assert_eq!(log[1].reset_count, 1);
        assert!(!log[1].is_panic);
    });
}

#[component]
fn PanickingChild(should_panic: Signal<bool>) -> Element {
    if should_panic() {
        #[expect(clippy::panic, reason = "intentionally panics to verify ErrorBoundary recovery")]
        {
            panic!("simulated subtree panic");
        }
    }
    rsx! {
        text { test_id: "panic-child-ok", "Recovered from panic" }
    }
}

#[component]
fn ErrorBoundaryPanicApp() -> Element {
    let mut should_panic = use_signal(|| true);
    let mut sibling_clicks = use_signal(|| 0u32);

    rsx! {
        ThemeProvider {
            view {
                pressable {
                    test_id: "panic-sibling-btn",
                    onpress: move |_| sibling_clicks += 1,
                    text { test_id: "panic-sibling-val", "{sibling_clicks}" }
                }
                UiErrorBoundary {
                    test_id: "eb-panic",
                    on_error: move |report: UiErrorReport| {
                        TELEMETRY_LOG.with(|l| l.borrow_mut().push(report));
                    },
                    on_reset: move |_| {
                        should_panic.set(false);
                    },
                    PanickingChild { should_panic }
                }
            }
        }
    }
}

#[test]
fn error_boundary_catches_real_subtree_panic_and_recovers_via_default_themed_retry() {
    clear_logs();
    let mut h = DualHarness::new(ErrorBoundaryPanicApp);

    // Default themed nve_ui::ErrorBoundary fallback is mounted; panicking child is isolated.
    assert!(h.maybe_find("panic-child-ok").is_none());
    assert!(h.maybe_find("eb-panic").is_some());
    assert!(h.text_of("eb-panic-message").contains("Encountered panic"));
    assert!(h.web.render_ssr(h.surface.store()).html.contains("role=\"alert\""));

    TELEMETRY_LOG.with(|l| {
        let log = l.borrow();
        assert_eq!(log.len(), 1);
        assert!(log[0].is_panic);
        assert_eq!(log[0].reset_count, 0);
    });

    // Sibling still responds while panic fallback is displayed.
    h.press("panic-sibling-btn");
    assert_eq!(h.text_of("panic-sibling-val"), "1");

    // Clicking the built-in Retry button resets the boundary and mounts the healthy child.
    h.press("eb-panic-retry");
    assert!(h.maybe_find("eb-panic").is_none());
    assert_eq!(h.text_of("panic-child-ok"), "Recovered from panic");
    assert_eq!(h.text_of("panic-sibling-val"), "1");
}

#[component]
fn AutofillFormApp() -> Element {
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut otp = use_signal(String::new);
    let mut raw_tel = use_signal(String::new);

    rsx! {
        ThemeProvider {
            view {
                Input {
                    test_id: "field-email",
                    label: "Email",
                    value: email(),
                    autocomplete: AutofillHint::Email,
                    input_mode: InputMode::Email,
                    on_change: move |v| email.set(v),
                }
                Input {
                    test_id: "field-password",
                    label: "Password",
                    value: password(),
                    secure: true,
                    autocomplete: AutofillHint::CurrentPassword,
                    on_change: move |v| password.set(v),
                }
                Input {
                    test_id: "field-otp",
                    label: "One-Time Code",
                    value: otp(),
                    autocomplete: AutofillHint::OneTimeCode,
                    input_mode: InputMode::Numeric,
                    on_change: move |v| otp.set(v),
                }
                textinput {
                    test_id: "raw-tel",
                    value: "{raw_tel}",
                    autocomplete: AutofillHint::Tel.as_html_autocomplete(),
                    input_mode: InputMode::Tel.as_html_inputmode(),
                    onchange: move |ev: TextEvent| raw_tel.set(ev.data().value.clone()),
                }
                input {
                    test_id: "raw-cc",
                    autocomplete: AutofillHint::CcNumber.as_html_autocomplete(),
                    input_mode: InputMode::Numeric.as_html_inputmode(),
                }
                text { test_id: "state-summary", "{email}|{password}|{otp}|{raw_tel}" }
            }
        }
    }
}

#[test]
fn autofill_and_inputmode_emit_canonical_ssr_and_dom_attrs_and_sync_without_keydown() {
    let mut h = DualHarness::new(AutofillFormApp);

    // 1. Verify canonical typed TextInput autofill/input_mode across nve_ui::Input, <text_input>, <input>.
    let check_raw = |tid: &str, expected_ac: &str, expected_im: Option<&str>| {
        let id = h.find(tid);
        let ti = h
            .surface
            .store()
            .try_row::<TextInput>(id)
            .unwrap_or_else(|| panic!("expected TextInput row on {tid}"));
        let actual_ac = ti.autofill().map(AutofillHint::as_html_autocomplete);
        assert_eq!(
            actual_ac,
            Some(expected_ac),
            "autocomplete mismatch on {tid}"
        );
        let actual_im =
            (ti.input_mode() != InputMode::Text).then(|| ti.input_mode().as_html_inputmode());
        assert_eq!(actual_im, expected_im, "inputmode mismatch on {tid}");
    };
    check_raw("field-email", "email", Some("email"));
    check_raw("field-password", "current-password", None);
    check_raw("field-otp", "one-time-code", Some("numeric"));
    check_raw("raw-tel", "tel", Some("tel"));
    check_raw("raw-cc", "cc-number", Some("numeric"));

    // 2. Verify WebBackend emits canonical HTML attributes in BOTH render_ssr() and render_html().
    let ssr = h.web.render_ssr(h.surface.store()).html;
    let html = h.web.render_html(h.surface.store());
    for markup in [&ssr, &html] {
        assert!(markup.contains("autocomplete=\"email\""), "missing email autocomplete in {markup}");
        assert!(markup.contains("inputmode=\"email\""), "missing email inputmode in {markup}");
        assert!(markup.contains("autocomplete=\"current-password\""), "missing password autocomplete");
        assert!(markup.contains("type=\"password\""), "missing type=password");
        assert!(markup.contains("autocomplete=\"one-time-code\""), "missing OTP autocomplete");
        assert!(markup.contains("inputmode=\"numeric\""), "missing numeric inputmode");
        assert!(markup.contains("autocomplete=\"tel\""), "missing tel autocomplete");
        assert!(markup.contains("inputmode=\"tel\""), "missing tel inputmode");
        assert!(markup.contains("autocomplete=\"cc-number\""), "missing cc-number autocomplete");
    }

    // 3. Simulate programmatic OS/password-manager autofill firing `change` / `input` with ZERO
    // preceding `keydown` events via InputSyncTracker -> TextChange -> NVE state!
    let mut sync_tracker = InputSyncTracker::new();
    let email_id = h.find("field-email");
    let pw_id = h.find("field-password");
    let otp_id = h.find("field-otp");
    let tel_id = h.find("raw-tel");

    // Password manager fills email via `change`-only event (zero keydown, zero input).
    let email_val = sync_tracker
        .on_change_event(email_id, "sam@nve.dev".into())
        .expect("change-only autofill emits TextChange");
    h.text_change("field-email", &email_val);

    // Browser autofill fills password via `input` then duplicate `change` (zero keydown).
    let pw_val = sync_tracker
        .on_input_event(pw_id, "s3cr3t!".into(), false)
        .expect("input autofill emits TextChange");
    assert_eq!(
        sync_tracker.on_change_event(pw_id, "s3cr3t!".into()),
        None,
        "duplicate change event after identical input must be deduplicated"
    );
    h.text_change("field-password", &pw_val);

    // iOS/Android SMS OTP suggestion inserts code with zero keydown.
    let otp_val = sync_tracker
        .on_change_event(otp_id, "492817".into())
        .expect("OTP autofill emits TextChange");
    h.text_change("field-otp", &otp_val);

    let tel_val = sync_tracker
        .on_change_event(tel_id, "+14155550199".into())
        .expect("tel autofill emits TextChange");
    h.text_change("raw-tel", &tel_val);

    assert_eq!(
        h.text_of("state-summary"),
        "sam@nve.dev|s3cr3t!|492817|+14155550199"
    );
}

#[component]
fn UnrelatedSibling() -> Element {
    UNRELATED_RENDERS.with(|c| c.set(c.get() + 1));
    rsx! {
        text { test_id: "unrelated-sibling", "Static sibling" }
    }
}

#[component]
fn KeyboardAvoidingApp() -> Element {
    let store = use_provide_trait_store(|| {
        let mut s = TraitStore::new();
        s.set::<SafeAreaInsets>(SafeAreaInsets::new(47.0, 0.0, 34.0, 0.0));
        s
    });
    TEST_TRAIT_STORE.with(|slot| *slot.borrow_mut() = Some(store));

    let kb = use_keyboard_insets();
    let eff_overlay = use_effective_bottom_inset(false);
    let eff_resize = use_effective_bottom_inset(true);

    rsx! {
        view {
            UnrelatedSibling {}
            text {
                test_id: "kb-readout",
                "{kb.visible}:{kb.height}:{eff_overlay}:{eff_resize}"
            }
            KeyboardAvoidingView {
                test_id: "ios-composer",
                os_adjusts_resize: false,
                text { "Chat Composer (iOS/Overlay)" }
            }
            KeyboardAvoidingView {
                test_id: "android-composer",
                os_adjusts_resize: true,
                text { "Chat Composer (Android adjustResize)" }
            }
        }
    }
}

#[test]
fn keyboard_insets_and_avoiding_view_handle_visual_viewport_and_zero_double_padding() {
    UNRELATED_RENDERS.with(|c| c.set(0));
    TEST_TRAIT_STORE.with(|s| *s.borrow_mut() = None);

    let mut h = UiHarness::new(KeyboardAvoidingApp);
    let mut web = WebBackend::new();
    let store = TEST_TRAIT_STORE
        .with(|s| s.borrow().clone())
        .expect("trait store provided");

    // 1. Keyboard closed: both views respect safe_area.bottom (34.0) with no extra padding.
    assert_eq!(h.text_of("kb-readout"), "false:0:34:34");
    assert_eq!(h.padding(h.find("ios-composer")).bottom, 34.0);
    assert_eq!(h.padding(h.find("android-composer")).bottom, 34.0);
    assert_eq!(UNRELATED_RENDERS.with(Cell::get), 1);

    // 2. Web `visualViewport` shrinks (layout=844, visual=508 -> 336px keyboard).
    assert!(web.update_visual_viewport(844.0, 508.0, 0.0, 1.0));
    let kb_open = web.keyboard_insets();
    assert!(kb_open.visible && kb_open.height == 336.0);
    h.surface.in_root_scope(|| {
        assert!(store.set::<KeyboardInsets>(kb_open));
    });
    h.render();

    // iOS/overlay uses max(34, 336) = 336 (NOT 370); Android adjustResize uses 0 (zero double-padding).
    assert_eq!(h.text_of("kb-readout"), "true:336:336:0");
    assert_eq!(h.padding(h.find("ios-composer")).bottom, 336.0);
    assert_eq!(h.padding(h.find("android-composer")).bottom, 0.0);
    // Unrelated sibling never re-rendered when KeyboardInsets changed.
    assert_eq!(UNRELATED_RENDERS.with(Cell::get), 1);

    // 3. Interactive iOS drag-dismiss partially open below home indicator (height=18, visible=true).
    h.surface.in_root_scope(|| {
        assert!(store.set::<KeyboardInsets>(KeyboardInsets::new(18.0, true, 0, true)));
    });
    h.render();
    assert_eq!(h.padding(h.find("ios-composer")).bottom, 34.0);
    assert_eq!(h.padding(h.find("android-composer")).bottom, 0.0);

    // 4. Web `visualViewport` restores to full height -> keyboard dismissed -> safe_area.bottom=34.
    assert!(web.update_visual_viewport(844.0, 844.0, 0.0, 1.0));
    h.surface.in_root_scope(|| {
        assert!(store.set::<KeyboardInsets>(web.keyboard_insets()));
    });
    h.render();
    assert_eq!(h.text_of("kb-readout"), "false:0:34:34");
    assert_eq!(h.padding(h.find("ios-composer")).bottom, 34.0);
    assert_eq!(h.padding(h.find("android-composer")).bottom, 34.0);
    assert_eq!(UNRELATED_RENDERS.with(Cell::get), 1);
}
