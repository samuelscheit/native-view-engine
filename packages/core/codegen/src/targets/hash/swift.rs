//! Swift (`NveHash.generated.swift`) XXH64 mirror.

use std::fmt::Write;

use super::{banner, byte_list, HASH_VECTORS, STREAM_SPLITS};

const BODY: &str = r#"import Foundation

/// 64-bit identity key crossing 53-bit hosts as `(lo, hi)` `UInt32` halves (Rust `nve_hash::Key64`).
public struct NveKey64: Hashable, Sendable {
    public let value: UInt64

    public init(_ value: UInt64) { self.value = value }
    public init(lo: UInt32, hi: UInt32) { value = UInt64(hi) << 32 | UInt64(lo) }

    public var lo: UInt32 { UInt32(truncatingIfNeeded: value) }
    public var hi: UInt32 { UInt32(truncatingIfNeeded: value >> 32) }

    /// Key of a reverse-DNS or plain name (Rust `Key64::of`).
    public static func of(_ name: String) -> NveKey64 { NveKey64(NveXxh64.constId(name)) }
}

/// Streaming canonical XXH64; digests equal `NveXxh64.hash` for any chunking.
public struct NveXxh64Hasher: Sendable {
    private var totalLen: UInt64 = 0
    private let seed: UInt64
    private var v1, v2, v3, v4: UInt64
    private var buf = [UInt8](repeating: 0, count: 32)
    private var bufLen = 0

    public init(seed: UInt64 = 0) {
        self.seed = seed
        v1 = seed &+ NveXxh64.prime1 &+ NveXxh64.prime2
        v2 = seed &+ NveXxh64.prime2
        v3 = seed
        v4 = seed &- NveXxh64.prime1
    }

    public mutating func update(_ bytes: UnsafeRawBufferPointer) {
        let len = bytes.count
        totalLen &+= UInt64(len)
        var i = 0
        if bufLen > 0 {
            while bufLen < 32 && i < len {
                buf[bufLen] = bytes[i]
                bufLen += 1
                i += 1
            }
            if bufLen < 32 { return }
            buf.withUnsafeBytes { stripe($0, 0) }
            bufLen = 0
        }
        while i + 32 <= len {
            stripe(bytes, i)
            i += 32
        }
        while i < len {
            buf[bufLen] = bytes[i]
            bufLen += 1
            i += 1
        }
    }

    public mutating func update(_ bytes: [UInt8]) { bytes.withUnsafeBytes { update($0) } }
    public mutating func update(_ data: Data) { data.withUnsafeBytes { update($0) } }

    public func digest() -> UInt64 {
        var h: UInt64
        if totalLen >= 32 {
            h = rotl(v1, 1) &+ rotl(v2, 7) &+ rotl(v3, 12) &+ rotl(v4, 18)
            for lane in [v1, v2, v3, v4] {
                h = (h ^ round(0, lane)) &* NveXxh64.prime1 &+ NveXxh64.prime4
            }
        } else {
            h = seed &+ NveXxh64.prime5
        }
        h &+= totalLen
        return buf.withUnsafeBytes { p -> UInt64 in
            var i = 0
            while i + 8 <= bufLen {
                h ^= round(0, p.loadUnaligned(fromByteOffset: i, as: UInt64.self).littleEndian)
                h = rotl(h, 27) &* NveXxh64.prime1 &+ NveXxh64.prime4
                i += 8
            }
            if i + 4 <= bufLen {
                h ^= UInt64(p.loadUnaligned(fromByteOffset: i, as: UInt32.self).littleEndian) &* NveXxh64.prime1
                h = rotl(h, 23) &* NveXxh64.prime2 &+ NveXxh64.prime3
                i += 4
            }
            while i < bufLen {
                h ^= UInt64(p[i]) &* NveXxh64.prime5
                h = rotl(h, 11) &* NveXxh64.prime1
                i += 1
            }
            h ^= h >> 33
            h = h &* NveXxh64.prime2
            h ^= h >> 29
            h = h &* NveXxh64.prime3
            return h ^ (h >> 32)
        }
    }

    private mutating func stripe(_ p: UnsafeRawBufferPointer, _ i: Int) {
        v1 = round(v1, p.loadUnaligned(fromByteOffset: i, as: UInt64.self).littleEndian)
        v2 = round(v2, p.loadUnaligned(fromByteOffset: i + 8, as: UInt64.self).littleEndian)
        v3 = round(v3, p.loadUnaligned(fromByteOffset: i + 16, as: UInt64.self).littleEndian)
        v4 = round(v4, p.loadUnaligned(fromByteOffset: i + 24, as: UInt64.self).littleEndian)
    }

    @inline(__always) private func rotl(_ x: UInt64, _ r: UInt64) -> UInt64 { (x << r) | (x >> (64 - r)) }

    @inline(__always) private func round(_ acc: UInt64, _ lane: UInt64) -> UInt64 {
        rotl(acc &+ lane &* NveXxh64.prime2, 31) &* NveXxh64.prime1
    }
}

/// Canonical XXH64 (bit-identical to Rust `nve_hash::xxh64`).
public enum NveXxh64 {
    public static let prime1: UInt64 = 0x9E37_79B1_85EB_CA87
    public static let prime2: UInt64 = 0xC2B2_AE3D_27D4_EB4F
    public static let prime3: UInt64 = 0x1656_67B1_9E37_79F9
    public static let prime4: UInt64 = 0x85EB_CA77_C2B2_AE63
    public static let prime5: UInt64 = 0x27D4_EB2F_1656_67C5

    public static func hash(_ bytes: UnsafeRawBufferPointer, seed: UInt64 = 0) -> UInt64 {
        var h = NveXxh64Hasher(seed: seed)
        h.update(bytes)
        return h.digest()
    }

    public static func hash(_ bytes: [UInt8], seed: UInt64 = 0) -> UInt64 {
        bytes.withUnsafeBytes { hash($0, seed: seed) }
    }

    public static func hash(_ data: Data, seed: UInt64 = 0) -> UInt64 {
        data.withUnsafeBytes { hash($0, seed: seed) }
    }

    /// Identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`).
    public static func constId(_ name: String) -> UInt64 { hash(Array(name.utf8)) }
"#;

/// Generates `hosts/swiftui-kit/Sources/NveSwiftUI/NveHash.generated.swift`.
pub(super) fn generate_swift_hash() -> String {
    let mut s = banner();
    s.push_str(BODY);
    s.push_str(
        "\n    /// Golden vectors `(bytes, seed, expected)` shared with every NVE host.\n    \
         public static let testVectors: [(bytes: [UInt8], seed: UInt64, expected: UInt64)] = [\n",
    );
    for v in HASH_VECTORS {
        let bytes = byte_list(v.bytes, |b| format!("0x{b:02X}"));
        writeln!(
            s,
            "        ([{bytes}], 0x{:016X}, 0x{:016X}), // {}",
            v.seed,
            v.expected(),
            v.label
        )
        .unwrap();
    }
    let [a, b, c] = STREAM_SPLITS;
    write!(
        s,
        r#"    ]

    /// `true` when one-shot and chunked digests of every golden vector match Rust.
    public static func verifyTestVectors() -> Bool {{
        testVectors.allSatisfy {{ v in
            var h = NveXxh64Hasher(seed: v.seed)
            var at = 0
            for split in [{a}, {b}, {c}] {{
                let end = min(split, v.bytes.count)
                if end > at {{
                    h.update(Array(v.bytes[at..<end]))
                    at = end
                }}
            }}
            h.update(Array(v.bytes[at...]))
            return hash(v.bytes, seed: v.seed) == v.expected && h.digest() == v.expected
        }}
    }}
}}
"#
    )
    .unwrap();
    s
}
