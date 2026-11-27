# swift-collection-endindex-oom: ObjectOutliner allocates 8 bytes per tail element of a constant-count `alloc_ref`

**Verdict: genuine upstream bug (compiler memory blowup), not yet reported, still present on `main`.**

## Repro

```swift
// repro.swift
struct HugeBytes: RandomAccessCollection {
    var startIndex: Int { 0 }
    var endIndex: Int { Int(UInt32.max) + 1 }
    subscript(_: Int) -> UInt8 { 0 }
}
```

`swiftc -O -parse-as-library -emit-object repro.swift`

`./run.sh` runs every compile under `watchdog.sh`, which kills the compile at 6 GB RSS or after 120 s. `./variants.sh` builds the variant matrix (3 GB cap). Outputs go to `out/`, which is gitignored.

- **Expected**: compiles in about 1 s with about 100 MB RSS. The type is never used, and `-Onone` compiles it that way.
- **Actual**: `swift-frontend` grows to about 32 GiB RSS (2^32 × 8 B) and spins. The watchdog kills it at 6–7 GB within 4–10 s.

Environment: Apple Swift 6.4 (`swiftlang-6.4.0.34.1`, swift-driver 1.168.6), arm64-apple-macosx27.2.0, 64 GB RAM.

## Observed output (`./run.sh`)

```
Apple Swift version 6.4 (swiftlang-6.4.0.34.1 clang-2100.3.34.1)
--- repro.swift (endIndex = 2^32), -O, last SIL passes before kill:
result=KILLED rss>6291456KB exit=137 elapsed=10s peak_rss=7112MB
  Run #30422, stage LowLevel,Function, pass 61: DeadObjectElimination (deadobject-elim), Function: $ss32_copyCollectionToContiguousArrayys0dE0Vy7ElementQzGxSlRzlF5repro9HugeBytesV_Tg5
  Run #30423, stage LowLevel,Function, pass 62: ObjectOutliner (object-outliner), Function: $ss32_copyCollectionToContiguousArrayys0dE0Vy7ElementQzGxSlRzlF5repro9HugeBytesV_Tg5
--- same file, -Onone (control):
result=ok exit=0 elapsed=1s peak_rss=3MB
--- linear scaling (peak RSS from /usr/bin/time -l):
endIndex = 1<<20  peak RSS   101 MB
endIndex = 1<<24  peak RSS   221 MB
endIndex = 1<<26  peak RSS   605 MB
endIndex = 1<<28  peak RSS  2141 MB
```

Going from 2^26 to 2^28 adds 1536 MB for 201 M extra elements, which is exactly 8 bytes per element.

Variant matrix (`variants.sh`; values come from `/usr/bin/time -l` and the watchdog):

| variant | result |
|---|---|
| `RandomAccessCollection`, computed `endIndex` 2^32, `-O` / `-Osize` | blows up |
| same, `-Onone` | fine |
| `Collection` or `BidirectionalCollection` (own `index(after:)`/`index(before:)`), 2^32, `-O` | fine |
| stored `let endIndex` (with or without a default value) | fine |
| `Int.max`, `1 << 62`, `-1` | fine: the count is not a literal by the time the outliner runs |
| `1<<24` / `1<<28` / ≥ `1<<30` | 220 MB / 2.1 GB / killed |
| `[UInt8](repeating: 0, count: 1 << 32)`, `reserveCapacity(1 << 32)` | fine: no inlined literal `alloc_ref` |

## Root cause

The conformance's `Sequence._copyToContiguousArray` witness specializes `_copyCollectionToContiguousArray<HugeBytes>`. After inlining and constant folding, the specialization contains the following (seen at `1<<16` with `-emit-sil`):

```
%2 = integer_literal $Builtin.Word, 65536
%3 = alloc_ref [tail_elems $UInt8 * %2] $_ContiguousArrayStorage<UInt8>
... loop of `store` through index_addr ... end_cow_mutation %4
```

ObjectOutliner (`SwiftCompilerSources/Sources/Optimizer/FunctionPasses/ObjectOutliner.swift`, unchanged on `main` as of commit 40567aac60, 2026-11-12) handles this in `getInitialization(of:ignore:_:)`:

```swift
guard let numTailElements = allocRef.numTailElements else { return nil }   // literal count, no upper bound
...
let tailCount = numTailElements != 0 ? numTailElements * allocRef.numStoresPerTailElement : 0
var tailStores = Array<StoreInst?>(repeating: nil, count: tailCount)       // 8 B × N, eagerly
if !findInitStores(...) { return nil }                                      // then bails: stores are in a loop
```

`numTailElements` returns any `IntegerLiteralInst` count without a limit. The pass therefore allocates (and nil-fills) an N-entry side table before it checks that the object even has N individual constant-index stores, and it throws the table away when it finds none. An object with 2^32 tail elements can never be outlined into a static initializer anyway. `numTailElements * numStoresPerTailElement` also has no overflow guard, so a large enough literal times a tuple width would trap in the compiler.

Suggested fix: bail out early when `numTailElements` (times `numStoresPerTailElement`) exceeds a sane limit, for example a few thousand elements, or the number of uses of the `alloc_ref`/`ref_tail_addr`. Alternatively, collect stores into a dictionary keyed by index and allocate only after the stores have been found.

`-Xfrontend -debug-time-function-bodies` does not help here because it only times type checking. `-Xllvm -sil-print-pass-name` (above) and `sample` (main thread inside `SILPassManager::runFunctionPasses`) pinpoint the pass.

## Existing reports (searched 2026-11-27)

None match. I searched issues for ObjectOutliner, "out of memory", large tail counts, and `_copyCollectionToContiguousArray`, and checked the ObjectOutliner PRs and the file's commit history on `main`:

- https://forums.swift.org/t/consistency-of-the-objectoutliner-siloptimizer-pass/53764: ObjectOutliner memory blowup with large nested literals. Related, but a different trigger (an array literal with many elements, not one constant-count allocation).
- https://github.com/swiftlang/swift/issues/51762: slow compile of a 70k-element literal. Different cause.
- ObjectOutliner PRs #93302, #92181, #91097, #88178 and #73706 do not touch the count bound.

Caveat: `gh search issues` hit the GitHub API rate limit partway through the search, so the plain query "ObjectOutliner" over issues was covered only by web search. Re-run `gh search issues ObjectOutliner -R swiftlang/swift` before filing.

## Draft upstream report

- **Where**: https://github.com/swiftlang/swift/issues/new?template=bug.yml (template "Bug Report"). Suggested labels: `bug`, `compiler`, `SILOptimizer`, `performance` (compile-time memory).
- **Title**: `ObjectOutliner allocates 8 bytes per tail element of a constant-count alloc_ref (32 GB for a Collection with endIndex 2^32)`
- **Body**:

> ### Description
> With `-O`/`-Osize`, `swift-frontend` uses memory linear in a *compile-time constant* collection size and reaches about 32 GB for 2^32. The cause is `ObjectOutliner.getInitialization`, which eagerly allocates `Array<StoreInst?>(repeating: nil, count: numTailElements)` for any `alloc_ref [tail_elems $T * <integer_literal>]`, with no upper bound, before it checks for constant-index stores. Here the specialized `_copyCollectionToContiguousArray<HugeBytes>` (from the default `_copyToContiguousArray` witness) has such an `alloc_ref` with count 2^32. The type does not even need to be used.
>
> ### Reproduction
> ```swift
> struct HugeBytes: RandomAccessCollection {
>     var startIndex: Int { 0 }
>     var endIndex: Int { Int(UInt32.max) + 1 }
>     subscript(_: Int) -> UInt8 { 0 }
> }
> ```
> `swiftc -O -parse-as-library -emit-object repro.swift`, then watch RSS. Peak RSS by `endIndex`: 1<<20 → 101 MB, 1<<24 → 221 MB, 1<<26 → 605 MB, 1<<28 → 2141 MB, 2^32 → about 32 GB (8 B per element). `-Xllvm -sil-print-pass-name` shows the last pass as `ObjectOutliner` on `$ss32_copyCollectionToContiguousArray…HugeBytesV_Tg5`. `-Onone`, a stored `let endIndex`, or `Collection`/`BidirectionalCollection` conformances compile instantly.
>
> ### Expected behavior
> Compile time and memory independent of the constant value. ObjectOutliner should bail out early for tail counts that are too large to outline (and guard `numTailElements * numStoresPerTailElement` against overflow).
>
> ### Environment
> Apple Swift version 6.4 (swiftlang-6.4.0.34.1 clang-2100.3.34.1), Target: arm64-apple-macosx27.2.0. The code path is unchanged on `main` (`SwiftCompilerSources/Sources/Optimizer/FunctionPasses/ObjectOutliner.swift`, `getInitialization` / `numTailElements`).
>
> ### Additional information
> Workaround: make the size non-constant (a stored property).

File a new issue; there is no existing issue to comment on.
