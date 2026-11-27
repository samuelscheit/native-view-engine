#!/usr/bin/env bash
# Native test suites of the five static host runtimes (`packages/core/host-*`).
#
# Sourced by `scripts/check-all.sh` (which records each suite), or run standalone:
#   scripts/host-runtimes.sh [cxx|swift|java|dart|ts|all]   (default: all)
# Build output goes to $BUILD_ROOT (a fresh temp dir when unset); nothing is written to the repo.

HOST_RT_ROOT="${REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# Required tool per suite (`check-all.sh` uses it to fail or skip clearly when missing).
host_runtime_tool() {
  case "$1" in
    cxx) printf 'cmake' ;;
    swift) printf 'swift' ;;
    java) printf 'javac' ;;
    dart) printf 'dart' ;;
    ts) printf 'node' ;;
  esac
}

host_runtime_has_tools() {
  command -v "$(host_runtime_tool "$1")" >/dev/null 2>&1 || return 1
  case "$1" in
    java) command -v java >/dev/null 2>&1 ;;
    dart) command -v node >/dev/null 2>&1 ;;  # JS parity run
    *) return 0 ;;
  esac
}

# C11 + C++20 (ASan/UBSan, -Werror) and the negative-compile guards.
suite_host_cxx() {
  local dir="$BUILD_ROOT/host-cxx"
  local gen=()
  command -v ninja >/dev/null 2>&1 && gen=(-G Ninja)
  cmake -S "$HOST_RT_ROOT/packages/core/host-cxx" -B "$dir" ${gen[@]+"${gen[@]}"} -DCMAKE_BUILD_TYPE=Debug
  cmake --build "$dir"
  ctest --test-dir "$dir" --output-on-failure
}

# Swift 6 strict concurrency, warnings as errors; plus a 32-bit `arm64_32` compile when the watchOS SDK exists.
suite_host_swift() {
  local pkg="$HOST_RT_ROOT/packages/core/host-swift"
  swift test --package-path "$pkg" --scratch-path "$BUILD_ROOT/host-swift" -Xswiftc -warnings-as-errors
  if command -v xcrun >/dev/null 2>&1 && xcrun --sdk watchos --show-sdk-path >/dev/null 2>&1; then
    xcrun --sdk watchos swiftc -target arm64_32-apple-watchos9.0 -swift-version 6 -warnings-as-errors \
      -parse-as-library -module-name NveHost -wmo -c "$pkg"/Sources/NveHost/*.swift \
      -o "$BUILD_ROOT/host-swift-arm64_32.o"
  else
    echo "note: watchOS SDK not installed; arm64_32 compile check skipped"
  fi
}

# `javac --release 11 -Xlint:all -Werror`, then the plain-main suite.
suite_host_java() {
  local out="$BUILD_ROOT/host-java"
  mkdir -p "$out"
  # shellcheck disable=SC2046
  javac --release 11 -Xlint:all -Werror -d "$out" $(find "$HOST_RT_ROOT/packages/core/host-java/src" -name '*.java')
  java -cp "$out" dev.nve.host.HostTestMain
}

# Analyzer (fatal infos, avoid_js_rounded_ints), VM suite, then the same suite compiled to JS on node.
suite_host_dart() {
  local pkg="$HOST_RT_ROOT/packages/core/host-dart"
  (
    cd "$pkg"
    dart pub get --offline
    dart analyze --fatal-infos
    dart run --enable-asserts test/host_test.dart
    dart compile js -O2 -o "$BUILD_ROOT/host-dart-web.js" test/web_parity_entry.dart
    node "$BUILD_ROOT/host-dart-web.js"
  )
  rm -rf "$pkg/.dart_tool" "$pkg/pubspec.lock"
}

# node:test suites, then `tsc --noEmit` (PATH `tsc`, else TypeScript via npx).
suite_host_ts() {
  local pkg="$HOST_RT_ROOT/packages/core/host-ts"
  node --test "$pkg"/test/*.test.ts
  if command -v tsc >/dev/null 2>&1; then
    tsc --noEmit -p "$pkg/tsconfig.json"
  else
    npx -y -p typescript@5.9 tsc --noEmit -p "$pkg/tsconfig.json"
  fi
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  set -euo pipefail
  BUILD_ROOT="${BUILD_ROOT:-$(mktemp -d "${TMPDIR:-/tmp}/nve-host-runtimes.XXXXXX")}"
  suites=("${1:-all}")
  [ "${suites[0]}" = "all" ] && suites=(cxx swift java dart ts)
  for s in "${suites[@]}"; do
    if ! host_runtime_has_tools "$s"; then
      echo "error: host-$s needs '$(host_runtime_tool "$s")' (and node for dart/ts, java for java)" >&2
      exit 1
    fi
    printf '\n== host-%s ==\n' "$s"
    "suite_host_$s"
  done
fi
