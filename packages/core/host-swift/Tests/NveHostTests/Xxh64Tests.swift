import NveHost
import XCTest

final class Xxh64Tests: XCTestCase {
    func testGoldenVectorsOneShot() {
        for v in Golden.vectors {
            XCTAssertEqual(Xxh64.hash(v.bytes, seed: v.seed), v.expected, v.label)
        }
    }

    func testStreamingMatchesOneShotForEveryChunking() {
        for v in Golden.vectors {
            for step in [1, 3, 7, 13, 24, 31, 32, 33, 64, 100] {
                var h = Xxh64Hasher(seed: v.seed)
                var at = 0
                while at < v.bytes.count {
                    let end = min(at + step, v.bytes.count)
                    h.update(v.bytes[at..<end])
                    at = end
                }
                XCTAssertEqual(h.digest(), v.expected, "\(v.label) step \(step)")
            }
        }
    }

    func testNonContiguousSequencesAndRawBuffers() {
        let bytes = Golden.seq(101)
        XCTAssertEqual(Xxh64.hash(bytes.lazy.map { $0 }, seed: Golden.wireSeed), 0x277E_092A_8C3D_B1BC)
        XCTAssertEqual(bytes.withUnsafeBytes { (raw) -> UInt64 in
            var h = Xxh64Hasher(seed: Golden.wireSeed)
            h.update(raw)
            return h.digest()
        }, 0x277E_092A_8C3D_B1BC)
    }

    func testDigestDoesNotConsumeAndTypedFeeds() {
        var h = Xxh64Hasher(seed: 7)
        h.update(u32: 0x0403_0201)
        XCTAssertEqual(h.digest(), h.digest())
        h.update(u64: 0x0C0B_0A09_0807_0605)
        XCTAssertEqual(h.digest(), Xxh64.hash([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], seed: 7))
    }

    func testKey64() {
        let k = Key64.of("nve.visual")
        XCTAssertEqual(k.value, 0x33E7_3115_985A_4FD6)
        XCTAssertEqual(k.lo, 0x985A_4FD6)
        XCTAssertEqual(k.hi, 0x33E7_3115)
        XCTAssertEqual(Key64(lo: k.lo, hi: k.hi), k)
        XCTAssertEqual(Key64(lo: .max, hi: .max).value, .max)
        XCTAssertEqual(Key64(1).hex, "0000000000000001")
        XCTAssertEqual(Xxh64.constId("abc"), 0x44BC_2CF5_AD77_0999)
    }
}
