#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Prefer a HotSpot/JBR JDK over Eclipse OpenJ9 (see docs/UPSTREAM.md).
is_openj9() {
  local java_bin="$1"
  "$java_bin" -version 2>&1 | grep -qi "OpenJ9"
}

if [ -z "${JAVA_HOME:-}" ] || is_openj9 "${JAVA_HOME}/bin/java"; then
  for candidate in \
    "/Applications/Android Studio.app/Contents/jbr/Contents/Home" \
    "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home" \
    "/Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home"; do
    if [ -x "$candidate/bin/java" ] && ! is_openj9 "$candidate/bin/java"; then
      export JAVA_HOME="$candidate"
      break
    fi
  done
fi

if [ -n "${JAVA_HOME:-}" ]; then
  export PATH="$JAVA_HOME/bin:$PATH"
fi

if [ -z "${ANDROID_HOME:-}" ] && [ -z "${ANDROID_SDK_ROOT:-}" ]; then
  if [ -d "$HOME/Library/Android/sdk" ]; then
    export ANDROID_HOME="$HOME/Library/Android/sdk"
  elif [ -d "$HOME/Android/Sdk" ]; then
    export ANDROID_HOME="$HOME/Android/Sdk"
  fi
fi

if [ "$#" -eq 0 ]; then
  exec ./gradlew testDebugUnitTest --stacktrace
else
  exec ./gradlew "$@"
fi
