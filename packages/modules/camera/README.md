# `nve-camera`

Optional VisionCamera v5 hybrid adapter and typed `CameraPreview` spec (`packages/modules/camera`, **Capability Module layer**) with a 40-byte `#[repr(C)]` `CameraFrameWireLayout` contract.

## Key Public API

- `CameraFacing`, `CameraPreviewTable`, `CameraStabilization`, `CameraTorch`
- `CameraFrameDescriptor`, `CameraFrameWireLayout`, `CameraFrameProcessor`, `CameraPixelFormat`
- `nitro` (feature-gated), `frame`, `table`, `types`

```rust
use nve_camera::CameraFacing;

assert_eq!(CameraFacing::from_preview_u8(2), Some(CameraFacing::Front));
assert_eq!(CameraFacing::from_preview_u8(0), None);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-hash`, `nve-store`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-camera
```
