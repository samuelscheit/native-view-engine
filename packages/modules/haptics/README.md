# `nve-haptics`

Standalone opt-in tactile haptic feedback capability module (`packages/modules/haptics`, **Capability Module layer**, `SERVICE_HAPTICS = 0x0016`) with direct in-process `HapticsBackend` dispatch and C ABI codecs.

## Key Public API

- `SERVICE_HAPTICS`, `HapticsBackend`, `HapticsClient`, `MemoryHaptics`, `NativeHaptics`
- `ImpactStyle`, `NotificationHaptic`, `RecordedHaptic`
- `METHOD_IMPACT`, `METHOD_NOTIFICATION`, `METHOD_SELECTION`, `dispatch_haptics_wire`

```rust
use nve_haptics::{HapticsBackend, ImpactStyle, MemoryHaptics};

let haptics = MemoryHaptics::new();
haptics.impact(ImpactStyle::Medium).unwrap();
assert_eq!(haptics.recorded().len(), 1);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-host-services`.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-haptics
```
