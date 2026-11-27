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
| `nve/host/bin_codec.hpp` | C++20 | `BinReader` / `BinWriter`: bounds-checked LE primitives plus the tagged `NveBinValue` format, with sticky errors, no exceptions, and zero heap allocations on read |
| `nve/host/table_reader.hpp` | C++20 | `TableReader` over any `NveTableView`-shaped struct (`std::optional` on every read; specs that do not fit the row read `nullopt`); `field()` is `consteval` and rejects invalid specs at compile time |
| `nve/host/scene_reader.hpp` | C++20 | `SceneReader<View>` over the generated `NveSceneView`. It validates once, then does O(1) bounds-checked reads. |
| `nve/host/hybrid_runtime.hpp` | C++20 | `HybridStatus`, `HostStatus`, `check_schema`, `DynamicValue` |

`BinWriter` records `InvalidUtf8` for strings that are not well-formed UTF-8, and `TooLarge` for a
length above `u32`; a rejected write leaves no bytes at all. `end(mark)` records `InvalidMark`
when the mark:

- is stale: it predates `clear()`, which bumps the mark's generation;
- is already closed;
- does not point just past an `Array` / `Map` tag.

While validating, `BinReader` bounds every child read by its container's end, as Rust does.

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
