# `nve-i18n`

Platform-first, `wasm32`-ready CLDR internationalization and localization crate (`packages/framework/i18n`, **Framework layer**) using OS/browser `Intl` APIs first with a pure-Rust `BuiltinI18nBackend` fallback for SSR/headless targets.

## Key Public API

- `I18nBackend`, `NativeI18nBackend`, `BuiltinI18nBackend`, `MessageCatalog`, `Translator`
- `I18nError`, `MissingKeyDiagnostic`, `locale`, `message`, `plural`, `pseudo`
- `format::{CalendarDate, Currency, DateStyle, NumberFormatOptions, format_currency_minor, format_relative_time_auto}`

```rust
use nve_i18n::{BuiltinI18nBackend, LocaleId, MessageCatalog, Translator};

let en = LocaleId::parse("en").unwrap();
let catalog = MessageCatalog::new(en.clone());
let _tr = Translator::with_backend(catalog, en, BuiltinI18nBackend);
```

## Dependency Invariants

- **Depends on**: `wasm32`-gated `wasm-bindgen` only (zero heavy Rust dependencies on native targets).
- **Must NOT depend on**: `dioxus*`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-i18n
```
