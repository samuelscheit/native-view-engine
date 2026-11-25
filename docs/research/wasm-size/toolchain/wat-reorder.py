#!/usr/bin/env python3
"""Experiment: reorder defined functions to improve compressibility.

    wat-reorder.py IN.wasm OUT.wasm MODE      MODE: name | size | body | shuffle

References in `wasm-tools print` output are symbolic ($name) when the name section is present,
so moving whole `(func ...)` blocks is semantics-preserving; indices are reassigned on parse.
"""
import hashlib
import random
import re
import subprocess
import sys

src, dst, mode = sys.argv[1:]
wat = subprocess.run(["wasm-tools", "print", src], check=True, capture_output=True, text=True).stdout
lines = wat.split("\n")
head = re.compile(r'^  \(func \$(?:"((?:[^"\\]|\\.)*)"|(\S+)) ')
pre, funcs, post = [], [], []
i = 0
while i < len(lines):
    m = head.match(lines[i])
    if m:
        j = i
        while lines[j] != "  )":
            j += 1
        funcs.append((m.group(1) or m.group(2), lines[i : j + 1]))
        i = j + 1
    else:
        (post if funcs else pre).append(lines[i])
        i += 1


def strip_hash(n: str) -> str:
    return re.sub(r"\[[0-9a-f]{16}\]", "", n)


def body_key(f):
    # Group by normalized body shape (opcodes only), so near-duplicate monomorphizations neighbour.
    ops = " ".join(l.strip().split(" ")[0] for l in f[1][1:])
    return (ops[:200], len(f[1]))


if mode == "name":
    funcs.sort(key=lambda f: strip_hash(f[0]))
elif mode == "size":
    funcs.sort(key=lambda f: len(f[1]))
elif mode == "body":
    funcs.sort(key=body_key)
elif mode == "shuffle":
    random.Random(0).shuffle(funcs)
out = pre + [l for f in funcs for l in f[1]] + post
subprocess.run(["wasm-tools", "parse", "-o", dst, "-"], input="\n".join(out), check=True, text=True)
