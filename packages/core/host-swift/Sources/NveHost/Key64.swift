/// 64-bit identity key (table keys, schema hashes, interface ids; Rust `nve_hash::Key64`).
///
/// Crosses 53-bit hosts (JavaScript, Dart-web) as `(lo, hi)` `UInt32` halves.
public struct Key64: Hashable, Sendable, CustomStringConvertible {
    public let value: UInt64

    public init(_ value: UInt64) { self.value = value }

    public init(lo: UInt32, hi: UInt32) { value = UInt64(hi) << 32 | UInt64(lo) }

    /// Key of a reverse-DNS or plain name (Rust `Key64::of`).
    public static func of(_ name: String) -> Key64 { Key64(Xxh64.constId(name)) }

    public var lo: UInt32 { UInt32(truncatingIfNeeded: value) }
    public var hi: UInt32 { UInt32(truncatingIfNeeded: value >> 32) }

    /// 16 lowercase hex digits.
    public var hex: String {
        let digits = String(value, radix: 16)
        return String(repeating: "0", count: 16 - digits.count) + digits
    }

    public var description: String { "Key64(0x\(hex))" }
}
