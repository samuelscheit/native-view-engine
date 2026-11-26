# `nve-clipboard`

Standalone opt-in clipboard capability module (`packages/modules/clipboard`, **Capability Module layer**, `SERVICE_CLIPBOARD = 0x0010`) with direct in-process `ClipboardBackend` dispatch and compact binary wire codecs.

## Key Public API

- `SERVICE_CLIPBOARD`, `ClipboardBackend`, `ClipboardClient`, `MemoryClipboard`, `NativeClipboard`
- `METHOD_READ_TEXT`, `METHOD_WRITE_TEXT`, `METHOD_HAS_TEXT`, `METHOD_CLEAR`
- `dispatch_clipboard_wire`, `encode_write_text_request`, `decode_read_text_response`

```rust
use nve_clipboard::{ClipboardBackend, MemoryClipboard};

let clip = MemoryClipboard::new();
clip.write_text("hello").unwrap();
assert_eq!(clip.read_text().unwrap().as_deref(), Some("hello"));
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-host-services`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-clipboard
```
