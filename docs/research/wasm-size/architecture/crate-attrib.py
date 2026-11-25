#!/usr/bin/env python3
"""Group `twiggy top -n 100000 -f csv` rows by crate (first `crate[hash]::` or `<crate[hash]::`
occurrence; generic instantiations land in the crate that defines the fn). Skips custom sections."""
import csv, re, sys, collections
rows = csv.DictReader(open(sys.argv[1]))
agg = collections.Counter()
for r in rows:
    name, size = r["Name"], int(r["ShallowSize"])
    if name.startswith("custom section") or "subsection" in name:
        continue
    m = re.search(r"([a-z_][a-z0-9_]*)\[[0-9a-f]+\]::", name)
    if m: key = m.group(1)
    elif name.startswith("data segment"): key = name
    elif name.startswith("import "): key = "imports"
    elif name.startswith("export "): key = "exports"
    else: key = "other:" + name[:40]
    agg[key] += size
total = sum(agg.values())
for k, v in agg.most_common(int(sys.argv[2]) if len(sys.argv) > 2 else 30):
    print(f"{v:8d} {100*v/total:5.1f}%  {k}")
print(f"{total:8d} total (code+data+imports/exports, excluding custom sections)")
