#!/usr/bin/env bash
# Runs the repro against crates.io dioxus-core 0.7.10 and DioxusLabs/dioxus main (dioxus-vdom).
set -u
cd "$(dirname "$0")"
mkdir -p out
for feat in v0710 main; do
  for profile in debug release; do
    flags=(--no-default-features --features "$feat"); [ "$profile" = release ] && flags+=(--release)
    cargo build -q "${flags[@]}" 2>&1 | tail -20
    for mode in runtime-spawn in-scope-spawn; do
      echo "=== $feat $profile $mode"
      RUST_BACKTRACE=0 timeout 30 cargo run -q "${flags[@]}" -- "$mode" 2>&1 | grep -v '^note:'
      echo "exit=${PIPESTATUS[0]}"
    done
  done
done 2>&1 | tee out/run.log
