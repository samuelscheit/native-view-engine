import XCTest
@testable import NveSwiftUI

final class NvePairClientTests: XCTestCase {
    func testWireFrameV2RoundtripAndXxh64CorruptionRejection() throws {
        let payload = Data("command-delta-swift".utf8)
        let encoded = NveWireCodec.encodeFrame(
            channel: .uiScene,
            kind: .commandDelta,
            sequence: 19,
            payload: payload
        )
        XCTAssertEqual(encoded.count, NveWireCodec.headerSize + payload.count + NveWireCodec.trailerSize)

        let decoded = try NveWireCodec.decodeFrame(encoded)
        XCTAssertEqual(decoded.channel, .uiScene)
        XCTAssertEqual(decoded.kind, .commandDelta)
        XCTAssertEqual(decoded.sequence, 19)
        XCTAssertEqual(decoded.payload, payload)

        var corrupted = encoded
        corrupted[NveWireCodec.headerSize] ^= 0x01
        XCTAssertThrowsError(try NveWireCodec.decodeFrame(corrupted)) { err in
            XCTAssertEqual(err as? NvePairProtocolError, .checksumMismatch)
        }
    }

    func testPairUriParsingWithQuicWssAndCertHash() throws {
        let parsed = try NveParsedPairUri.parse(
            "nve://pair?host=127.0.0.1&quic=8091&wss=8092&token=nve_secret&cert_hash=abcdef"
        )
        XCTAssertEqual(parsed.webSocketURL.absoluteString, "wss://127.0.0.1:8092/nve-wire")
        XCTAssertEqual(parsed.token, "nve_secret")
        XCTAssertEqual(parsed.certHashHex, "abcdef")
    }

    @MainActor
    func testClientStateMachineHandshakeCommandDeltaAndEventBatch() throws {
        let client = NvePairClient()
        XCTAssertFalse(client.state.isPaired)

        var ackPayload = Data(count: 12)
        ackPayload[0] = 0x4D // sessionId = 77
        ackPayload[4] = 2    // schema = 2
        let ackFrame = NveWireCodec.encodeFrame(
            channel: .control,
            kind: .helloAck,
            sequence: 1,
            payload: ackPayload
        )
        _ = try client.handleInboundBinary(ackFrame)
        XCTAssertEqual(client.state, .paired(sessionId: 77))

        let cmdPayload = Data([0x4E, 0x56, 0x45, 0x31, 1, 2, 3, 4])
        let deltaFrame = NveWireCodec.encodeFrame(
            channel: .uiScene,
            kind: .commandDelta,
            sequence: 5,
            payload: cmdPayload
        )
        _ = try client.handleInboundBinary(deltaFrame)
        XCTAssertEqual(client.latestFrame, cmdPayload)
        XCTAssertEqual(client.latestFrameSeq, 5)
        XCTAssertEqual(client.telemetry.framesReceived, 1)

        let evFrame = try client.sendInputEvents([
            NveEvent(surfaceId: 1, nodeId: 2, timestampUs: 100, kind: .press, payload: .point(x: 10, y: 20))
        ])
        let decodedEv = try NveWireCodec.decodeFrame(evFrame)
        XCTAssertEqual(decodedEv.channel, .inputEvents)
        XCTAssertEqual(decodedEv.kind, .eventBatch)
        XCTAssertEqual(client.telemetry.eventsSent, 1)
    }
}
