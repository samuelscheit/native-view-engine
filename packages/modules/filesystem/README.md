# `nve-filesystem`

Standalone opt-in file picker and standard OS paths capability module (`packages/modules/filesystem`, **Capability Module layer**, `SERVICE_FILESYSTEM = 0x0011`) with `MemoryFilesystem` and OS `NativeFilesystem` handlers.

## Key Public API

- `SERVICE_FILESYSTEM`, `StandardPathKind`, `FileFilter`, `FilePickerOptions`, `SaveFileOptions`
- `NativeFilesystem`, `native_standard_path`, `MemoryFilesystem`, `FilesystemBackend`, `FilesystemClient`
- `METHOD_STANDARD_PATH`, `METHOD_PICK_FILES`, `METHOD_SAVE_FILE`, binary request/response codecs

```rust
use nve_filesystem::{FileFilter, FilePickerOptions};

let opts = FilePickerOptions { multiple: true, filters: vec![FileFilter { name: "Images".into(), extensions: vec!["png".into()] }], ..Default::default() };
assert!(opts.multiple);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-host-services`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-filesystem
```
