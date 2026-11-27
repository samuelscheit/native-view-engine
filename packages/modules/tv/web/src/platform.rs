//! Vendor runtime helpers (Tizen key registration, webOS / Tizen app exit). Every helper probes
//! the global object dynamically and no-ops gracefully off TV runtimes and off `wasm32`.

use nve_tv::WebTvPlatform;

/// Detects the running TV browser from `navigator.userAgent` (`Standard` off `wasm32`).
pub fn detect_platform() -> WebTvPlatform {
    #[cfg(target_arch = "wasm32")]
    if let Some(ua) = web_sys::window().and_then(|w| w.navigator().user_agent().ok()) {
        return WebTvPlatform::from_user_agent(&ua);
    }
    WebTvPlatform::Standard
}

/// Registers vendor remote keys that are not delivered to web apps by default. On Tizen this
/// calls `tizen.tvinputdevice.registerKeyBatch` with the supported subset of
/// [`nve_tv::web_keys::TIZEN_REGISTERED_KEYS`]. Returns the number of keys registered.
pub fn register_platform_keys(platform: WebTvPlatform) -> usize {
    match platform {
        #[cfg(target_arch = "wasm32")]
        WebTvPlatform::Tizen => wasm::register_tizen_keys().unwrap_or(0),
        _ => 0,
    }
}

/// Xbox Edge / WebView2 deliver gamepad presses as keyboard events too
/// (`navigator.gamepadInputEmulation = "keyboard"`): `false` switches that off while the Gamepad
/// API poller runs, so one press is not dispatched twice. Returns `false` where the property
/// does not exist (every other browser, native).
pub fn set_xbox_gamepad_emulation(keyboard: bool) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        wasm::set_gamepad_input_emulation(if keyboard { "keyboard" } else { "gamepad" })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = keyboard;
        false
    }
}

/// Hands Back on the root screen to the platform: `webOS.platformBack()` on webOS,
/// `tizen.application.getCurrentApplication().exit()` on Tizen. Returns `false` when the runtime
/// offers no such API (desktop browsers, other platforms, native).
pub fn exit_to_platform(platform: WebTvPlatform) -> bool {
    match platform {
        #[cfg(target_arch = "wasm32")]
        WebTvPlatform::WebOs => wasm::call_path(&["webOS"], "platformBack").is_some(),
        #[cfg(target_arch = "wasm32")]
        WebTvPlatform::Tizen => wasm::tizen_exit().is_some(),
        _ => false,
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use js_sys::{Array, Function, Reflect};
    use nve_tv::web_keys::TIZEN_REGISTERED_KEYS;
    use wasm_bindgen::{JsCast, JsValue};

    /// Resolves `globalThis.a.b...`, `None` when any segment is missing.
    fn global_path(path: &[&str]) -> Option<JsValue> {
        path.iter()
            .try_fold(JsValue::from(js_sys::global()), |obj, seg| {
                Reflect::get(&obj, &JsValue::from_str(seg))
                    .ok()
                    .filter(|v| !v.is_undefined() && !v.is_null())
            })
    }

    fn method(obj: &JsValue, name: &str) -> Option<Function> {
        Reflect::get(obj, &JsValue::from_str(name))
            .ok()?
            .dyn_into::<Function>()
            .ok()
    }

    /// Calls `globalThis.<path>.<name>()`.
    pub(super) fn call_path(path: &[&str], name: &str) -> Option<JsValue> {
        let obj = global_path(path)?;
        method(&obj, name)?.call0(&obj).ok()
    }

    pub(super) fn tizen_exit() -> Option<JsValue> {
        let app = call_path(&["tizen", "application"], "getCurrentApplication")?;
        method(&app, "exit")?.call0(&app).ok()
    }

    pub(super) fn set_gamepad_input_emulation(mode: &str) -> bool {
        let Some(navigator) = global_path(&["navigator"]) else {
            return false;
        };
        let property = JsValue::from_str("gamepadInputEmulation");
        Reflect::has(&navigator, &property).unwrap_or(false)
            && Reflect::set(&navigator, &property, &JsValue::from_str(mode)).unwrap_or(false)
    }

    /// `registerKeyBatch` (Tizen 4+), else one `registerKey` per name (Tizen 2.3-3).
    pub(super) fn register_tizen_keys() -> Option<usize> {
        let device = global_path(&["tizen", "tvinputdevice"])?;
        let supported: Array = method(&device, "getSupportedKeys")?
            .call0(&device)
            .ok()?
            .dyn_into()
            .ok()?;
        let names = Array::new();
        for key in supported.iter() {
            let name = Reflect::get(&key, &JsValue::from_str("name"))
                .ok()
                .and_then(|n| n.as_string());
            if let Some(name) = name.filter(|n| TIZEN_REGISTERED_KEYS.contains(&n.as_str())) {
                names.push(&JsValue::from_str(&name));
            }
        }
        if names.length() == 0 {
            return Some(0);
        }
        if let Some(batch) = method(&device, "registerKeyBatch") {
            batch.call1(&device, &names).ok()?;
            return Some(names.length() as usize);
        }
        let single = method(&device, "registerKey")?;
        Some(
            names
                .iter()
                .filter(|name| single.call1(&device, name).is_ok())
                .count(),
        )
    }
}
