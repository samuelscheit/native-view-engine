#!/usr/bin/env python3
"""Classify the .rodata data segment of a wasm: printable string runs (>=6 chars) by category
(source paths, CSS-ish style strings, panic/fmt messages, other text) vs binary (vtables,
tables, pointers). Usage: rodata-classify.py file.wasm"""
import re, subprocess, sys, collections
txt = subprocess.run(["wasm-tools", "print", sys.argv[1]], capture_output=True, text=True).stdout
m = re.search(r'\(data \$\.rodata \(;0;\) \(i32\.const \d+\) "(.*)"\)\n', txt)
raw = m.group(1)
# decode wat string escapes
b = bytearray(); i = 0
while i < len(raw):
    c = raw[i]
    if c == "\\":
        n = raw[i+1]
        if n in "0123456789abcdef" and raw[i+2] in "0123456789abcdef":
            b.append(int(raw[i+1:i+3], 16)); i += 3; continue
        b.append({"n": 10, "t": 9, "\\": 92, '"': 34, "'": 39}[n]); i += 2; continue
    b += c.encode(); i += 1
data = bytes(b)
cats = collections.Counter(); examples = collections.defaultdict(list)
strs = 0
for mm in re.finditer(rb"[\x20-\x7e\n\t\xc2-\xf4\x80-\xbf]{6,}", data):
    s = mm.group().decode("utf-8", "replace"); n = len(mm.group()); strs += n
    if re.search(r"(/|\.rs\b|\.cargo|registry)", s) and ".rs" in s: k = "source paths (panic Location)"
    elif re.search(r"[a-z-]+:\s*[^;]+;", s) or re.search(r"\b(px|flex|rgba|#[0-9a-f]{6})\b", s): k = "CSS / style strings"
    elif re.search(r"^data:|base64", s): k = "data URI / base64"
    elif re.search(r"(index|overflow|panick|called|unwrap|failed|invalid|error|expected|out of|borrow|capacity|utf-8|slice|range)", s, re.I): k = "panic/error/fmt messages"
    else: k = "other text (UI copy, names, keys)"
    cats[k] += n
    if len(examples[k]) < 6: examples[k].append(s[:90])
print(f"rodata {len(data)} B; printable runs {strs} B; binary {len(data)-strs} B")
for k, v in cats.most_common():
    print(f"  {v:7d}  {k}")
    for e in examples[k]: print("            ", repr(e))
