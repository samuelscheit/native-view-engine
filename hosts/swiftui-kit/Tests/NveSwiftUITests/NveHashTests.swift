import XCTest
@testable import NveSwiftUI

/// Generated `NveHash.generated.swift` must stay bit-identical to Rust `nve_hash`.
final class NveHashTests: XCTestCase {
    func testGoldenVectorsMatchRust() {
        XCTAssertTrue(NveXxh64.verifyTestVectors())
        XCTAssertEqual(NveXxh64.constId("abc"), 0x44BC_2CF5_AD77_0999)
        let key = NveKey64.of("abc")
        XCTAssertEqual(NveKey64(lo: key.lo, hi: key.hi), key)
    }

    func testWireTrailerUsesWireSeed() {
        let frame = NveWireCodec.encodeFrame(channel: .control, kind: .hello, sequence: 1, payload: Data([1, 2, 3]))
        let body = frame.prefix(frame.count - NveWireCodec.trailerSize)
        let trailer = frame.suffix(NveWireCodec.trailerSize).withUnsafeBytes { $0.loadUnaligned(as: UInt64.self).littleEndian }
        XCTAssertEqual(trailer, NveXxh64.hash(Data(body), seed: NveWireCodec.xxh64Seed))
    }
}
