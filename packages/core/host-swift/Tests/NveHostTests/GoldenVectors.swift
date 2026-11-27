import NveHost

/// Golden XXH64 vectors computed by the canonical Rust `nve-hash` crate.
enum Golden {
    static let prime32: UInt64 = 0x9E37_79B1
    static let wireSeed: UInt64 = 0x4E56_4557_4952_4532

    static func seq(_ n: Int) -> [UInt8] { (0..<n).map { UInt8(truncatingIfNeeded: $0 * 31 + 7) } }

    /// Official Cyan4973 xxHash sanity buffer.
    static func cyan(_ n: Int) -> [UInt8] {
        var gen = prime32
        return (0..<n).map { _ in
            defer { gen = gen &* Xxh64.prime1 }
            return UInt8(truncatingIfNeeded: gen >> 56)
        }
    }

    static let vectors: [(label: String, bytes: [UInt8], seed: UInt64, expected: UInt64)] = [
        ("empty", [], 0, 0xEF46_DB37_51D8_E999),
        ("a", Array("a".utf8), 0, 0xD24E_C4F1_A98C_6E5B),
        ("abc", Array("abc".utf8), 0, 0x44BC_2CF5_AD77_0999),
        ("nve.visual", Array("nve.visual".utf8), 0, 0x33E7_3115_985A_4FD6),
        ("high_bits", [0x00, 0x01, 0x7F, 0x80, 0xFF], 0, 0x8BAB_CAB4_99A1_41EA),
        ("utf8", Array("héllo wörld — 日本語 🚀".utf8), 0, 0x72AC_442F_F213_F592),
        ("seq101", seq(101), 0, 0xFBFD_F3FA_1A53_BC7F),
        ("seq101_wire", seq(101), wireSeed, 0x277E_092A_8C3D_B1BC),
        ("seq37_prime1", seq(37), Xxh64.prime1, 0x33DD_B9A8_85FC_58A4),
        ("seq1024", seq(1024), 0, 0x149A_A449_72CD_AE00),
        ("cyan1", cyan(1), 0, 0xE934_A84A_DB05_2768),
        ("cyan4", cyan(4), 0, 0xE244_DDBC_F81E_29E4),
        ("cyan14", cyan(14), 0, 0xB89B_3598_E0BD_0A0A),
        ("cyan222", cyan(222), 0, 0x06CC_5BD9_30BC_EC3A),
        ("cyan1024", cyan(1024), 0, 0x2E30_7F2F_E063_991A),
        ("cyan1_p32", cyan(1), prime32, 0x5014_6076_43A9_B4C3),
        ("cyan222_p32", cyan(222), prime32, 0x0B10_5469_DF89_AF66),
        ("cyan32_p1", cyan(32), Xxh64.prime1, 0x6D02_42B0_B7E2_78ED),
        ("cyan1024_p1", cyan(1024), Xxh64.prime1, 0xF464_C3F6_9A39_421C),
    ]

    /// Validated by Rust `BinValueView::parse`: {"k": [1, "é", null, true, 1.5, b"\xFF"]}.
    static let binValueHex =
        "07 34000000 01000000 01000000 6b 06 26000000 06000000 02 0100000000000000 04 02000000 c3a9 00 0101 03 "
        + "000000000000f83f 05 01000000 ff"

    static func fromHex(_ hex: String) -> [UInt8] {
        let digits = hex.filter { $0 != " " }.compactMap { $0.hexDigitValue }
        return stride(from: 0, to: digits.count, by: 2).map { UInt8(digits[$0] * 16 + digits[$0 + 1]) }
    }
}
