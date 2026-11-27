# `nve-video-web`

Thin Web (`nve-backend-web`) leaf adapter rendering `nve-video` `VideoPlayer` nodes as HTML5 `<video>` elements with playback command handling.

## Role & API

- Exports `register_video_web_system(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for `VideoPlayer`.

## Invariants

- Leaf backend adapter only; `nve-backend-web` has zero dependency on `nve-video`.
- Blocks unsafe URI schemes before assigning `<video src>`.

## Verification

```bash
cargo test -p nve-video-web
```
