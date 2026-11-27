import NveHost
import XCTest
@testable import NveSwiftUI

/// The pair-client wire trailer is the `NveHost` XXH64 of the body with the wire seed (the digest
/// itself is covered against generated golden vectors by `packages/core/host-swift`).
final class NveHashTests: XCTestCase {
    func testWireTrailerUsesWireSeed() {
        let frame = NveWireCodec.encodeFrame(channel: .control, kind: .hello, sequence: 1, payload: Data([1, 2, 3]))
        let body = frame.prefix(frame.count - NveWireCodec.trailerSize)
        let trailer = frame.suffix(NveWireCodec.trailerSize).withUnsafeBytes { $0.loadUnaligned(as: UInt64.self).littleEndian }
        XCTAssertEqual(trailer, Xxh64.hash(Data(body), seed: NveWireCodec.xxh64Seed))
    }
}
