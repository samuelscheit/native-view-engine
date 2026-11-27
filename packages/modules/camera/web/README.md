# `nve-camera-web`

Thin Web (`nve-backend-web`) leaf adapter for `nve-camera` rendering `CameraPreview` `<video>` elements via `WebTableRegistry`.

## Role & API

- Exports `register_camera_web_system(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for `CameraPreview`.

## Invariants

- Leaf backend adapter only; keeps camera/WebRTC glue completely out of `nve-backend-web` for minimal apps.
- Supports both SSR markup and live WASM DOM attachment.

## Verification

```bash
cargo test -p nve-camera-web
```
