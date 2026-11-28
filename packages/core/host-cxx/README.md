# nve_host_cxx

Header-only, zero-dependency C11 + C++20 NVE host runtime. It holds the static half of every
C / C++ / Objective-C++ host binding. Codegen emits only thin data next to it: ABI structs, table
keys, and field specs.

| Header | Language | Contents |
|---|---|---|
| `nve/host/api.h` | C11 / C++20 | `NVE_API`, `NVE_EXTERN_C_BEGIN/END`, `NVE_STATIC_ASSERT`, `NVE_ALIGNOF`, `NVE_ALIGNAS`, `NVE_OFFSETOF`, little-endian guard, alignment-free `nve_load_*_le` / `nve_store_*_le`, bit-field extract / sign-extend |
| `nve/host/xxh64.h` | C11 (`static inline`) / C++20 (`constexpr`) | Canonical XXH64 (`nve_xxh64*`), bit-identical to Rust `nve-hash`, and `NveKey64` |
| `nve/host/xxh64.hpp` | C++20 | `nve::host::Xxh64Hasher` and `xxh64()` (`constexpr`); `const_id()` / `Key64::of()` are `consteval` and reject non-UTF-8 names at compile time (runtime names: `nve_key64_of`, which returns 0 for invalid UTF-8) |
| `nve/host/utf8.h` | C11 / C++20 | Allocation-free strict UTF-8 validation (same acceptance as Rust `str::from_utf8`) |
| `nve/host/table_word.h` | C11 / C++20 | `NveFieldKind`, `NveFieldSpec`, `nve_field_valid` / `nve_field_fits`, field / `Length` / RGBA word decoding; every bit helper is total (an invalid range reads 0, never a UB shift) |
| `nve/host/bin_codec.hpp` | C++20 | `BinReader` / `BinWriter`: bounds-checked LE primitives plus the tagged `NveBinValue` format, with sticky errors (`to_string(BinError)` names them), no exceptions, and zero heap allocations on read |
| `nve/host/utf16.hpp` | C++20 | Strict `utf16_to_utf8` / `utf8_to_utf16` (lone surrogates and invalid UTF-8 are `nullopt`) |
| `nve/host/table_reader.hpp` | C++20 | `TableReader` over any `NveTableView`-shaped struct (`std::optional` on every read; specs that do not fit the row read `nullopt`); `field()` is `consteval` and rejects invalid specs at compile time |
| `nve/host/scene_reader.hpp` | C++20 | `SceneReader<View>` over the generated `NveSceneView`. It validates once, then does O(1) bounds-checked reads. |
| `nve/host/hybrid_runtime.hpp` | C++20 | `HybridStatus`, `HostStatus`, `check_schema`, `DynamicValue` (`from_bytes` validates a buffer, `read` a reader's next value with sticky errors); with `bin_codec.hpp`, the runtime of every generated C++ hybrid client and JNI glue (`nve codegen hybrid`) |
| `nve/host/value_tree.hpp` | C++20 (+ generated ABI header) | `NveValueTree`: the C `NveValue` view of a `DynamicValue`, for C signatures that take `NveValue` |
| `nve/host/runtime_session.hpp` | C++20 | `RuntimeSession` / `RuntimeSceneGuard` (borrowed `resource(id)` payloads) / `RuntimeSceneReader` over `libnve_runtime`'s `NveRuntimeApi`, including the runtime's app files: `load_app_module` (staged `cdylib` copies, state-carrying hot reload) and `save_state_file` / `restore_state_file` / `clear_state_file` (the engine's checksummed, crash-atomic `StateFile`); `scroll_to` (absolute, runtime-clamped scroll) |
| `nve/host/bound_table.hpp` | C++20 | `BoundTable` (a generated `NveTableDesc` resolved in one scene, O(1) node → row; a dense table's row belongs to a node only when its `table_set` bit is set) and `RowRef` (row or defaults); packed `NodeId` helpers |
| `nve/host/scene_nodes.hpp` | C++20 | `SceneTables` (every core table a view host renders) and `read_node` → `NodeData` (kind, absolute / local rects, visual, inherited typography, text, image, text input, scroll, interaction, accessibility, link); `aria_role` / `accessible_label` (explicit or kind-native semantics shared by every host) |
| `nve/host/controlled_text.hpp` | C++20 | `ControlledText`: when a native text field must show the app's value and when a native edit is a `TextChange` (echo / IME safe; host writes are not edits); synchronous `app_answered` writes back rejected / transformed edits, paired (async) answers never overwrite newer typing |
| `nve/host/urls.hpp` | C++20 | URL policy: `openable_link` (link presses open only `http(s)://` / `mailto:`) and `local_image_path` (image files only from local paths / `file:` URLs on localhost: no UNC or remote hosts) |
| `nve/host/launch_options.hpp` | C++20 | Shared native-shell command line (`--dylib` / `--pair` / `--state` / ... + host flags) and app-state lifecycle (`resolve_launch_state`, `state_action_after_load`: a fallback app never touches the failed app's state) |
| `nve/host/retained_tree.hpp` | C++20 | `RetainedTree<Adapter>`: O(changed) native-view reconciler (removals, `dirty_indices`, typography cascades, generation-checked slot reuse, relinking recreated views) |
| `nve/host/host_session.hpp` | C++20 | `HostSession`: runtime surface + in-process / runtime-loaded `cdylib` (hot reload, `ModuleError` status mapping) / paired app, UI-thread wake poster, `EventFactory` (press / pointer / text / focus / scroll), emitted-event sink, props, app-state files |
| `nve/host/app_state.hpp` | C++20 | App-state path policy only: per-app cache path, `NVE_STATE_DISABLE` / `NVE_STATE_PATH`, `reset_requested()` (`NVE_RESET_STATE`); the file itself is the runtime's |
| `nve/host/nve2_writer.hpp` | C++20 | `Nve2Writer`: `NVE2` frames from generated `NVE_TABLE_<T>_DESC`s (C++ apps, demos, fixtures) |
| `nve/host/core_resources.hpp` | C++20 | `ImageSrc` / `BoxShadows` resource codecs |
| `nve/host/demo_counter_app.hpp` | C++20 | Built-in counter demo app (`nve_get_interface`) shared by every native shell and fixture |
| `tests/support/nve/host/testing/scripted_app.hpp` | C++20 (tests) | `ScriptedApp`: a C ABI app whose frames a test writes, recording dispatched events (shared by every host suite) |
| `tests/support/nve/host/testing/loopback_pair_server.hpp` | C++20 (tests) | `LoopbackPairServer`: in-process `ws://` pairing server (handshake, HelloAck, queued `NVE2` keyframes) for live paired round trips (POSIX + Winsock) |
| `nve/host/jni.hpp` | C++20 (+ `<jni.h>`) | CheckJNI-clean helpers of the generated JNI glue: `throw_java_exception`, strict `direct_buffer_span`, `to_jbyte_array`, and UTF-16 (never Modified UTF-8) `utf8_of` / `java_string` |

`BinWriter` records `InvalidUtf8` for strings that are not well-formed UTF-8, and `TooLarge` for a
length above `u32`; a rejected write leaves no bytes at all. `end(mark)` records `InvalidMark`
when the mark:

- is stale: it predates `clear()`, which bumps the mark's generation;
- is already closed;
- does not point just past an `Array` / `Map` tag.

`BinReader` bounds every child read by its container's end, as Rust does: while validating, and
between `begin_array()` / `begin_map()` (which return a `ReadMark` with the entry count) and
`end(mark)`, which also requires the payload to have been read exactly (`TrailingBytes`).

The readers are templates over the generated structs, so the node record layout is never
hard-coded here. A codegen change to `NveRenderNode` / `NveEvent` needs no edit in this package.

## Use

```cmake
add_subdirectory(${NVE_ROOT}/packages/core/host-cxx nve_host_cxx)
target_link_libraries(my_target PRIVATE nve::host_cxx)
```

```cpp
#include <nve/host/scene_reader.hpp>
constexpr auto kVisual = nve::host::Key64::of("nve.visual");  // baked at compile time
auto scene = nve::host::SceneReader<NveSceneView>::open(view, nve::host::Key64{NVE_SCENE_SCHEMA_HASH});
```

## Generated data

`nve codegen abi` writes `include/nve/generated/nve_abi.generated.h` (Core + Scene C ABI with layout `static_assert`s for LP64, ILP32 and i386 System V), `include/nve/generated/nve_tables.generated.h` (table keys, defaults, `NveFieldSpec`s, X-macro lists) and `tests/hash_vectors.generated.h`. All three are gitignored; the CMake helper `cxx/cmake/NveCodegen.cmake` regenerates them when stale. `tests/test_generated.cpp` checks every generated key and field spec at compile time.

## Verify

`scripts/host-runtimes.sh cxx` runs this suite. `scripts/check-all.sh` runs it too: C/C++ and TS in `--quick` mode, all five in a full run.

```bash
cmake -S packages/core/host-cxx -B "$TMPDIR/nve-host-cxx" -G Ninja && cmake --build "$TMPDIR/nve-host-cxx" \
  && ctest --test-dir "$TMPDIR/nve-host-cxx" --output-on-failure
```

The suites build with `-Wall -Wextra -Werror -pedantic -Wconversion -Wsign-conversion -Wshadow`
under ASan and UBSan:

- C11 (`tests/test_c11.c`)
- C++20 XXH64, including `static_assert` golden vectors (`tests/test_xxh64.cpp`)
- `BinReader` / `BinWriter` (`tests/test_bin_codec.cpp`): golden bytes, every truncated prefix, count bombs, depth 64/65, a 20k-case fuzz, and a counting `operator new` that proves reads and steady-state writes do not allocate
- Table / scene layout and bounds (`tests/test_table_scene.cpp`)
- Negative compile checks (`tests/compile_fail.cpp`): invalid `field()` specs and non-UTF-8 `Key64::of` names must not compile
