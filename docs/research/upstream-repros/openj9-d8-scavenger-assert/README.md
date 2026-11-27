# OpenJ9: instance `<init>` receiver slot forced live → GC scans an `int` as an object (d8 crash)

**Verdict: genuine upstream bug in OpenJ9. Present on latest release 0.63.0 and on `master` @ `5f66f8ce3c` (2026-11-26). Not Homebrew-specific. Not reported upstream.**

## Root cause

`runtime/stackmap/localmap.c` (`j9localmap_LocalBitsForPC`, lines 481-484 on master):

```c
/* Ensure that the receiver is marked for all <init>()V methods */
if ((J9_ARE_NO_BITS_SET(romMethod->modifiers, J9AccStatic)) && ('<' == J9UTF8_DATA(J9ROMMETHOD_NAME(romMethod))[0])) {
    *resultArrayBase |= 1;
}
```

This code ORs bit 0 into the interpreter's object-slot map for every instance `<init>`, ignoring the liveness and type analysis that `mapAllLocals` just did. JVMS 4.10.1 allows a constructor to store a primitive into local 0 after `invokespecial super.<init>`: local types are not fixed, and HotSpot verifies and runs such code. R8/D8 does this in `com.android.tools.r8.internal.n30.<init>(I)V` (a fastutil-style map): `... dup; istore_0; anewarray ...`. When a GC happens at that `anewarray`, the stack walker hands the int in slot 0 to the GC as an object reference, and `MM_StackSlotValidator` asserts (`j9mm.479`).

- The line was introduced by eclipse-openj9/openj9#14419 (commit `cc5e11f7b6`, 2022, "Mark receivers of all constructors as oslots") for `J9_STACKWALK_HIDE_EXCEPTION_FRAMES`. The review there called it "completely safe", assuming slot 0 always still holds the receiver. That assumption is false for legal bytecode.
- Which assertion fires depends on the garbage value. In d8 the old pointer's high word was still in the slot (`0x0000007000000401`, inside the heap), giving `ScavengerRootScanner.hpp:105 COULD_BE_FORWARDED` "Misaligned object". In the fixture the value is `0x401`, giving `:109`/`:108 NOT_ON_HEAP` "Object neither in heap nor stack-allocated". Under optthruput it fires in `MarkingSchemeRootMarker.cpp:52`, under balanced in `CopyForwardScheme.cpp`. If the int happened to look like a valid aligned heap address, the result would be silent heap corruption instead of an assertion.
- The bug is in the interpreter only (JIT frames use their own GC maps). `-Xint` does not avoid it, and no GC policy avoids it.

## Fixture

- `Gen.java` uses `java.lang.classfile` (JDK 24+) to emit `Victim.class` (v52) with `Victim(int n){ super(); istore_0 ← n; loop 1e6 × new Object[slot0]; }`.
- `run.sh`: `JAVA=<openj9 java> [HOTSPOT_JAVA=<hotspot java>] ./run.sh`. It sets `ulimit -c 0` and `-Xdump:none` and wraps each run in `timeout 120`; outputs go to `out/` (gitignored).

Expected: `OK: constructor finished` (as on HotSpot). Actual: assertion on the first GC, exit 255. Failure rate: **100%** under every option tested.

### Observed (2026-11-26, macOS 26 arm64)

Homebrew `openj9 0.59.0` (`25.0.3-openj9-0.59.0`, OpenJ9/OMR `e4aaece3226`):

```
--- java -Xdump:none -Xmn4m -Xint
Object neither in heap nor stack-allocated in thread main
	O-Slot value=0000000000000401
	method=000000751265C030 (Victim.<init>(I)V) (Interpreted)
j9mm.479 * ** ASSERTION FAILED ** at .../openj9-openj9-0.59.0/openj9/runtime/gc_glue_java/ScavengerRootScanner.hpp:109: ((MM_StackSlotValidator(MM_StackSlotValidator::NOT_ON_HEAP, *slotPtr, stackLocation, walkState).validate(_env)))
exit=255
--- (default)                  same, ScavengerRootScanner.hpp:109, exit=255
--- -Xgcpolicy:optthruput      MarkingSchemeRootMarker.cpp:52, exit=255
--- -Xgcpolicy:balanced        gc_vlhgc/CopyForwardScheme.cpp:3892, exit=255
--- -XX:-UseCompressedOops     ScavengerRootScanner.hpp:109, exit=255
Temurin 21.0.11 (HotSpot) -Xverify:all -Xint / default:  OK: constructor finished, exit=0
```

IBM Semeru 25.0.5.0 / **OpenJ9 0.63.0** (latest release, official build, `dcc7ee16aa`/OMR `c2a3a9ae0`): identical, with asserts at `ScavengerRootScanner.hpp:108`, `MarkingSchemeRootMarker.cpp:52` and `CopyForwardScheme.cpp:3826`, exit 255 in all 5 configurations.

Original d8 trigger (Homebrew 0.59.0, `d8 --min-api 24 --lib android-36.1/android.jar` on this repo's 5 `NveMainActivity*.class` files): 6/25 failures by default, 12/30 with `-Xint`, 13/30 with `-Xnoaot`, 8/30 with `-Xshareclasses:none`. The failure is intermittent because a GC must land inside `n30.<init>`:

```
Misaligned object in thread ForkJoinPool-1-worker-3
	O-Slot value=0000007000000401
	method=... (com/android/tools/r8/internal/n30.<init>(I)V) (Interpreted)
j9mm.479 ** ASSERTION FAILED ** at .../ScavengerRootScanner.hpp:105: ((MM_StackSlotValidator(MM_StackSlotValidator::COULD_BE_FORWARDED, ...
```

`-Xgcpolicy:optthruput` and `balanced` gave 0/30 on d8 only because those short runs rarely collect while that frame is live. The fixture shows both still crash, so the `-JXgcpolicy:optthruput` fallback in `hosts/android-shell/run-emulator.sh` only lowers the odds; switching to HotSpot is the real mitigation.

## Existing reports (searched GitHub issues/PRs, web)

None with this cause. Same assertion text but different, unrelated causes: #18098 (JIT PRE/`aselect`), #22163 (invalid `Unsafe` test), #15251, #16510, #24451 (OMR codegen, fixed). #8823 is R8 local-slot reuse with a different symptom. Origin: PR #14419.

## Draft upstream report

- **Where:** https://github.com/eclipse-openj9/openj9/issues/new/choose → "Bug report" template (sections: Java -version output / Summary of problem / Diagnostic files). Suggested labels: `comp:vm` (stackmap), `userRaised`. File a new issue rather than comment on an existing one: there is no matching issue. Mention @tajila / @gacholio (authors/reviewers of #14419).
- **Title:** `Interpreter local map marks slot 0 of every <init> as an object even after istore_0; GC asserts (StackSlotValidator) — crashes Android R8/D8`
- **Body:**

````markdown
Java -version output
--------------------
```
openjdk version "25.0.5.0.3" 2026-10-24 LTS
IBM Semeru Runtime Open Edition 25.0.5.0 (build 25.0.5.0.3+1-LTS)
Eclipse OpenJ9 VM 25.0.5.0 (build 25.0.5.0.3+1-openj9-0.63.0, JRE 25 Mac OS X aarch64-64-Bit 20261024_368 (JIT enabled, AOT enabled)
OpenJ9   - dcc7ee16aa
OMR      - c2a3a9ae0
JCL      - 3edbf43d7f7 based on jdk-25.0.5.0.3+1)
```
Also reproduced with 0.59.0 (Homebrew, JDK 25.0.3). The code is unchanged on master (5f66f8ce3c).

Summary of problem
------------------
`j9localmap_LocalBitsForPC` (runtime/stackmap/localmap.c) unconditionally sets bit 0 for every non-static `<init>`:

```c
/* Ensure that the receiver is marked for all <init>()V methods */
if ((J9_ARE_NO_BITS_SET(romMethod->modifiers, J9AccStatic)) && ('<' == J9UTF8_DATA(J9ROMMETHOD_NAME(romMethod))[0])) {
    *resultArrayBase |= 1;
}
```

(added in #14419). After `invokespecial super.<init>`, a constructor may legally store a primitive into local 0 (JVMS 4.10.1; HotSpot with `-Xverify:all` accepts and runs it). Android's R8/D8 emits exactly this in `com.android.tools.r8.internal.n30.<init>(I)V` (`dup; istore_0; anewarray`). When a GC happens in such a frame, the interpreter stack walk reports the int as an object slot:

```
Object neither in heap nor stack-allocated in thread main
	O-Slot value=0000000000000401
	method=... (Victim.<init>(I)V) (Interpreted)
j9mm.479 * ** ASSERTION FAILED ** at .../runtime/gc_glue_java/ScavengerRootScanner.hpp:108: ((MM_StackSlotValidator(MM_StackSlotValidator::NOT_ON_HEAP, *slotPtr, stackLocation, walkState).validate(_env)))
```

In d8 (Android build-tools 36.0.0) the stale high word of `this` makes it `Misaligned object ... (com/android/tools/r8/internal/n30.<init>(I)V) (Interpreted)` → `ScavengerRootScanner.hpp:105 COULD_BE_FORWARDED`, intermittently (~25% of `d8` runs). It reproduces with gencon, optthruput (`MarkingSchemeRootMarker.cpp:52`) and balanced (`CopyForwardScheme.cpp:3826`), with and without `-Xint` and with `-XX:-UseCompressedOops`. If the int looks like a valid aligned heap address, the result would presumably be silent heap corruption.

Minimal reproducer (100% failure rate), generator using `java.lang.classfile`:

```java
<Gen.java contents>
```
```
java Gen.java Victim.class && java -Xmn4m -cp . Victim
```
Expected: `OK: constructor finished` (HotSpot 21: OK). Actual: the assertion above, exit 255.

Suggested fix: only force the receiver bit when slot 0 still holds the receiver at `pc`. For example, OR in bit 0 only if the method never stores to local 0 (track `astore_0`/`istore_0`/`lstore_0`/`fstore_0`/`dstore_0`/`istore 0`/`iinc 0`-style writes in ClassFileOracle, similar to existing method flags). Alternatively, have the `HIDE_EXCEPTION_FRAMES` check tolerate a non-object A0 instead of keeping it alive through the GC map.

Diagnostic files
----------------
Self-contained reproducer above; javacore available on request.
````

(When filing, paste `Gen.java` verbatim where `<Gen.java contents>` is.)
