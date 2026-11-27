# nve-host-fixtures

`nve-host-fixtures <out_dir>` writes real `NVE2` frames built with the production `NodeStore`
and standard tables, so native host suites drive `libnve_runtime` end to end instead of
hand-encoded wire bytes.

| File | Contents |
|---|---|
| `showcase_1.nve2` | keyframe: styled column, heading text (typography), secure text input, image (`asset://` URL), vertical scroll with 3 rows, canvas draw list, pressable button (a11y role + test id) |
| `showcase_2.nve2` | delta: text edit, image swap to inline bytes, row removal |
| `hot_v1.nve2` / `hot_v2.nve2` | one-text frames for app hot-reload generations |
| `showcase.ids` | `name=value` node ids, resource ids |

Used by `hosts/android-shell` CTest (`add_custom_command` runs it into the build tree).
