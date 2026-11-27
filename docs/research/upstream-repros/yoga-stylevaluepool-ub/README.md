# Yoga `StyleValuePool` float→int32 cast UB — fixed upstream

**Verdict: fixed-upstream (genuine UB, fix not yet released or synced into React Native).**

- Fix: https://github.com/react/yoga/pull/2075 (formerly facebook/yoga), MERGED 2026-11-26, commit `3a201ff4`, "Check the inline range before casting style values to int32". It also adds regression tests in `tests/StyleValuePoolTest.cpp`.
- Cause (`yoga/style/StyleValuePool.h`, `isIntegerPackable`, introduced in #1534, 2024-01): `auto i = static_cast<int32_t>(f);` ran before the range check. Converting a float outside the int32 range, or ±inf, to an integer is UB under C++ [conv.fpint]. The fix compares against ±2047 first and casts only after that.
- **Threshold**: |v| ≥ 2^31. The bad values are `v >= 2147483648.0f` or `v <= -2147483904.0f`; `INT32_MIN` itself is exact. Setters for numbers (`flexGrow`, `flexShrink`, `flexBasis` number, etc.) also hit it with ±inf. Length/size setters map inf to undefined.
- Still present: no release contains the fix (latest tag `v3.2.2`). React Native `main` `packages/react-native/ReactCommon/yoga/yoga/style/StyleValuePool.h` still has the old code. RN picks it up at the next Yoga sync.
- No separate issue exists; tracker searches for `isIntegerPackable` / `StyleValuePool` / float-cast-overflow found nothing besides the PR. **Do not comment**: the PR already covers the affected versions, the values and the UBSan evidence.

## Effect on our mitigation

`kMaxLayoutScalar = 1e7f` in `cxx/nve-fabric-bridge/src/command_decoder.hpp` is a product clamp, not the UB threshold: values up to about 2.1e9 are safe. Keep the clamp until RN ships a Yoga that contains #2075. UPSTREAM.md should state the real threshold (2^31) and link the PR.

## Sibling entry: "Detached `YGNodeRef` returns NaN layout metrics"

This is **intended behavior**, not a bug. The layout results default to `YGUndefined` (NaN) until `YGNodeCalculateLayout` has run on the node's tree. Yoga's public API documents `YGUndefined` as NaN and uses it as the "not computed / undefined" sentinel. Turning NaN into 0 before handing values to AppKit is our host's job, and the existing sanitization in `fabric_adapter.cpp` is the right place for it. Nothing to report.
