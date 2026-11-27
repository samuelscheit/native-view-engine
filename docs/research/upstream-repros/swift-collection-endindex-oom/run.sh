#!/usr/bin/env bash
# Minimal repro: swift-frontend -O allocates 8 bytes per tail element in ObjectOutliner.
# Every compile runs under watchdog.sh (kill at 6 GB RSS or 120 s); runs are sequential.
set -u; cd "$(dirname "$0")"; mkdir -p out
swiftc --version 2>&1 | head -1
echo "--- repro.swift (endIndex = 2^32), -O, last SIL passes before kill:"
./watchdog.sh 6291456 120 -- swiftc -O -parse-as-library -emit-object repro.swift -o out/repro.o \
  -Xllvm -sil-print-pass-name > out/passes.txt 2>&1
tail -1 out/passes.txt; grep -v '^result=' out/passes.txt | tail -2
echo "--- same file, -Onone (control):"
./watchdog.sh 6291456 120 -- swiftc -Onone -parse-as-library -emit-object repro.swift -o out/repro-onone.o
echo "--- linear scaling (peak RSS from /usr/bin/time -l):"
for e in 20 24 26 28; do
  sed "s/Int(UInt32.max) + 1/1 << $e/" repro.swift > out/scale$e.swift
  /usr/bin/time -l swiftc -O -parse-as-library -emit-object out/scale$e.swift -o out/scale$e.o 2> out/time$e.txt
  printf 'endIndex = 1<<%s  peak RSS %5d MB\n' $e $(( $(awk '/maximum resident/{print $1}' out/time$e.txt) / 1048576 ))
done
