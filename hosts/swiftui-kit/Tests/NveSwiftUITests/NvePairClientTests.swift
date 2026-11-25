import Foundation
import NveSwiftUI
import XCTest

private final class MockPairTransport: NvePairTransport {
    var connectedUrl: URL?
    var sentFrames: [Data] = []
    var closedCount = 0
    var onOpen: (@Sendable () -> Void)?
    var onBinary: (@Sendable (Data) -> Void)?
    var onClose: (@Sendable (Error?) -> Void)?

    func connect(
        url: URL,
        onOpen: @escaping @Sendable () -> Void,
        onBinary: @escaping @Sendable (Data) -> Void,
        onClose: @escaping @Sendable (Error?) -> Void
    ) {
        connectedUrl = url
        self.onOpen = onOpen
        self.onBinary = onBinary
        self.onClose = onClose
    }

    func send(_ data: Data) {
        sentFrames.append(data)
    }

    func close() {
        closedCount += 1
    }
}

final class NvePairClientTests: XCTestCase {
    // MARK: - Helpers

    private func appendLE<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
    }

    private func writeVarint(_ value: UInt64, to data: inout Data) {
        var v = value
        while true {
            var b = UInt8(v & 0x7F)
            v >>= 7
            if v != 0 {
                b |= 0x80
                data.append(b)
            } else {
                data.append(b)
                break
            }
        }
    }

    private func encodeXorRleDelta(base: Data, next: Data) -> Data {
        var out = Data(capacity: 64)
        writeVarint(UInt64(next.count), to: &out)
        let b = Array(base), n = Array(next); let len = n.count; var i = 0
        while i < len {
            let z0 = i
            while i < len && ((i < b.count ? b[i] : 0) ^ n[i]) == 0 { i += 1 }
            let zr = i - z0
            if zr > 0 { if zr <= 127 { out.append(UInt8(zr)) } else { out.append(0); writeVarint(UInt64(zr), to: &out) } }
            if i >= len { break }
            let l0 = i
            while i < len {
                if ((i < b.count ? b[i] : 0) ^ n[i]) == 0 {
                    let nz = (i + 1 < len) && (((i + 1 < b.count ? b[i + 1] : 0) ^ n[i + 1]) == 0)
                    if nz || i + 1 == len { break }
                }
                i += 1
            }
            let ll = i - l0
            if ll <= 127 { out.append(0x80 | UInt8(ll)) } else { out.append(0x80); writeVarint(UInt64(ll), to: &out) }
            for k in l0 ..< i { out.append((k < b.count ? b[k] : 0) ^ n[k]) }
        }
        return out
    }

    /// Builds a valid `NVE1` `CommandBuffer` with `CreateNode`, `SetText`, `SetChildren`, `Commit`.
    private func makeNveCommandBuffer(surfaceId: UInt32 = 1, frameId: UInt64, text: String) -> Data {
        var ops: [Data] = []

        // CreateNode(id=2, kind=2 (Text), name="")
        var op1 = Data([1])
        appendLE(UInt32(2), to: &op1)
        appendLE(UInt16(2), to: &op1)
        appendLE(UInt16(0), to: &op1)
        ops.append(op1)

        // SetText(id=2, text)
        let textBytes = Data(text.utf8)
        var op2 = Data([4])
        appendLE(UInt32(2), to: &op2)
        appendLE(UInt32(textBytes.count), to: &op2)
        op2.append(textBytes)
        ops.append(op2)

        // SetChildren(parent=1, [2])
        var op3 = Data([3])
        appendLE(UInt32(1), to: &op3)
        appendLE(UInt32(1), to: &op3)
        appendLE(UInt32(2), to: &op3)
        ops.append(op3)

        // Commit(sync=1)
        ops.append(Data([10, 1]))

        var out = Data()
        appendLE(NvePairProtocol.nveAbiMagic, to: &out)
        appendLE(NvePairProtocol.nveAbiVersion, to: &out)
        appendLE(UInt16(0), to: &out)
        appendLE(surfaceId, to: &out)
        appendLE(frameId, to: &out)
        appendLE(UInt32(ops.count), to: &out)
        for op in ops { out.append(op) }
        return out
    }

    /// Builds a valid `CommandOpDelta` payload replacing op index 1 (`SetText`) via `XorOp` or `LiteralOp`.
    private func makeCommandOpDeltaPayload(
        base: Data,
        next: Data,
        useXorForChangedOp: Bool
    ) throws -> Data {
        let (_, baseRanges) = try XCTUnwrap(NveDeltaCodec.scanNveCommandSlices(base))
        let (nextSummary, nextRanges) = try XCTUnwrap(NveDeltaCodec.scanNveCommandSlices(next))
        var out = Data()
        appendLE(nextSummary.flags, to: &out)
        writeVarint(UInt64(nextSummary.surfaceId), to: &out)
        writeVarint(nextSummary.frameId, to: &out)
        writeVarint(UInt64(nextSummary.opCount), to: &out)

        // CopyRun(start=0, count=1) for op 0
        out.append(1)
        writeVarint(0, to: &out)
        writeVarint(1, to: &out)

        // Op 1 changed: either XorOp(3) or LiteralOp(2)
        let baseOp1 = base[baseRanges[1]]
        let nextOp1 = next[nextRanges[1]]
        if useXorForChangedOp {
            let xorBytes = encodeXorRleDelta(base: baseOp1, next: nextOp1)
            out.append(3)
            writeVarint(1, to: &out)
            writeVarint(UInt64(xorBytes.count), to: &out)
            out.append(xorBytes)
        } else {
            out.append(2)
            writeVarint(UInt64(nextOp1.count), to: &out)
            out.append(nextOp1)
        }

        // CopyRun(start=2, count=2) for ops 2 & 3
        out.append(1)
        writeVarint(2, to: &out)
        writeVarint(2, to: &out)
        return out
    }

    // MARK: - URI Parsing Tests

    func testPairUriParsingVariants() throws {
        let full = try NvePairUri.parse("nve://pair?host=127.0.0.1&udp=8091&ws=8092&token=tok123&w=393&h=852")
        XCTAssertEqual(full.wsUrl.absoluteString, "ws://127.0.0.1:8092/ws/nvd")
        XCTAssertEqual(full.token, "tok123")
        XCTAssertEqual(full.wsPort, 8092)
        XCTAssertEqual(full.udpPort, 8091)
        XCTAssertEqual(full.width, 393)
        XCTAssertEqual(full.height, 852)
        XCTAssertEqual(full.schemaVersion, 1)

        let embeddedWs = try NvePairUri.parse("nve://pair?ws=wss://studio.local:9443/ws/nvd&token=abc&schema=2")
        XCTAssertEqual(embeddedWs.wsUrl.absoluteString, "wss://studio.local:9443/ws/nvd")
        XCTAssertEqual(embeddedWs.token, "abc")
        XCTAssertEqual(embeddedWs.schemaVersion, 2)

        let ipv6 = try NvePairUri.parse("nve://pair?host=::1&ws=8092&token=v6tok")
        XCTAssertEqual(ipv6.wsUrl.absoluteString, "ws://[::1]:8092/ws/nvd")

        let directWs = try NvePairUri.parse("ws://10.0.0.2:8092/ws/nvd", fallbackToken: "override")
        XCTAssertEqual(directWs.wsUrl.absoluteString, "ws://10.0.0.2:8092/ws/nvd")
        XCTAssertEqual(directWs.token, "override")
    }

    func testPairUriRejectsMalformedInputs() {
        XCTAssertThrowsError(try NvePairUri.parse(""))
        XCTAssertThrowsError(try NvePairUri.parse("nve://wrong?ws=8092&token=t"))
        XCTAssertThrowsError(try NvePairUri.parse("nve://pair?host=127.0.0.1&token=t"))
        XCTAssertThrowsError(try NvePairUri.parse("nve://pair?host=127.0.0.1&ws=8092"))
        XCTAssertThrowsError(try NvePairUri.parse("nve://pair?host=127.0.0.1&ws=99999&token=t"))
        XCTAssertThrowsError(try NvePairUri.parse("http://127.0.0.1:8092"))
    }

    // MARK: - NVT1 Framing & Crypto Round-Trip

    func testNvt1HeaderAndPayloadsRoundTrip() throws {
        // Known FNV-1a 64-bit standard test vector for "foobar": 0x85944171f73967e8
        XCTAssertEqual(NvePairCrypto.fnv1a64(Data("foobar".utf8)), 0x8594_4171_F739_67E8)
        XCTAssertEqual(NvePairCrypto.fnv1a64(Data()), 0xCBF2_9CE4_8422_2325)

        let proof = NvePairCrypto.deriveTokenProof(pairingToken: "secret-pair", schemaVersion: 1)
        XCTAssertEqual(proof.count, 16)
        let hello = NveHelloPayload(
            schemaVersion: 1,
            maxWindow: 64,
            clientLastSeq: 7,
            clientNonce: 0x0102_0304_0506_0708,
            tokenProof: proof
        )
        let envBytes = NveTransportEnvelope.encode(kind: .hello, sessionId: 0, payload: hello.encode())
        XCTAssertEqual(envBytes.count, 8 + 32)
        XCTAssertEqual(Array(envBytes.prefix(4)), [0x4E, 0x54, 1, 1])

        let parsed = try NveTransportEnvelope.parse(envBytes)
        XCTAssertEqual(parsed.kind, .hello)
        XCTAssertEqual(parsed.sessionId, 0)
        XCTAssertEqual(try NveHelloPayload.decode(parsed.payload), hello)

        let sid = NvePairCrypto.deriveSessionId(
            pairingToken: "secret-pair",
            schemaVersion: 1,
            clientNonce: 0x1111,
            serverNonce: 0x2222
        )
        XCTAssertNotEqual(sid, 0)

        let ack = NveHelloAckPayload(schemaVersion: 1, serverMaxWindow: 64, serverSeq: 12, serverNonce: 0x2222, resyncRequired: true)
        let ackEnv = try NveTransportEnvelope.parse(NveTransportEnvelope.encode(kind: .helloAck, sessionId: sid, payload: ack.encode()))
        XCTAssertEqual(try NveHelloAckPayload.decode(ackEnv.payload), ack)

        let kfReq = NvePairCodec.encodeKeyframeRequest(sessionId: sid, lastSeq: 42, reason: 3)
        let kfEnv = try NveTransportEnvelope.parse(kfReq)
        XCTAssertEqual(kfEnv.kind, .keyframeRequest)
        let decodedReq = try NvePairCodec.decodeKeyframeRequest(kfEnv.payload)
        XCTAssertEqual(decodedReq.lastSeq, 42)
        XCTAssertEqual(decodedReq.reason, 3)

        let pingEnv = try NveTransportEnvelope.parse(NvePairCodec.encodePing(sessionId: sid, timestampUs: 987_654))
        XCTAssertEqual(pingEnv.kind, .ping)
        XCTAssertEqual(try NvePairCodec.decodeTimestampUs(pingEnv.payload), 987_654)
    }

    // MARK: - NVD1 Keyframe + XOR-RLE + CommandOpDelta Reconstruction

    func testNvd1KeyframeAndDeltaReconstruction() throws {
        var receiver = NveDeltaReceiver()
        let frame1 = makeNveCommandBuffer(frameId: 1, text: "Count: 0")
        let frame2 = makeNveCommandBuffer(frameId: 2, text: "Count: 12345")
        let frame3 = makeNveCommandBuffer(frameId: 3, text: "Count: 12346")

        // 1. Keyframe
        let kfPkt = NveDeltaPacket.encode(kind: .keyframe, seq: 1, baseSeq: 0, payload: frame1)
        let out1 = try receiver.receivePacket(kfPkt)
        XCTAssertEqual(out1.appliedFrames.count, 1)
        XCTAssertEqual(out1.appliedFrames[0].frame, frame1)
        XCTAssertFalse(out1.gapDetected)
        XCTAssertNotNil(out1.ackPacket)

        // 2. CommandOpDelta (with variable-length string shift "Count: 0" -> "Count: 12345")
        let cmdDeltaPayload = try makeCommandOpDeltaPayload(base: frame1, next: frame2, useXorForChangedOp: false)
        let p2 = NveDeltaPacket.encode(kind: .commandOpDelta, seq: 2, baseSeq: 1, payload: cmdDeltaPayload)
        let out2 = try receiver.receivePacket(p2)
        XCTAssertEqual(out2.appliedFrames.count, 1)
        XCTAssertEqual(out2.appliedFrames[0].frame, frame2)

        // 3. CommandOpDelta with per-op XorOp (tag 3)
        let xorOpDeltaPayload = try makeCommandOpDeltaPayload(base: frame2, next: frame3, useXorForChangedOp: true)
        let p3 = NveDeltaPacket.encode(kind: .commandOpDelta, seq: 3, baseSeq: 2, payload: xorOpDeltaPayload)
        let out3 = try receiver.receivePacket(p3)
        XCTAssertEqual(out3.appliedFrames.count, 1)
        XCTAssertEqual(out3.appliedFrames[0].frame, frame3)

        // 4. Byte-level XorRleDelta with extended (>127 byte) zero run and size change
        let largeBase = Data(repeating: 0xAA, count: 500)
        var largeNext = Data(repeating: 0xAA, count: 650)
        largeNext[250] = 0x55
        largeNext[649] = 0x11
        let xorPayload = encodeXorRleDelta(base: largeBase, next: largeNext)
        XCTAssertLessThan(xorPayload.count, 200)
        let decodedLarge = try NveDeltaCodec.decodeXorRleDelta(base: largeBase, deltaPayload: xorPayload)
        XCTAssertEqual(decodedLarge, largeNext)
    }

    // MARK: - Corrupt & Truncated Packet Rejection

    func testRejectsCorruptAndTruncatedPackets() {
        XCTAssertThrowsError(try NveTransportEnvelope.parse(Data([0x4E, 0x54, 1]))) { err in
            XCTAssertEqual(err as? NvePairProtocolError, .truncatedFrame)
        }
        XCTAssertThrowsError(try NveTransportEnvelope.parse(Data([0x00, 0x00, 1, 1, 0, 0, 0, 0]))) { err in
            XCTAssertEqual(err as? NvePairProtocolError, .invalidTransportMagic(0, 0))
        }
        XCTAssertThrowsError(try NveTransportEnvelope.parse(Data([0x4E, 0x54, 9, 1, 0, 0, 0, 0]))) { err in
            XCTAssertEqual(err as? NvePairProtocolError, .unsupportedTransportVersion(9))
        }
        XCTAssertThrowsError(try NveTransportEnvelope.parse(Data([0x4E, 0x54, 1, 99, 0, 0, 0, 0]))) { err in
            XCTAssertEqual(err as? NvePairProtocolError, .invalidTransportKind(99))
        }

        // Corrupt checksum in NVD1 packet
        var validNvd = NveDeltaPacket.encode(kind: .keyframe, seq: 1, payload: Data([1, 2, 3, 4]))
        validNvd[validNvd.count - 1] ^= 0xFF
        XCTAssertThrowsError(try NveDeltaPacket.parse(validNvd)) { err in
            if case .checksumMismatch = (err as? NvePairProtocolError) {
                return
            }
            XCTFail("Expected checksumMismatch, got \(err)")
        }

        // Truncated NVD1 packet
        let truncNvd = NveDeltaPacket.encode(kind: .keyframe, seq: 1, payload: Data([1, 2, 3, 4])).dropLast()
        XCTAssertThrowsError(try NveDeltaPacket.parse(Data(truncNvd))) { err in
            XCTAssertEqual(err as? NvePairProtocolError, .lengthMismatch(declared: 4, actual: 3))
        }
    }

    // MARK: - InputEvents = 11 Binary Encoding

    func testInputEventsBinaryEncodingAndBounds() throws {
        let events = [
            NveEvent(surfaceId: 1, nodeId: 42, timestampUs: 123_456, kind: .press, payload: .point(x: 12.5, y: 34.5)),
            NveEvent(surfaceId: 1, nodeId: 42, timestampUs: 123_500, kind: .textChange, payload: .text("hello")),
        ]
        let packet = try NvePairCodec.encodeInputEvents(events, sessionId: 0xDEAD_BEEF)
        let env = try NveTransportEnvelope.parse(packet)
        XCTAssertEqual(env.kind, .inputEvents)
        XCTAssertEqual(env.kind.rawValue, 11)
        XCTAssertEqual(env.sessionId, 0xDEAD_BEEF)
        let decoded = try NveEventRingCodec.decodeStrict(env.payload)
        XCTAssertEqual(decoded, events)

        // Oversized (> 64 KiB) rejected
        let oversized = Data(repeating: 0, count: NvePairProtocol.maxInputEventBytes + 1)
        XCTAssertThrowsError(try NvePairCodec.encodeRawInputEvents(oversized, sessionId: 1))
    }

    // MARK: - NvePairClient State Machine, Gap Recovery & Telemetry

    @MainActor
    func testPairClientHandshakeGapRecoveryPingAndEvents() throws {
        var nowUs: UInt64 = 100_000
        let mock = MockPairTransport()
        let client = NvePairClient(
            transportFactory: { mock },
            autoReconnect: false,
            heartbeatIntervalSeconds: 0,
            nowMicros: { nowUs }
        )
        var receivedFrames: [(UInt32, Data)] = []
        client.onFrame = { receivedFrames.append(($0, $1)) }

        XCTAssertEqual(client.state, .disconnected)
        try client.connect(uriString: "nve://pair?host=127.0.0.1&ws=8092&token=tok_swift")
        XCTAssertEqual(client.state, .connecting)
        XCTAssertEqual(mock.connectedUrl?.absoluteString, "ws://127.0.0.1:8092/ws/nvd")

        // Handshake Hello
        let helloBytes = try XCTUnwrap(client.beginHandshake())
        let helloEnv = try NveTransportEnvelope.parse(helloBytes)
        XCTAssertEqual(helloEnv.kind, .hello)
        let hello = try NveHelloPayload.decode(helloEnv.payload)
        XCTAssertEqual(hello.tokenProof, NvePairCrypto.deriveTokenProof(pairingToken: "tok_swift", schemaVersion: 1))

        // Server replies HelloAck
        let serverNonce: UInt64 = 0xCAFE_BABE_1234_5678
        let sid = NvePairCrypto.deriveSessionId(
            pairingToken: "tok_swift",
            schemaVersion: 1,
            clientNonce: hello.clientNonce,
            serverNonce: serverNonce
        )
        let ackPayload = NveHelloAckPayload(
            schemaVersion: 1,
            serverMaxWindow: 64,
            serverSeq: 0,
            serverNonce: serverNonce,
            resyncRequired: false
        )
        _ = try client.handleInboundBinary(NveTransportEnvelope.encode(kind: .helloAck, sessionId: sid, payload: ackPayload.encode()))
        XCTAssertEqual(client.state, .paired(sessionId: sid))

        // Keyframe seq=1
        let f1 = makeNveCommandBuffer(frameId: 1, text: "Hello SwiftUI")
        let nvd1 = NveDeltaPacket.encode(kind: .keyframe, seq: 1, baseSeq: 0, payload: f1)
        let out1 = try client.handleInboundBinary(NveTransportEnvelope.encode(kind: .deltaSingle, sessionId: sid, payload: nvd1))
        XCTAssertEqual(receivedFrames.count, 1)
        XCTAssertEqual(client.latestFrame, f1)
        XCTAssertEqual(client.latestSummary?.opCount, 4)
        XCTAssertEqual(out1.count, 1) // NVD1 Ack

        // Sequence gap: seq=3 arrives while seq=2 is missing -> triggers Nack + KeyframeRequest!
        let f3 = makeNveCommandBuffer(frameId: 3, text: "After gap")
        let nvd3Delta = NveDeltaPacket.encode(
            kind: .xorRleDelta,
            seq: 3,
            baseSeq: 2,
            payload: encodeXorRleDelta(base: f1, next: f3)
        )
        let gapOut = try client.handleInboundBinary(NveTransportEnvelope.encode(kind: .deltaSingle, sessionId: sid, payload: nvd3Delta))
        let kinds = try gapOut.map { try NveTransportEnvelope.parse($0).kind }
        XCTAssertTrue(kinds.contains(.keyframeRequest), "expected KeyframeRequest on sequence gap, got \(kinds)")
        XCTAssertEqual(client.telemetry.keyframeRequestsSent, 1)

        // Server recovers with fresh Keyframe seq=4
        let nvd4Kf = NveDeltaPacket.encode(kind: .keyframe, seq: 4, baseSeq: 0, payload: f3)
        _ = try client.handleInboundBinary(NveTransportEnvelope.encode(kind: .deltaSingle, sessionId: sid, payload: nvd4Kf))
        XCTAssertEqual(client.latestFrameSeq, 4)
        XCTAssertEqual(client.latestFrame, f3)

        // Ping -> Pong echo + Client Ping -> Pong RTT measurement
        let serverPing = NvePairCodec.encodePing(sessionId: sid, timestampUs: 55_555)
        let pongOut = try client.handleInboundBinary(serverPing)
        XCTAssertEqual(try NveTransportEnvelope.parse(try XCTUnwrap(pongOut.first)).kind, .pong)

        _ = client.sendPing()
        nowUs += 4_200
        _ = try client.handleInboundBinary(NvePairCodec.encodePong(sessionId: sid, timestampUs: 100_000))
        XCTAssertEqual(client.telemetry.lastRttUs, 4_200)
        XCTAssertEqual(client.telemetry.smoothedRttUs, 4_200)

        // InputEvents = 11 backchannel
        let evPacket = try XCTUnwrap(client.sendInputEvents([
            NveEvent(surfaceId: 1, nodeId: 2, timestampUs: nowUs, kind: .press, payload: .point(x: 5, y: 10)),
        ]))
        XCTAssertEqual(try NveTransportEnvelope.parse(evPacket).kind, .inputEvents)
        XCTAssertEqual(client.telemetry.eventsSent, 1)

        // Graceful disconnect sends Bye
        client.disconnect()
        XCTAssertEqual(client.state, .disconnected)
        XCTAssertEqual(try NveTransportEnvelope.parse(try XCTUnwrap(mock.sentFrames.last)).kind, .bye)
    }

    @MainActor
    func testPairClientHandlesHandshakeRejection() throws {
        let mock = MockPairTransport()
        let client = NvePairClient(transportFactory: { mock }, autoReconnect: false, heartbeatIntervalSeconds: 0)
        try client.connect(uriString: "nve://pair?host=127.0.0.1&ws=8092&token=bad_token")
        _ = client.beginHandshake()

        let rejectEnv = NveTransportEnvelope.encode(
            kind: .reject,
            sessionId: 0,
            payload: NveRejectPayload(reason: .invalidPairingToken, expectedSchema: 1).encode()
        )
        XCTAssertThrowsError(try client.handleInboundBinary(rejectEnv))
        if case let .error(msg) = client.state {
            XCTAssertTrue(msg.contains("invalidPairingToken"))
        } else {
            XCTFail("Expected .error state, got \(client.state)")
        }
    }
}
