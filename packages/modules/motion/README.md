# `nve-motion`

Pure-spec motion capability: `EasingCurve` (CSS keywords, `cubic-bezier`, springs), closed-form
`SpringSpec`/`SpringState`, the `MotionClock` track slab, interpolation helpers, and the
`Motion` (`nve.motion`) table with its `Transitions` resource.

## Role & API

- `Motion` (`#[derive(NveTable)]`) references a validated `Transitions` resource (7 words/spec).
- `MotionClock` keys tracks by `MotionKey::Named(Key64)` or `MotionKey::Node(NodeId, TransitionProperty)`
  in a sorted dense slab: binary-search lookups, contiguous per-node runs, allocation-free ticks.
- Leaf adapters: `web/` (`nve-motion-web`, native CSS `transition`, springs via CSS `linear()`),
  `skia/` (`nve-motion-skia`, a `SkiaFrameHook` driving animated paint).

```rust
use nve_motion::{EasingCurve, MotionClock, MotionKey, SpringSpec, TransitionProperty, TransitionSpec};

let mut clock = MotionClock::new();
clock.animate_spring(MotionKey::named("sheet"), SpringSpec::snappy(), 0.0, 1.0);
while clock.has_active_animations() {
    clock.tick(1.0 / 60.0);
}
assert_eq!(clock.value(&MotionKey::named("sheet")), Some(1.0));

let spring = TransitionSpec::spring(TransitionProperty::Opacity, SpringSpec::bouncy());
assert!(matches!(spring.easing, EasingCurve::Spring(_)));
assert_eq!(spring.easing.sample(1.0), 1.0);
```

## Invariants

- Zero backend/platform dependencies; never imported by `packages/core/*` or `packages/backends/*`.
- `Transitions::write_words` self-validates: producers only intern what replicas accept.

## Verification

```bash
cargo test -p nve-motion -p nve-motion-web -p nve-motion-skia
```
