# `nve-motion-skia`

Skia adapter for `nve-motion`: `MotionDriver` is a `SkiaFrameHook` that animates each node's
declarative `Motion` transitions (colors, radius, opacity, transform) on the Skia frame clock and
lets the host's display link sleep once everything settled.

```rust
use nve_backend_skia::SkiaBackend;
use nve_motion_skia::MotionDriver;

let mut skia = SkiaBackend::new();
skia.register_frame_hook(MotionDriver::new());
assert!(!skia.needs_next_frame());
```

## Verification

```bash
cargo test -p nve-motion-skia
```
