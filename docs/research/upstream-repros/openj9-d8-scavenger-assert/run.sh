#!/usr/bin/env bash
# Repro: OpenJ9 interpreter local map forces slot 0 of every instance <init> to be an object
# reference, even after the constructor stored a primitive into it (istore_0). First GC in that
# frame scans the int as an object -> j9mm.479 StackSlotValidator assertion.
#
# Env:
#   JAVA         OpenJ9 java to test (default: java on PATH). Must be JDK 24+ (generator uses java.lang.classfile).
#   HOTSPOT_JAVA optional HotSpot java (any version >= 8) for the comparison run.
set -uo pipefail
cd "$(dirname "$0")"
JAVA="${JAVA:-java}"
OUT=out; mkdir -p "$OUT"
ulimit -c 0                                   # no multi-GB core files

"$JAVA" -version 2>&1 | sed 's/^/# /'
"$JAVA" Gen.java "$OUT/Victim.class" || exit 1

run() { echo "--- $*"; timeout 120 "$@" -cp "$OUT" Victim 2>&1 | grep -E 'OK:|ASSERTION|in thread|O-Slot value|method='; echo "exit=${PIPESTATUS[0]}"; }

for opts in "-Xint" "" "-Xgcpolicy:optthruput" "-Xgcpolicy:balanced" "-XX:-UseCompressedOops"; do
  # shellcheck disable=SC2086
  run "$JAVA" -Xdump:none -Xmn4m $opts
done

if [[ -n "${HOTSPOT_JAVA:-}" ]]; then
  "$HOTSPOT_JAVA" -version 2>&1 | sed 's/^/# /'
  run "$HOTSPOT_JAVA" -Xverify:all -Xint -Xmn4m
  run "$HOTSPOT_JAVA" -Xverify:all -Xmn4m
fi
