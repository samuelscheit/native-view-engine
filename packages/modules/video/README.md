# `nve-video`

Pure-spec capability crate defining the `VideoPlayer` (`nve.video_player`) word table and playback command opcodes.

## Role & API

- Exposes `VideoPlayer` (`#[derive(NveTable)]`) with `source_uri`, `autoplay`, `muted`, `looping`, and `controls`, plus playback commands (`Play`, `Pause`, `SeekMs`, `SetVolume`).
- Platform adapters live in sibling subdirectories (`web/`, `apple/`).

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Enforces safe media URI schemes (`https://`, `http://`, `file://`).

## Verification

```bash
cargo test -p nve-video
```
