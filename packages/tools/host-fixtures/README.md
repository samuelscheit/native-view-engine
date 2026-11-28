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

The package's library is `libnve_host_fixture_app` (cdylib exporting `nve_get_interface`): an
echo app whose `set_props` returns `true` on changed props and emits a structured `Custom`
event (`props`: `NveBinValue` map `{count, keys, ok, ratio}`); a truthy `malformed` prop first
emits a `malformed` custom event with invalid `NveBinValue` payload bytes (`[0xFF]`). Host FFI
suites (`NVE_HOST_FIXTURE_APP_LIB`, `scripts/host-runtimes.sh`) attach it to prove app-emitted
events cross the runtime C ABI.
