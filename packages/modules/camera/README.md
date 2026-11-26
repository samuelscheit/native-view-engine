# `nve-camera`

Optional VisionCamera v5 hybrid adapter and typed `CameraPreview` spec (`packages/modules/camera`, **Capability Module layer**) with a 40-byte `#[repr(C)]` `CameraFrameWireLayout` contract.

## Key Public API

- `CameraFacing`, `CameraPreview`, `CameraPreviewProps`, `CameraPreviewCommand`, `StabilizationMode`, `TorchMode`
- `CameraFrameDescriptor`, `CameraFrameWireLayout`, `CameraFrameProcessor`, `CameraPixelFormat`
- `nitro`, `interop`, `spec`

```rust
use nve_camera::{CameraFacing, CameraPreviewProps};

let props = CameraPreviewProps::new().with_facing(CameraFacing::Front);
assert_eq!(props.facing, CameraFacing::Front);
```

## Dependency Invariants

- **Depends on**: optional `nve-components` (`spec`), `nve-interop` (`interop`), `serde`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-camera
```
