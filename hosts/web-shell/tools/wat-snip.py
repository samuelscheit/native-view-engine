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

    wat-snip.py IN.wasm OUT.wasm [--no-sort] REGEX [REGEX...]
"""
import re
import subprocess
import sys


def main() -> None:
    args = sys.argv[1:]
    src, dst = args[0], args[1]
    sort = "--no-sort" not in args
    patterns = [a for a in args[2:] if a != "--no-sort"]
    rx = re.compile("|".join(f"(?:{p})" for p in patterns)) if patterns else None
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
        if rx and rx.search(m.group(1) or m.group(2)):
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
