import NveHost

/// Golden XXH64 vectors (generated from the canonical Rust `nve_hash::xxh64` into
/// `HashVectors.generated.swift`) plus the shared `NveBinValue` fixture.
enum Golden {
    static let wireSeed: UInt64 = 0x4E56_4557_4952_4532

    static func seq(_ n: Int) -> [UInt8] { (0..<n).map { UInt8(truncatingIfNeeded: $0 * 31 + 7) } }

    static let vectors: [(label: String, bytes: [UInt8], seed: UInt64, expected: UInt64)] =
        hashVectors.map { ($0.label, fromHex($0.hex), $0.seed, $0.expected) }

    /// Validated by Rust `BinValueView::parse`: {"k": [1, "é", null, true, 1.5, b"\xFF"]}.
    static let binValueHex =
        "07 34000000 01000000 01000000 6b 06 26000000 06000000 02 0100000000000000 04 02000000 c3a9 00 0101 03 "
        + "000000000000f83f 05 01000000 ff"

    static func fromHex(_ hex: String) -> [UInt8] {
        let digits = hex.filter { $0 != " " }.compactMap { $0.hexDigitValue }
        return stride(from: 0, to: digits.count, by: 2).map { UInt8(digits[$0] * 16 + digits[$0 + 1]) }
    }
}
