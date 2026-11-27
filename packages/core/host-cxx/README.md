# nve_host_cxx

Header-only, zero-dependency C11 + C++20 NVE host runtime. It holds the static half of every
C / C++ / Objective-C++ host binding. Codegen emits only thin data next to it: ABI structs, table
keys, and field specs.

| Header | Language | Contents |
|---|---|---|
| `nve/host/api.h` | C11 / C++20 | `NVE_API`, `NVE_EXTERN_C_BEGIN/END`, `NVE_STATIC_ASSERT`, `NVE_ALIGNOF`, `NVE_OFFSETOF`, little-endian guard, alignment-free `nve_load_*_le` / `nve_store_*_le`, bit-field extract / sign-extend |
| `nve/host/xxh64.h` | C11 (`static inline`) / C++20 (`constexpr`) | Canonical XXH64 (`nve_xxh64*`), bit-identical to Rust `nve-hash`, and `NveKey64` |
| `nve/host/xxh64.hpp` | C++20 | `nve::host::Xxh64Hasher`, `xxh64()`, `const_id()`, `Key64` — all `constexpr` |
| `nve/host/utf8.h` | C11 / C++20 | Allocation-free strict UTF-8 validation (same acceptance as Rust `str::from_utf8`) |
| `nve/host/table_word.h` | C11 / C++20 | `NveFieldKind`, `NveFieldSpec`, field / `Length` / RGBA word decoding |
| `nve/host/bin_codec.hpp` | C++20 | `BinReader` / `BinWriter`: bounds-checked LE primitives plus the tagged `NveBinValue` format, with sticky errors, no exceptions, and zero heap allocations on read |
| `nve/host/table_reader.hpp` | C++20 | `TableReader` over any `NveTableView`-shaped struct (`std::optional` on every read) |
| `nve/host/scene_reader.hpp` | C++20 | `SceneReader<View>` over the generated `NveSceneView`. It validates once, then does O(1) bounds-checked reads. |
| `nve/host/hybrid_runtime.hpp` | C++20 | `HybridStatus`, `HostStatus`, `check_schema`, `DynamicValue` |

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

## Verify

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
