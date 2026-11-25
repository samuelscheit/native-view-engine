#!/usr/bin/env python3
"""Sort a wasm module's defined functions by body size (one `wasm-tools print`/`parse` round trip).

Placing similar-sized (and therefore often similar-shaped) functions next to each other is worth
~1 KB brotli / ~22 KB gzip on the web-shell bundle (docs/research/wasm-size/toolchain.md). Moving
`(func ...)` blocks is semantics-preserving only because the printed text references functions
symbolically (`$name`), so the input must still carry its `name` section (wasm-bindgen output
without --remove-name-section); a module with an unnamed defined function is rejected. Run
wasm-opt afterwards to strip the names; its pass list must not reorder functions again.

    wasm-sort-funcs.py IN.wasm OUT.wasm
"""
import re
import subprocess
import sys


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__.strip().splitlines()[-1].strip())
    src, dst = sys.argv[1], sys.argv[2]
    wat = subprocess.run(["wasm-tools", "print", src], check=True, capture_output=True, text=True).stdout
    lines = wat.split("\n")
    named = re.compile(r'^  \(func \$(?:"(?:[^"\\]|\\.)*"|\S+) ')
    quoted = re.compile(r'"(?:[^"\\]|\\.)*"')
    pre, funcs, post, i = [], [], [], 0
    while i < len(lines):
        line = lines[i]
        if not line.startswith("  (func "):
            (post if funcs else pre).append(line)
            i += 1
            continue
        if not named.match(line):
            sys.exit(f"wasm-sort-funcs: unnamed function, cannot reorder safely: {line[:80]}")
        j = i
        unquoted = quoted.sub('""', line)
        if unquoted.count("(") != unquoted.count(")"):  # multi-line block, else `(func ... )` one-liner
            while lines[j] != "  )":
                j += 1
        funcs.append(lines[i : j + 1])
        i = j + 1
    funcs.sort(key=len)  # stable: equal sizes keep link order
    out = pre + [line for block in funcs for line in block] + post
    subprocess.run(["wasm-tools", "parse", "-o", dst, "-"], input="\n".join(out), check=True, text=True)
    print(f"sorted {len(funcs)} functions by size", file=sys.stderr)


if __name__ == "__main__":
    main()
