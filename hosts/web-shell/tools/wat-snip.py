#!/usr/bin/env python3
"""Post-link wasm rewrite for the web-shell size pipeline (one `wasm-tools print`/`parse` round trip).

1. Snip: replace the bodies of functions whose *name* matches a regex with `unreachable`
   (wasm-snip 0.4.0 / walrus cannot parse externref modules, which wasm32-unknown-unknown emits
   by default since Rust 1.82, so this works on the text form instead).
2. Order: sort defined functions by body size. Placing similar-sized (and therefore often
   similar-shaped) functions next to each other measurably helps brotli/gzip/zstd; references in
   the printed text are symbolic ($name), so moving `(func ...)` blocks is semantics-preserving.

Input must still have its `name` section (wasm-bindgen output without --remove-name-section).
Run wasm-opt afterwards: it removes the callees that became unreachable and strips the names
(its pass list must not reorder functions).

`--keep REGEX` is a safety net: if a snip pattern matches a function whose name also matches a
keep pattern, the tool fails instead of turning a live path (e.g. `Backtrace::capture`, which
every `anyhow::Error` construction calls) into a trap.

    wat-snip.py IN.wasm OUT.wasm [--no-sort] [--keep REGEX]... REGEX [REGEX...]
"""
import re
import subprocess
import sys


def alternation(patterns: list[str]) -> re.Pattern[str] | None:
    return re.compile("|".join(f"(?:{p})" for p in patterns)) if patterns else None


def main() -> None:
    args = sys.argv[1:]
    src, dst, rest = args[0], args[1], args[2:]
    sort, keep, patterns = True, [], []
    while rest:
        arg = rest.pop(0)
        if arg == "--no-sort":
            sort = False
        elif arg == "--keep":
            keep.append(rest.pop(0))
        else:
            patterns.append(arg)
    rx, keep_rx = alternation(patterns), alternation(keep)
    wat = subprocess.run(["wasm-tools", "print", src], check=True, capture_output=True, text=True).stdout
    lines = wat.split("\n")
    head = re.compile(r'^  \(func \$(?:"((?:[^"\\]|\\.)*)"|(\S+)) ')
    quoted = re.compile(r'"(?:[^"\\]|\\.)*"')
    pre, funcs, post, snipped, i = [], [], [], 0, 0
    while i < len(lines):
        m = head.match(lines[i])
        if not m:
            (post if funcs else pre).append(lines[i])
            i += 1
            continue
        unquoted = quoted.sub('""', lines[i])
        j = i
        if unquoted.count("(") != unquoted.count(")"):  # multi-line block, else `(func ... )` one-liner
            while lines[j] != "  )":
                j += 1
        block = lines[i : j + 1]
        name = m.group(1) or m.group(2)
        if rx and rx.search(name):
            if keep_rx and keep_rx.search(name):
                sys.exit(f"wat-snip: refusing to snip live function {name}")
            block = [lines[i], "    unreachable", "  )"]
            snipped += 1
        funcs.append(block)
        i = j + 1
    if sort:
        funcs.sort(key=len)  # stable: equal sizes keep link order
    out = pre + [line for block in funcs for line in block] + post
    subprocess.run(["wasm-tools", "parse", "-o", dst, "-"], input="\n".join(out), check=True, text=True)
    print(f"snipped {snipped} functions, sorted {len(funcs) if sort else 0}", file=sys.stderr)


if __name__ == "__main__":
    main()
