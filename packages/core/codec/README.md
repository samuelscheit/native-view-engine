# `nve-codec`

Zero-unsafe (`#![forbid(unsafe_code)]`), zero-runtime-dependency Tier-2 little-endian binary framing primitives (`ByteReader`, `ByteWriter`, `CodecError`, `NveEncode`, `NveDecode`) and `#[derive(NveCodec)]` (`packages/core/codec`, **Core Foundation layer**, strictly below `nve-abi`).

## Key Public API

- `ByteReader<'a>` — strictly bounds-checked LE reader (strict `0|1` bools, canonical LE floats, bounded `read_bytes`/`read_str`/`read_vec`, `expect_magic`, `expect_zero`, `finish()`).
- `ByteWriter` — reusable LE writer (`clear()`, `with_buf()`, `patch_u32`, length-prefixed slices/strings/sequences).
- `CodecError` — typed, allocation-free binary framing error (`Truncated`, `BadMagic`, `BadVersion`, `BadEnum`, `BadBool`, `NonZeroReserved`, `LenOverflow`, `TrailingBytes`, `InvalidUtf8`).
- `NveEncode` / `NveDecode` and `#[derive(NveCodec)]`.

```rust
use nve_codec::{ByteReader, ByteWriter, NveCodec, NveDecode, NveEncode};

#[derive(Debug, PartialEq, NveCodec)]
struct Ping {
    seq: u32,
    ok: bool,
    label: String,
}

let msg = Ping { seq: 7, ok: true, label: "nve".into() };
let bytes = msg.encode_to_vec().unwrap();
assert_eq!(Ping::decode_exact(&bytes).unwrap(), msg);

let mut w = ByteWriter::with_capacity(16);
w.write_u32(42);
let mut r = ByteReader::new(w.as_slice());
assert_eq!(r.read_u32().unwrap(), 42);
r.finish().unwrap();
```

## Dependency Invariants

- **Depends ONLY on**: `nve-abi-macros` (zero-dependency proc-macro crate), keeping the core DAG strictly acyclic below `nve-abi`.
- **Must NOT depend on**: `nve-abi`, `dioxus*`, `serde*`, platform crates, or `packages/modules/*`.

## Verification

```bash
cargo test -p nve-codec
cargo bench -p nve-codec --no-run
```
