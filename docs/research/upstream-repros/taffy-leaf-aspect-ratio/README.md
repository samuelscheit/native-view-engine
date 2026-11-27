# taffy-leaf-aspect-ratio — already reported upstream

Verdict: **already reported** (genuine bug, still present on taffy 0.16.0 and `main` 497de49, 2026-11-23).

An empty flex item with `aspect-ratio` and an `auto` cross size gets cross size 0: Flexbox sizes leaves with `SizingMode::ContentSize`, and `compute_leaf_layout` (`src/compute/leaf.rs`) drops `style.aspect_ratio` in that mode, while `flexbox.rs` never transfers the flexed main size through the ratio itself.

## Existing reports

- Issue https://github.com/DioxusLabs/taffy/issues/804 (open) — "`aspect_ratio` is not respected in flex layouts". The 2026-11-04 comment shows `flex-grow: 1; aspect-ratio: 2` in a 500×100 row → Taffy 500×0, Chrome 500×250, on 0.15.1 and `main` fca167c.
- PR https://github.com/DioxusLabs/taffy/pull/1081 (open) — fixes #804 according to that comment.
- PR https://github.com/DioxusLabs/taffy/pull/1213 (open) — resolves aspect ratios the same way in every layout algorithm, including `leaf.rs` and `flexbox.rs`.
- PR https://github.com/DioxusLabs/taffy/pull/1215 (open) — removes `SizingMode`, so parents resolve `aspect_ratio` for their children.
- Related: #1418, #1421 (other aspect-ratio defects), #733.

## Observed before the stop rule (cargo fixture, now removed)

Row/column flex container, `align-items: flex-start`, one empty leaf:

| Case | taffy 0.16.0 | `main` 497de49 | HeadlessChrome 158 |
| --- | --- | --- | --- |
| row 500×100, `flex-basis:200px; flex-shrink:0; aspect-ratio:2` | 200×0 | 200×0 | 200×100 |
| row 500×100, `flex-grow:1; aspect-ratio:2` | 500×0 | 500×0 | 500×250 |
| column 100×400, `flex-basis:200px; flex-shrink:0; aspect-ratio:0.5` | 0×200 | 0×200 | 100×200 |
| control: row, `width:200px; aspect-ratio:2` | 200×100 | 200×100 | 200×100 |

## Should we comment?

A comment adds little. It would only say that 0.16.0 and `main` 497de49 are still affected and that `flex-basis` (not only `flex-grow`) triggers it. If you post one, add it to #804 with the first table row.

## Note for this repo

The mitigation in `docs/UPSTREAM.md` points to `packages/backends/headless/src/taffy_engine.rs`, which no longer exists. `taffy_store/style_view.rs` only forwards `aspect_ratio`. The entry is stale: either the workaround was lost, or it should point to its new location.
