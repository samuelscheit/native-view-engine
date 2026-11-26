# `nve-system-env`

Standalone opt-in live system theme and locale capability module (`packages/modules/system-env`, **Capability Module layer**, `SERVICE_SYSTEM_ENV = 0x0014`) with direct `SystemEnvBackend` dispatch and `NveChannelBus` bridge.

## Key Public API

- `SERVICE_SYSTEM_ENV`, `SystemEnvBackend`, `SystemEnvClient`, `SystemEnvBridge`, `MemorySystemEnv`, `NativeSystemEnv`
- `ThemeSnapshot`, `ColorScheme`, `ContrastLevel`, `LocaleSnapshot`, `TextDirection`, `SystemEnvEvent`
- `parse_locale_tag`, `dispatch_system_env_ffi`, `METHOD_GET_THEME`, `METHOD_GET_LOCALE`

```rust
use nve_system_env::{MemorySystemEnv, SystemEnvBackend};

let env = MemorySystemEnv::default();
let _theme = env.get_theme().unwrap();
let _locale = env.get_locale().unwrap();
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-host-services`, `nve-interop` (`channel` feature only).
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-system-env
```
