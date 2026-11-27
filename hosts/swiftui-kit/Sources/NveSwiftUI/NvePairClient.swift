import Combine
import Foundation

/// `NveWireFrame` v2 logical channel identifiers matching `nve_wire_core::NveWireChannel`.
public enum NveWireChannel: UInt8, Sendable {
    case control = 0
    case uiScene = 1
    case asyncSlot = 2
    case inputEvents = 3
    case stateSync = 4
    case presence = 5
}

/// `NveWireFrame` v2 frame kinds matching `nve_wire_core::NveWireKind`.
public enum NveWireKind: UInt8, Sendable {
    case hello = 1
    case helloAck = 2
    case reject = 3
    case sceneKeyframe = 4
    case commandDelta = 5
    case nodePatchDelta = 6
    case slotChunk = 7
    case eventBatch = 8
    case crdtDelta = 9
    case presence = 10
    case hotPatch = 11
    case ping = 12
    case pong = 13
    case keyframeRequest = 14
    case bye = 15
}

public enum NvePairProtocolError: Error, Equatable, Sendable {
    case invalidUri(String)
    case truncated
    case invalidMagic
    case unsupportedVersion(UInt8)
    case invalidHeader
    case frameTooLarge(Int)
    case checksumMismatch
    case rejected(String)
    case notConnected
}

/// Thin `NveWireFrame` v2 codec (16-byte LE header + payload + optional 8-byte `XXH64` trailer).
public enum NveWireCodec {
    public static let magic: [UInt8] = [0x4E, 0x57] // "NW"
    public static let version: UInt8 = 2
    public static let headerSize = 16
    public static let trailerSize = 8
    public static let flagXxh64: UInt8 = 1 << 0
    public static let xxh64Seed: UInt64 = 0x4E56_4557_4952_4532 // "NVEWIRE2"
    public static let defaultSchemaVersion: UInt16 = 2
    public static let maxFrameBytes = 16 * 1024 * 1024

    public static func encodeFrame(
        channel: NveWireChannel,
        kind: NveWireKind,
        sequence: UInt32,
        payload: Data = Data(),
        flags: UInt8 = flagXxh64
    ) -> Data {
        let hasCk = (flags & flagXxh64) != 0
        var out = Data(capacity: headerSize + payload.count + (hasCk ? trailerSize : 0))
        out.append(contentsOf: magic)
        out.append(version)
        out.append(channel.rawValue)
        out.append(kind.rawValue)
        out.append(flags)
        appendLE(&out, UInt16(0))
        appendLE(&out, sequence)
        appendLE(&out, UInt32(payload.count))
        out.append(payload)
        if hasCk {
            let ck = NveXxh64.hash(out, seed: xxh64Seed)
            appendLE(&out, ck)
        }
        return out
    }

    public static func decodeFrame(
        _ data: Data,
        maxBytes: Int = maxFrameBytes
    ) throws -> (channel: NveWireChannel, kind: NveWireKind, sequence: UInt32, payload: Data) {
        guard data.count >= headerSize else { throw NvePairProtocolError.truncated }
        let b = [UInt8](data)
        guard b[0] == magic[0], b[1] == magic[1] else { throw NvePairProtocolError.invalidMagic }
        guard b[2] == version else { throw NvePairProtocolError.unsupportedVersion(b[2]) }
        guard let ch = NveWireChannel(rawValue: b[3]), let kind = NveWireKind(rawValue: b[4]) else {
            throw NvePairProtocolError.invalidHeader
        }
        let reserved = readU16(b, 6)
        guard reserved == 0 else { throw NvePairProtocolError.invalidHeader }
        let seq = readU32(b, 8)
        let len = Int(readU32(b, 12))
        guard len <= maxBytes else { throw NvePairProtocolError.frameTooLarge(len) }
        let hasCk = (b[5] & flagXxh64) != 0
        let expectedTotal = headerSize + len + (hasCk ? trailerSize : 0)
        guard b.count == expectedTotal else { throw NvePairProtocolError.truncated }
        let bodyEnd = headerSize + len
        if hasCk {
            let expectedCk = readU64(b, bodyEnd)
            let actualCk = NveXxh64.hash(Data(b[0..<bodyEnd]), seed: xxh64Seed)
            guard actualCk == expectedCk else { throw NvePairProtocolError.checksumMismatch }
        }
        return (ch, kind, seq, Data(b[headerSize..<bodyEnd]))
    }

    public static func encodeHello(bearerToken: String, clientName: String = "nve-swiftui") -> Data {
        let tok = Data(bearerToken.utf8.prefix(512))
        let name = Data(clientName.utf8.prefix(128))
        var out = Data(capacity: 10 + tok.count + name.count)
        appendLE(&out, defaultSchemaVersion)
        appendLE(&out, UInt32(0x0F))
        appendLE(&out, UInt16(tok.count))
        appendLE(&out, UInt16(name.count))
        out.append(tok)
        out.append(name)
        return out
    }

    public static func decodeHelloAck(_ data: Data) throws -> (sessionId: UInt32, schemaVersion: UInt16) {
        guard data.count == 12 else { throw NvePairProtocolError.truncated }
        let b = [UInt8](data)
        return (readU32(b, 0), readU16(b, 4))
    }

    private static func appendLE<T: FixedWidthInteger>(_ data: inout Data, _ value: T) {
        var le = value.littleEndian
        withUnsafeBytes(of: &le) { data.append(contentsOf: $0) }
    }
    private static func readU16(_ b: [UInt8], _ o: Int) -> UInt16 {
        UInt16(b[o]) | (UInt16(b[o + 1]) << 8)
    }
    private static func readU32(_ b: [UInt8], _ o: Int) -> UInt32 {
        UInt32(b[o]) | (UInt32(b[o + 1]) << 8) | (UInt32(b[o + 2]) << 16) | (UInt32(b[o + 3]) << 24)
    }
    private static func readU64(_ b: [UInt8], _ o: Int) -> UInt64 {
        UInt64(readU32(b, o)) | (UInt64(readU32(b, o + 4)) << 32)
    }
}

public struct NveParsedPairUri: Equatable, Sendable {
    public let webSocketURL: URL
    public let token: String
    public let certHashHex: String?

    public static func parse(_ raw: String, fallbackToken: String? = nil) throws -> NveParsedPairUri {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw NvePairProtocolError.invalidUri("empty URI") }
        if trimmed.lowercased().hasPrefix("nve://") {
            guard let comps = URLComponents(string: trimmed) else {
                throw NvePairProtocolError.invalidUri("malformed nve://pair URI")
            }
            let q = Dictionary(uniqueKeysWithValues: (comps.queryItems ?? []).compactMap { item in
                item.value.map { (item.name, $0) }
            })
            let host = q["host"] ?? "127.0.0.1"
            let port = q["wss"] ?? q["ws"] ?? q["quic"] ?? q["udp"] ?? ""
            let tok = (q["token"]?.isEmpty == false ? q["token"] : fallbackToken) ?? ""
            guard !port.isEmpty, !tok.isEmpty else {
                throw NvePairProtocolError.invalidUri("URI requires port and token")
            }
            let scheme = q["wss"] != nil ? "wss" : "ws"
            guard let url = URL(string: "\(scheme)://\(host):\(port)/nve-wire") else {
                throw NvePairProtocolError.invalidUri("invalid resolved WebSocket URL")
            }
            return NveParsedPairUri(webSocketURL: url, token: tok, certHashHex: q["cert_hash"])
        }
        guard let url = URL(string: trimmed), let tok = fallbackToken, !tok.isEmpty else {
            throw NvePairProtocolError.invalidUri("direct URL requires non-empty token")
        }
        return NveParsedPairUri(webSocketURL: url, token: tok, certHashHex: nil)
    }
}

public struct NveCommandBufferSummary: Equatable, Sendable {
    public let surfaceId: UInt32
    public let frameId: UInt64
    public let opCount: UInt32
    public let flags: UInt16
}

public enum NvePairConnectionState: Equatable, Sendable {
    case disconnected
    case connecting
    case paired(sessionId: UInt32)
    case reconnecting(attempt: Int)
    case error(String)

    public var isPaired: Bool {
        if case .paired = self { return true }
        return false
    }

    public var label: String {
        switch self {
        case .disconnected: return "Disconnected"
        case .connecting: return "Connecting…"
        case let .paired(sid): return String(format: "Paired (0x%08X)", sid)
        case let .reconnecting(attempt): return "Reconnecting (#\(attempt))…"
        case let .error(msg): return "Error: \(msg)"
        }
    }
}

public struct NveSyncTelemetry: Equatable, Sendable {
    public var sessionId: UInt32 = 0
    public var framesReceived: UInt64 = 0
    public var keyframesReceived: UInt64 = 0
    public var bytesReceived: UInt64 = 0
    public var eventsSent: UInt64 = 0
    public var smoothedRttMs: Double?
    public var fps: Double = 0
    public var bytesPerSecRx: Double = 0
    public var compressionRatio: Double = 1.0
}

/// Thin SwiftUI pairing client speaking `NveWireFrame` v2 over WebSocket (or `nve-wire` FFI).
@MainActor
public final class NvePairClient: ObservableObject {
    @Published public private(set) var state: NvePairConnectionState = .disconnected
    @Published public private(set) var telemetry = NveSyncTelemetry()
    @Published public private(set) var latestFrame: Data?
    @Published public private(set) var latestFrameSeq: UInt32 = 0
    @Published public private(set) var latestSummary: NveCommandBufferSummary?

    private var task: URLSessionWebSocketTask?
    private var nextSeq: UInt32 = 2
    private var token: String = ""

    public init() {}

    public func connect(uriString: String, fallbackToken: String? = nil) throws {
        disconnect()
        let parsed = try NveParsedPairUri.parse(uriString, fallbackToken: fallbackToken)
        self.token = parsed.token
        self.state = .connecting
        let ws = URLSession.shared.webSocketTask(with: parsed.webSocketURL)
        self.task = ws
        ws.resume()
        let hello = NveWireCodec.encodeFrame(
            channel: .control,
            kind: .hello,
            sequence: 1,
            payload: NveWireCodec.encodeHello(bearerToken: parsed.token)
        )
        ws.send(.data(hello)) { [weak self] err in
            if let err {
                Task { @MainActor in self?.state = .error(err.localizedDescription) }
            }
        }
        receiveNext()
    }

    public func disconnect() {
        if case .paired = state, let task {
            let bye = NveWireCodec.encodeFrame(channel: .control, kind: .bye, sequence: allocSeq())
            task.send(.data(bye)) { _ in }
        }
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        state = .disconnected
    }

    public func requestKeyframe() {
        guard state.isPaired, let task else { return }
        let req = NveWireCodec.encodeFrame(channel: .control, kind: .keyframeRequest, sequence: allocSeq())
        task.send(.data(req)) { _ in }
    }

    @discardableResult
    public func sendInputEvents(_ events: [NveEvent]) throws -> Data {
        guard state.isPaired else { throw NvePairProtocolError.notConnected }
        let ring = NveEventRingCodec.encode(events)
        let frame = NveWireCodec.encodeFrame(
            channel: .inputEvents,
            kind: .eventBatch,
            sequence: allocSeq(),
            payload: ring
        )
        telemetry.eventsSent += 1
        task?.send(.data(frame)) { _ in }
        return frame
    }

    /// Processes one inbound `NveWireFrame` v2 binary message (also callable directly in tests).
    @discardableResult
    public func handleInboundBinary(_ data: Data) throws -> Data? {
        let decoded = try NveWireCodec.decodeFrame(data)
        telemetry.bytesReceived += UInt64(data.count)
        switch decoded.kind {
        case .helloAck:
            let ack = try NveWireCodec.decodeHelloAck(decoded.payload)
            telemetry.sessionId = ack.sessionId
            state = .paired(sessionId: ack.sessionId)
            return nil
        case .reject:
            state = .error("Pairing rejected")
            throw NvePairProtocolError.rejected("Pairing rejected")
        case .sceneKeyframe, .commandDelta:
            telemetry.framesReceived += 1
            if decoded.kind == .sceneKeyframe { telemetry.keyframesReceived += 1 }
            latestFrame = decoded.payload
            latestFrameSeq = decoded.sequence
            return nil
        case .ping:
            return NveWireCodec.encodeFrame(
                channel: .control,
                kind: .pong,
                sequence: allocSeq(),
                payload: decoded.payload
            )
        case .bye:
            state = .disconnected
            return nil
        default:
            return nil
        }
    }

    private func allocSeq() -> UInt32 {
        let s = nextSeq
        nextSeq &+= 1
        if nextSeq == 0 { nextSeq = 1 }
        return s
    }

    private func receiveNext() {
        task?.receive { [weak self] result in
            Task { @MainActor in
                guard let self else { return }
                switch result {
                case let .success(.data(bytes)):
                    if let reply = try? self.handleInboundBinary(bytes) {
                        self.task?.send(.data(reply)) { _ in }
                    }
                    self.receiveNext()
                case .success:
                    self.receiveNext()
                case let .failure(err):
                    if self.state != .disconnected {
                        self.state = .error(err.localizedDescription)
                    }
                }
            }
        }
    }
}
