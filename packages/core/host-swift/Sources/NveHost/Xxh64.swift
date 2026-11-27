/// Canonical XXH64, bit-identical to the Rust `nve-hash` crate.
public enum Xxh64 {
    public static let prime1: UInt64 = 0x9E37_79B1_85EB_CA87
    public static let prime2: UInt64 = 0xC2B2_AE3D_27D4_EB4F
    public static let prime3: UInt64 = 0x1656_67B1_9E37_79F9
    public static let prime4: UInt64 = 0x85EB_CA77_C2B2_AE63
    public static let prime5: UInt64 = 0x27D4_EB2F_1656_67C5

    /// One-shot digest of `bytes` with `seed`.
    public static func hash<Bytes: Sequence>(_ bytes: Bytes, seed: UInt64 = 0) -> UInt64 where Bytes.Element == UInt8 {
        var h = Xxh64Hasher(seed: seed)
        h.update(bytes)
        return h.digest()
    }

    /// Identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`).
    public static func constId(_ name: String) -> UInt64 { hash(name.utf8) }
}

/// Streaming XXH64; digests equal `Xxh64.hash` for any chunking. Allocation-free value type.
public struct Xxh64Hasher: Sendable {
    private let seed: UInt64
    private var totalLen: UInt64 = 0
    private var v1, v2, v3, v4: UInt64
    /// 32-byte stripe buffer as four lanes (inline storage, no heap allocation).
    private var buf: (UInt64, UInt64, UInt64, UInt64) = (0, 0, 0, 0)
    private var bufLen = 0

    public init(seed: UInt64 = 0) {
        self.seed = seed
        v1 = seed &+ Xxh64.prime1 &+ Xxh64.prime2
        v2 = seed &+ Xxh64.prime2
        v3 = seed
        v4 = seed &- Xxh64.prime1
    }

    public mutating func update<Bytes: Sequence>(_ bytes: Bytes) where Bytes.Element == UInt8 {
        let handled: Void? = bytes.withContiguousStorageIfAvailable { update(UnsafeRawBufferPointer($0)) }
        if handled == nil {
            for b in bytes { update(byte: b) }
        }
    }

    public mutating func update(_ bytes: UnsafeRawBufferPointer) {
        let len = bytes.count
        totalLen &+= UInt64(len)
        var i = 0
        while bufLen > 0 && bufLen < 32 && i < len {
            push(bytes[i])
            i += 1
        }
        if bufLen == 32 {
            flushBuffer()
        }
        while bufLen == 0 && len - i >= 32 {  // `i + 32` could overflow near Int.max (arm64_32)
            stripe(bytes, i)
            i += 32
        }
        while i < len {
            push(bytes[i])
            i += 1
        }
    }

    /// Feeds `v` as 4 little-endian bytes.
    public mutating func update(u32 v: UInt32) { withUnsafeBytes(of: v.littleEndian) { update($0) } }

    /// Feeds `v` as 8 little-endian bytes.
    public mutating func update(u64 v: UInt64) { withUnsafeBytes(of: v.littleEndian) { update($0) } }

    /// Digest of everything fed so far (the hasher stays usable).
    public func digest() -> UInt64 {
        var h: UInt64
        if totalLen >= 32 {
            h = rotl(v1, 1) &+ rotl(v2, 7) &+ rotl(v3, 12) &+ rotl(v4, 18)
            h = mergeRound(mergeRound(mergeRound(mergeRound(h, v1), v2), v3), v4)
        } else {
            h = seed &+ Xxh64.prime5
        }
        h &+= totalLen
        let count = bufLen
        return withUnsafeBytes(of: buf) { p -> UInt64 in
            var i = 0
            while i + 8 <= count {
                h ^= round(0, p.loadUnaligned(fromByteOffset: i, as: UInt64.self).littleEndian)
                h = rotl(h, 27) &* Xxh64.prime1 &+ Xxh64.prime4
                i += 8
            }
            if i + 4 <= count {
                h ^= UInt64(p.loadUnaligned(fromByteOffset: i, as: UInt32.self).littleEndian) &* Xxh64.prime1
                h = rotl(h, 23) &* Xxh64.prime2 &+ Xxh64.prime3
                i += 4
            }
            while i < count {
                h ^= UInt64(p[i]) &* Xxh64.prime5
                h = rotl(h, 11) &* Xxh64.prime1
                i += 1
            }
            h ^= h >> 33
            h = h &* Xxh64.prime2
            h ^= h >> 29
            h = h &* Xxh64.prime3
            return h ^ (h >> 32)
        }
    }

    private mutating func update(byte: UInt8) {
        totalLen &+= 1
        push(byte)
        if bufLen == 32 { flushBuffer() }
    }

    private mutating func push(_ byte: UInt8) {
        let at = bufLen
        withUnsafeMutableBytes(of: &buf) { $0[at] = byte }
        bufLen += 1
    }

    private mutating func flushBuffer() {
        let lanes = buf
        withUnsafeBytes(of: lanes) { stripe($0, 0) }
        bufLen = 0
    }

    private mutating func stripe(_ p: UnsafeRawBufferPointer, _ i: Int) {
        v1 = round(v1, p.loadUnaligned(fromByteOffset: i, as: UInt64.self).littleEndian)
        v2 = round(v2, p.loadUnaligned(fromByteOffset: i + 8, as: UInt64.self).littleEndian)
        v3 = round(v3, p.loadUnaligned(fromByteOffset: i + 16, as: UInt64.self).littleEndian)
        v4 = round(v4, p.loadUnaligned(fromByteOffset: i + 24, as: UInt64.self).littleEndian)
    }

    @inline(__always) private func rotl(_ x: UInt64, _ r: UInt64) -> UInt64 { (x << r) | (x >> (64 - r)) }

    @inline(__always) private func mergeRound(_ acc: UInt64, _ lane: UInt64) -> UInt64 {
        (acc ^ round(0, lane)) &* Xxh64.prime1 &+ Xxh64.prime4
    }

    @inline(__always) private func round(_ acc: UInt64, _ lane: UInt64) -> UInt64 {
        rotl(acc &+ lane &* Xxh64.prime2, 31) &* Xxh64.prime1
    }
}
