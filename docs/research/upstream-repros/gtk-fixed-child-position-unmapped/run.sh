#!/bin/sh
# Builds and runs the repro against pkg-config gtk4. Needs a display (macOS: native backend works).
set -eu
cd "$(dirname "$0")"
mkdir -p out
cc -std=c11 -Wall -o out/repro repro.c $(pkg-config --cflags --libs gtk4)
echo "pkg-config gtk4: $(pkg-config --modversion gtk4)"
exec out/repro
