# `nve-i18n`

Platform-first, `wasm32`-ready CLDR internationalization and localization crate (`packages/framework/i18n`, **Framework layer**) using OS/browser APIs first (Apple `CoreFoundation` on macOS/iOS, browser `Intl.*` on `wasm32`) with a pure-Rust `BuiltinI18nBackend` fallback for SSR/headless/non-Apple targets.

## Key Public API

- `I18nBackend`, `NativeI18nBackend`, `BuiltinI18nBackend`, `system_locale`, `MessageCatalog`, `Translator`
- `LocaleId`, `UnicodeExtensions`, `UnicodeHourCycle` (BCP-47 `-u-ca-*`, `-u-fw-*`, `-u-hc-*`, `-u-nu-*` with zero-alloc inline parsing)
- `I18nError`, `MissingKeyDiagnostic`, `locale`, `message`, `plural`, `pseudo`
- `format::{CalendarDate, CalendarTime, Currency, DateStyle, TimeStyle, LocaleMetadata, MeasurementSystem, NumberFormatOptions, NumberStyle, RoundingMode, format_time, locale_metadata, format_currency_minor, format_relative_time_auto}`

```rust
use nve_i18n::{BuiltinI18nBackend, LocaleId, MessageCatalog, Translator};

let en = LocaleId::parse("en-US-u-fw-mon").unwrap();
let catalog = MessageCatalog::new(en.clone());
let _tr = Translator::with_backend(catalog, en, BuiltinI18nBackend);
```

## Dependency Invariants

- **Depends on**: `wasm32`-gated `wasm-bindgen` only (zero Cargo dependencies on native targets; links system `CoreFoundation.framework` on Apple macOS/iOS).
- **Must NOT depend on**: `dioxus*`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test --locked -p nve-i18n
cargo bench --locked -p nve-i18n --bench i18n_bench -- --quick
```
