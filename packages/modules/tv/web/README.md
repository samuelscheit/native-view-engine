# `nve-tv-web`

Web TV D-Pad / remote / gamepad controller (`packages/modules/tv/web`, leaf backend of the `tv` capability module). Plugs `nve-tv` into `nve-backend-web` through the backend's generic key interceptor; the backend itself has zero TV knowledge.

## Behavior

- **Keys**: every surface-scoped `keydown` is normalized by `nve_tv::map_web_tv_key` (HbbTV, Tizen, webOS, Android TV / Fire TV, Xbox, PlayStation; IME and text-input aware). Non-TV keys pass through untouched.
- **Moves**: on a directional key only, focusable `[data-nve-id]` elements are measured just in time with `getBoundingClientRect()` (no per-frame DOM polling), spatial focus moves, the target gets DOM focus and is scrolled into view (`block/inline: nearest`, so off-screen shelves scroll), and the key's `preventDefault()` stops page scrolling.
- **Actions**: Back, media, channel, color, and digit keys go to the app's action handler; an unhandled Select clicks the focused element unless the browser activates it natively (Enter on a link / button).
- **DOM hints**: `data-nve-focus-group="<name>"` (group focus memory), `data-nve-next-{up,down,left,right}="<element id>"` (explicit neighbor), open modal `<dialog>` / `[data-nve-focus-trap]` (focus trap).
- **Platform**: Tizen remote keys are registered with `tvinputdevice.registerKeyBatch`; `platform::exit_to_platform` calls `webOS.platformBack()` / Tizen `exit()`. Both no-op elsewhere.
- **Gamepad** (opt-in): W3C standard-mapping poller on `requestAnimationFrame`, running only between `gamepadconnected` and the last `gamepaddisconnected`.

```rust
use nve_backend_web::{WebBackend, WebKeyDisposition, WebKeyEventInfo};
use nve_tv::{NodeId, RemoteAction, SpatialFocusNode, SpatialRect, WebTvPlatform};
use nve_tv_web::{WebTvConfig, WebTvController};

let mut backend = WebBackend::new();
let tv = WebTvController::install(&mut backend, WebTvConfig::for_platform(WebTvPlatform::Tizen));
tv.set_action_handler(|action, _node| action == RemoteAction::Back);

// Native stand-in for the wasm32 just-in-time rect collection.
tv.with_engine(|engine| {
    for i in 0..3 {
        let rect = SpatialRect::new(i as f32 * 120.0, 0.0, 100.0, 80.0);
        engine.register(SpatialFocusNode::new(NodeId::new(i + 1, 0), rect));
    }
});

let key = |key, key_code| WebKeyEventInfo {
    key,
    code: "",
    key_code,
    modifiers: 0,
    repeat: false,
    is_composing: false,
    in_text_input: false,
    is_multiline_input: false,
    target_node: None,
    timestamp_us: 0,
};
assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), WebKeyDisposition::ConsumedPreventDefault);
assert_eq!(tv.with_engine(|e| e.focused()), Some(NodeId::new(1, 0)));
// Tizen Back arrives as legacy keyCode 10009 and is handled by the app.
assert_eq!(backend.intercept_key(&key("Unidentified", 10009)), WebKeyDisposition::ConsumedPreventDefault);
// Ordinary typing is never intercepted.
assert_eq!(backend.intercept_key(&key("a", 65)), WebKeyDisposition::PassThrough);
```

## Dependency Invariants

- **Depends on**: `nve-tv`, `nve-backend-web`, `nve-hash`; `web-sys` / `js-sys` on `wasm32` only.
- `nve-backend-web` must never depend on this crate or on `nve-tv` (`[INV:module-and-host-boundaries]`).

## Verification

```bash
cargo test -p nve-tv-web
cargo clippy -p nve-tv-web --target wasm32-unknown-unknown -- -D warnings
```
