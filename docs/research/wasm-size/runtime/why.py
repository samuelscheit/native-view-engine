#!/usr/bin/env python3
"""why.py <file.wat> <regex> [depth]

Reverse "who retains this" query for a wasm-bindgen output printed by `wasm-tools print`
(symbol build, `--keep-debug`). twiggy stops at `table[0]` for every address-taken function
(vtables, fmt::Arguments fn pointers, closures), so this script adds the indirect edges itself:

  * direct:  `call $f`
  * fnptr:   `i32.const K` inside a function where K is f's table slot (fmt args, Box<dyn Fn>)
  * vtable:  `i32.const A` where A points at a rodata word range containing f's table slot
             (A <= slot_addr < A+64: an over-approximation of "this function loads f's vtable")

Edges through fnptr/vtable are over-approximate (small ints collide), so treat them as leads and
confirm in source. Prints a reverse tree up to `depth` (default 6) levels, skipping repeats.
"""
import re, sys, struct, collections

wat, pat = sys.argv[1], re.compile(sys.argv[2])
depth = int(sys.argv[3]) if len(sys.argv) > 3 else 6
text = open(wat).read()

strip = lambda n: re.sub(r'\[[0-9a-f]{16}\]', '', n)

# table slots
elem = re.search(r'\(elem \(;0;\) \(i32\.const (\d+)\) func (.*)\)\n', text)
base = int(elem.group(1))
slots = {}
for i, name in enumerate(re.findall(r'\$("(?:[^"\\]|\\.)*"|[^\s)]+)', elem.group(2))):
    slots.setdefault(name.strip('"'), []).append(base + i)

# data segments: find words equal to slot numbers
slot_of = {}
for n, ss in slots.items():
    for s in ss:
        slot_of[s] = n
data_words = collections.defaultdict(list)  # slot -> [addr]
for m in re.finditer(r'\(data \$?[^\s]* ?\(i32\.const (\d+)\) "((?:[^"\\]|\\.)*)"\)', text):
    addr = int(m.group(1))
    raw = bytearray()
    s = m.group(2)
    i = 0
    while i < len(s):
        if s[i] == '\\':
            nxt = s[i + 1]
            if nxt in 'nt"\\\'':
                raw.append({'n': 10, 't': 9, '"': 34, '\\': 92, "'": 39}[nxt]); i += 2
            elif nxt == 'u':
                j = s.index('}', i); raw += chr(int(s[i + 3:j], 16)).encode(); i = j + 1
            else:
                raw.append(int(s[i + 1:i + 3], 16)); i += 3
        else:
            raw += s[i].encode(); i += 1
    for off in range(0, len(raw) - 3, 4):
        w = struct.unpack_from('<I', raw, off)[0]
        if w in slot_of:
            data_words[w].append(addr + off)

funcs = {}
for m in re.finditer(r'^  \(func \$("(?:[^"\\]|\\.)*"|[^\s)]+) \(;\d+;\)(.*?)(?=^  \(func |^  \(table|^  \(memory|^  \(global|^  \(export|^  \(elem|^  \(data|\Z)', text, re.S | re.M):
    funcs[m.group(1).strip('"')] = m.group(2)
exports = {m.group(2).strip('"'): m.group(1) for m in re.finditer(r'\(export "([^"]+)" \(func \$("(?:[^"\\]|\\.)*"|[^\s)]+)\)\)', text)}

callers = collections.defaultdict(set)
consts = collections.defaultdict(set)  # int const -> funcs using it
for f, body in funcs.items():
    for c in re.findall(r'call \$("(?:[^"\\]|\\.)*"|[^\s)]+)', body):
        callers[c.strip('"')].add((f, 'call'))
    for k in set(re.findall(r'i32\.const (\d+)', body)):
        consts[int(k)].add(f)

sizes = {f: len(b) for f, b in funcs.items()}

def refs(f):
    out = set(callers.get(f, ()))
    for s in slots.get(f, ()):
        for g in consts.get(s, ()):
            out.add((g, 'fnptr'))
        for a in data_words.get(s, ()):
            for b in range(a - 60, a + 1, 4):
                for g in consts.get(b, ()):
                    out.add((g, f'vtable@{b}'))
    return out

seen = set()
def walk(f, d, ind):
    tag = f' [export {exports[f]}]' if f in exports else ''
    if f in seen:
        print(f'{ind}{strip(f)[:150]}{tag} (seen)'); return
    seen.add(f)
    rs = refs(f)
    print(f'{ind}{strip(f)[:150]}{tag}' + ('' if rs else '  <root>'))
    if d == 0: return
    for g, how in sorted(rs, key=lambda x: x[1])[:12]:
        print(f'{ind}  <- ({how})', end=' ')
        walk(g, d - 1, ind + '    ')

for f in [f for f in funcs if pat.search(f)][:4]:
    walk(f, depth, '')
    print()
