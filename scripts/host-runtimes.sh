#!/usr/bin/env bash
# Native test suites of the five static host runtimes (`packages/core/host-*`).
#
# Sourced by `scripts/check-all.sh` (which records each suite), or run standalone:
#   scripts/host-runtimes.sh [cxx|swift|java|dart|ts|all]   (default: all)
# Build output goes to $BUILD_ROOT (a fresh temp dir when unset); nothing is written to the repo.

HOST_RT_ROOT="${REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

HOST_RUNTIME_SUITES="cxx swift java dart ts"

# Prints the first binary suite `$1` needs that is not on PATH (status 0), or nothing (status 1).
host_runtime_missing_tool() {
  local tool
  case "$1" in
    cxx) set -- cmake ctest ;;
    swift) set -- swift ;;
    java) set -- javac java ;;
    dart) set -- dart node ;;  # node runs the dart2js parity build
    ts)
      command -v node >/dev/null 2>&1 || { printf 'node'; return 0; }
      command -v tsc >/dev/null 2>&1 || command -v npx >/dev/null 2>&1 || { printf 'tsc or npx'; return 0; }
      return 1
      ;;
    *)
      printf 'unknown suite %s' "$1"
      return 0
      ;;
  esac
  for tool in "$@"; do
    command -v "$tool" >/dev/null 2>&1 || { printf '%s' "$tool"; return 0; }
  done
  return 1
}

host_runtime_has_tools() {
  ! host_runtime_missing_tool "$1" >/dev/null
}

# Every suite tests against generated ABI / table / XXH64 vector files (`nve codegen abi`); they
# are gitignored, so generate them first. `scripts/check-all.sh` runs its ABI codegen suite instead.
host_runtime_generate() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is needed to generate the host ABI files (nve codegen abi)" >&2
    return 1
  fi
  (cd "$HOST_RT_ROOT" && cargo run --locked -q -p nve-cli -- codegen abi >/dev/null)
}

# Java sources under `$1`, NUL-separated (safe for paths with spaces or newlines).
host_runtime_java_sources() {
  find "$1" -name '*.java' -print0
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

# Swift 6 strict concurrency, warnings as errors, debug and release (the allocation guard only means
# something optimized); plus a 32-bit `arm64_32` compile when the watchOS SDK exists.
suite_host_swift() {
  local pkg="$HOST_RT_ROOT/packages/core/host-swift"
  swift test --package-path "$pkg" --scratch-path "$BUILD_ROOT/host-swift" -Xswiftc -warnings-as-errors
  swift test -c release --package-path "$pkg" --scratch-path "$BUILD_ROOT/host-swift" -Xswiftc -warnings-as-errors
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
  local out="$BUILD_ROOT/host-java" src
  local sources=()
  mkdir -p "$out"
  while IFS= read -r -d '' src; do sources+=("$src"); done \
    < <(host_runtime_java_sources "$HOST_RT_ROOT/packages/core/host-java/src")
  javac --release 11 -Xlint:all -Werror -d "$out" "${sources[@]}"
  java -cp "$out" dev.nve.host.HostTestMain
}

# Analyzer (fatal infos, avoid_js_rounded_ints), VM suite, then the same suite compiled to JS on node.
# Runs in a source copy under $BUILD_ROOT: pub writes `.dart_tool/` and `pubspec.lock`, and the
# developer's own copies in the repo must stay untouched.
suite_host_dart() {
  local work="$BUILD_ROOT/host-dart"
  rm -rf "$work"
  mkdir -p "$work"
  (cd "$HOST_RT_ROOT/packages/core/host-dart" && tar cf - --exclude .dart_tool --exclude pubspec.lock --exclude build .) \
    | (cd "$work" && tar xf -)
  (
    cd "$work"
    dart pub get --offline
    dart analyze --fatal-infos
    dart run --enable-asserts test/host_test.dart
    dart compile js -O2 -o "$BUILD_ROOT/host-dart-web.js" test/web_parity_entry.dart
    node "$BUILD_ROOT/host-dart-web.js"
  )
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
  if [ -z "${BUILD_ROOT:-}" ]; then
    BUILD_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nve-host-runtimes.XXXXXX")"
    trap 'rm -rf "$BUILD_ROOT"' EXIT  # we created it, so we remove it (also on failure)
  fi
  [ "$#" -eq 0 ] && set -- all
  suites=()
  for arg in "$@"; do
    if [ "$arg" = "all" ]; then
      # shellcheck disable=SC2206
      suites+=($HOST_RUNTIME_SUITES)
    else
      # Exact names only: a substring test would accept "cxx swift" as one argument.
      case "$arg" in
        cxx | swift | java | dart | ts) suites+=("$arg") ;;
        *)
          echo "error: unknown suite '$arg' (expected: $HOST_RUNTIME_SUITES all)" >&2
          exit 2
          ;;
      esac
    fi
  done
  for s in "${suites[@]}"; do
    if missing="$(host_runtime_missing_tool "$s")"; then
      echo "error: host-$s needs '$missing', which is not on PATH" >&2
      exit 1
    fi
  done
  host_runtime_generate
  for s in "${suites[@]}"; do
    printf '\n== host-%s ==\n' "$s"
    "suite_host_$s"
  done
fi
