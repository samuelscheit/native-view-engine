# `nve-tv-web`

Web TV D-Pad / remote / gamepad controller (`packages/modules/tv/web`, leaf backend of the `tv` capability module). Plugs `nve-tv` into `nve-backend-web` through the backend's generic key interceptor; the backend itself has zero TV knowledge.

## Behavior

- **Keys**: every surface-scoped `keydown` is normalized by `nve_tv::map_web_tv_key` (HbbTV, Tizen, webOS, Android TV / Fire TV, Xbox, PlayStation; IME and text-input aware; named non-TV keys are never remapped by `code` / `keyCode`). Non-TV keys pass through untouched.
- **Moves**: on a directional key only, focusable `[data-nve-id]` elements are measured just in time with `getBoundingClientRect()` (no per-frame DOM polling; skipped for throttled repeats). Disabled, `aria-disabled`, `inert` / `hidden`, `tabindex < 0`, `contenteditable="false"`, collapsed (`0x0`), and `visibility: hidden` elements are never candidates. Spatial focus moves, the target gets DOM focus (rolled back when focus does not stick) and is scrolled into view (`block/inline: nearest`, with a `scrollIntoViewIfNeeded` fallback on Chromium < 61 TVs), and the key's `preventDefault()` stops page scrolling. Initial focus prefers elements inside the viewport.
- **Arrow-owning widgets**: arrows on the axes a focused widget owns (`role=tablist|slider|menubar` horizontal, `listbox|menu|combobox|spinbutton` and native `<select>` vertical, `grid|treegrid|tree` both, `aria-orientation` respected) reach the widget; `data-nve-spatial-nav="off"` opts a subtree out entirely, any other value opts a widget back in.
- **Actions**: Back, media, channel, color, and digit keys go to the app's action handler; an unhandled Select clicks the focused element (resolved inside the active focus trap, never a `<dialog>`, disabled, inert, or `tabindex < 0` element) unless the browser activates it natively (Enter on a link / button holding DOM focus), and opens native `<select>` pickers with `showPicker()`. Back / Escape inside a text control first blurs it (closing the on-screen keyboard).
- **Held keys**: only moves, channel, seek, and digit keys auto-repeat (throttled per action); Select / Back / play-pause fire once. A dropped repeat is consumed exactly when its first press was.
- **DOM hints**: `data-nve-focus-group="<name>"` (group focus memory, bounded LRU), `data-nve-next-{up,down,left,right}="<element id>"` (explicit neighbor, read for the element focus leaves; naming the element itself pins focus), modal `<dialog>` anywhere in the document (`:modal`, else `aria-modal="true"`, which NVE writes for modal dialogs) or rendered `[data-nve-focus-trap]` other than `"false"` (focus trap).
- **Platform**: Tizen remote keys are registered with `tvinputdevice.registerKeyBatch` (`registerKey` on Tizen < 4); `platform::exit_to_platform` calls `webOS.platformBack()` / Tizen `exit()`. Both no-op elsewhere.
- **Gamepad** (opt-in): W3C standard-mapping poller on `requestAnimationFrame`, running only while a standard pad is connected and the page is visible; zero Rust allocations per frame. On Xbox, Edge's gamepad-as-keyboard emulation is switched off while it runs.
- **Lifecycle**: `install` hands the backend a strong handle; action handlers should capture `WebTvController::downgrade()`. `uninstall` removes the interceptor, gamepad poller, and handler. No `RefCell` borrow is held across DOM focus / click events or app code, so `focusin` listeners and handlers may re-enter the controller.

```rust
use nve_backend_web::{WebBackend, WebKeyDisposition, WebKeyEventInfo};
use nve_tv::{NodeId, RemoteAction, SpatialFocusNode, SpatialRect, WebTvPlatform};
use nve_tv_web::{WebTvConfig, WebTvController};

let mut backend = WebBackend::new();
let tv = WebTvController::install(&mut backend, WebTvConfig::for_platform(WebTvPlatform::Tizen));
let weak = tv.downgrade();
tv.set_action_handler(move |action, _node| action == RemoteAction::Back && weak.upgrade().is_some());

// Native stand-in for the wasm32 just-in-time rect collection.
tv.with_engine(|engine| {
    for i in 0..3 {
        let rect = SpatialRect::new(i as f32 * 120.0, 0.0, 100.0, 80.0);
        engine.register(SpatialFocusNode::new(NodeId::new(i + 1, 0), rect));
    }
});

let key = |key, key_code| WebKeyEventInfo::new(key, "", key_code);
assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), WebKeyDisposition::ConsumedPreventDefault);
assert_eq!(tv.with_engine(|e| e.focused()).flatten(), Some(NodeId::new(1, 0)));
// Tizen Back arrives as legacy keyCode 10009 and is handled by the app.
assert_eq!(backend.intercept_key(&key("Unidentified", 10009)), WebKeyDisposition::ConsumedPreventDefault);
// Ordinary typing is never intercepted.
assert_eq!(backend.intercept_key(&key("a", 65)), WebKeyDisposition::PassThrough);
tv.uninstall(&mut backend);
```

## Dependency Invariants

- **Depends on**: `nve-tv`, `nve-backend-web`, `nve-hash`; `web-sys` / `js-sys` on `wasm32` only.
- `nve-backend-web` must never depend on this crate or on `nve-tv` (`[INV:module-and-host-boundaries]`).
- Every DOM decision is a pure, natively tested `nve_tv_web::rules` function; `dom.rs` only gathers facts.

## Verification

```bash
cargo test -p nve-tv-web
cargo clippy -p nve-tv-web --target wasm32-unknown-unknown -- -D warnings
node examples/fullstack-issues/e2e/run_e2e.mjs   # suite 5: live WebTvController probe (trusted CDP keys)
```
