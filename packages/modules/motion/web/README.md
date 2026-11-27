# `nve-motion-web`

Web DOM & SSR adapter for `nve-motion`: `MotionWebSystem` compiles each node's `Transitions` into a
content-addressed native CSS `transition` class (springs become CSS `linear()` easings sampled from
the closed-form spring), plus a shared `@media (prefers-reduced-motion:reduce)` override.

```rust
use nve_motion::{EasingCurve, SpringSpec, TransitionProperty, TransitionSpec};

let mut css = String::new();
nve_motion_web::write_transition_entry(
    &TransitionSpec::new(TransitionProperty::Opacity, 150).easing(EasingCurve::EaseOut),
    &mut css,
);
assert_eq!(css, "opacity 150ms ease-out");

css.clear();
nve_motion_web::write_easing_css(EasingCurve::Spring(SpringSpec::bouncy()), &mut css);
assert!(css.starts_with("linear(0,") && css.ends_with(",1)"));
```

## Verification

```bash
cargo test -p nve-motion-web
```
