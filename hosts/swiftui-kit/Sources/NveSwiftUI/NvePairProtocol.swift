import Foundation

/// Wire constants for `NVT1` transport framing and `NVD1` frame-delta packets.
/// Matches `crates/nve-interop/src/sync_transport/protocol.rs` and `crates/nve-interop/src/sync_delta/`.
public enum NvePairProtocol {
    public static let transportMagic: [UInt8] = [0x4E, 0x54] // "NT"
    public static let transportVersion: UInt8 = 1
    public static let transportHeaderSize = 8
    public static let helloPayloadSize = 32
    public static let helloAckPayloadSize = 20
    public static let deltaMagic: [UInt8] = [0x4E, 0x56, 0x44, 0x31] // "NVD1"
    public static let deltaVersion: UInt8 = 1
    public static let deltaHeaderSize = 32
    public static let flagRetransmit: UInt16 = 1 << 0
    public static let defaultSchemaVersion: UInt16 = 1
    public static let defaultSyncWindow = 64
    public static let minSyncWindow = 4
    public static let maxSyncWindow = 1024
    public static let maxDecodedFrameBytes = 16 * 1024 * 1024
    public static let maxWireFrameBytes = maxDecodedFrameBytes + deltaHeaderSize
    public static let maxInputEventBytes = 64 * 1024
    public static let nveAbiMagic: UInt32 = 0x3145_564E // "NVE1" LE
    public static let nveAbiVersion: UInt16 = 1
    public static let nveCmdHeaderSize = 24
    fileprivate static let cmdCopyRun: UInt8 = 1
    fileprivate static let cmdLiteralOp: UInt8 = 2
    fileprivate static let cmdXorOp: UInt8 = 3
}

public enum NveTransportMsgKind: UInt8, Sendable, Equatable {
    case hello = 1, helloAck = 2, reject = 3, deltaSingle = 4, deltaFrag = 5, fragNack = 6
    case ping = 7, pong = 8, keyframeRequest = 9, bye = 10, inputEvents = 11
}

public enum NveRejectReason: UInt8, Sendable, Equatable {
    case invalidPairingToken = 1, schemaVersionMismatch = 2, unsupportedProtocolVersion = 3, serverBusy = 4
}

public enum NveDeltaPacketKind: UInt8, Sendable, Equatable {
    case keyframe = 1, xorRleDelta = 2, commandOpDelta = 3, ack = 4, nack = 5
}

public enum NvePairProtocolError: Error, Equatable, Sendable {
    case invalidUri(String), truncatedFrame, invalidTransportMagic(UInt8, UInt8)
    case unsupportedTransportVersion(UInt8), invalidTransportKind(UInt8), malformedFrame(String)
    case frameTooLarge(declared: Int, max: Int), invalidDeltaMagic, unsupportedDeltaVersion(UInt8)
    case invalidDeltaKind(UInt8), lengthMismatch(declared: Int, actual: Int)
    case checksumMismatch(expected: UInt64, actual: UInt64)
    case baseSequenceMismatch(expected: UInt32, actual: UInt32), corruptPayload(String)
}

public struct NvePairUri: Equatable, Sendable {
    public let wsUrl: URL
    public let token, host: String
    public let wsPort, udpPort: Int?
    public let width, height: Double?
    public let schemaVersion: UInt16

    public init(wsUrl: URL, token: String, host: String, wsPort: Int? = nil, udpPort: Int? = nil, width: Double? = nil, height: Double? = nil, schemaVersion: UInt16 = 1) {
        self.wsUrl = wsUrl; self.token = token; self.host = host; self.wsPort = wsPort
        self.udpPort = udpPort; self.width = width; self.height = height; self.schemaVersion = schemaVersion
    }

    public static func parse(_ raw: String, fallbackToken: String? = nil) throws -> NvePairUri {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw NvePairProtocolError.invalidUri("URI is empty") }
        let fallback = fallbackToken?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        if trimmed.lowercased().hasPrefix("nve://") {
            guard let comps = URLComponents(string: trimmed), comps.host?.lowercased() == "pair" else {
                throw NvePairProtocolError.invalidUri("expected nve://pair URI")
            }
            var p: [String: String] = [:]
            for item in comps.queryItems ?? [] { if let v = item.value { p[item.name.lowercased()] = v.trimmingCharacters(in: .whitespacesAndNewlines) } }
            let wsParam = p["ws"] ?? p["endpoint"] ?? p["connect"] ?? ""
            guard !wsParam.isEmpty else { throw NvePairProtocolError.invalidUri("missing ws parameter") }
            let token = !fallback.isEmpty ? fallback : (p["token"] ?? "")
            guard !token.isEmpty else { throw NvePairProtocolError.invalidUri("missing pairing token") }
            var schema: UInt16 = NvePairProtocol.defaultSchemaVersion
            if let s = p["schema"], !s.isEmpty {
                guard let parsed = UInt16(s), parsed > 0 else { throw NvePairProtocolError.invalidUri("invalid schema '\(s)'") }
                schema = parsed
            }
            let udp = p["udp"].flatMap(Int.init), w = p["w"].flatMap(Double.init), h = p["h"].flatMap(Double.init)
            if wsParam.lowercased().hasPrefix("ws://") || wsParam.lowercased().hasPrefix("wss://") {
                guard let url = URL(string: wsParam), let host = url.host, !host.isEmpty else {
                    throw NvePairProtocolError.invalidUri("invalid ws URL '\(wsParam)'")
                }
                return NvePairUri(wsUrl: url, token: token, host: host, wsPort: url.port, udpPort: udp, width: w, height: h, schemaVersion: schema)
            }
            guard let port = Int(wsParam), (1 ... 65535).contains(port) else { throw NvePairProtocolError.invalidUri("invalid ws port '\(wsParam)'") }
            let host = (p["host"]?.isEmpty == false) ? p["host"]! : "127.0.0.1"
            let hostPart = (host.contains(":") && !host.hasPrefix("[")) ? "[\(host)]" : host
            guard let wsUrl = URL(string: "ws://\(hostPart):\(port)/ws/nvd") else { throw NvePairProtocolError.invalidUri("invalid host '\(host)'") }
            return NvePairUri(wsUrl: wsUrl, token: token, host: host, wsPort: port, udpPort: udp, width: w, height: h, schemaVersion: schema)
        }

        let lower = trimmed.lowercased()
        let normalized = (lower.hasPrefix("ws://") || lower.hasPrefix("wss://")) ? trimmed : "ws://\(trimmed)"
        guard var comps = URLComponents(string: normalized), let scheme = comps.scheme?.lowercased(),
              scheme == "ws" || scheme == "wss", let host = comps.host, !host.isEmpty else {
            throw NvePairProtocolError.invalidUri("unsupported scheme or missing host")
        }
        let uriTok = comps.queryItems?.first(where: { $0.name.lowercased() == "token" })?.value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let token = !fallback.isEmpty ? fallback : uriTok
        guard !token.isEmpty else { throw NvePairProtocolError.invalidUri("missing pairing token") }
        if comps.path.isEmpty { comps.path = "/ws/nvd" }
        guard let wsUrl = comps.url else { throw NvePairProtocolError.invalidUri("invalid WebSocket URL") }
        return NvePairUri(wsUrl: wsUrl, token: token, host: host, wsPort: comps.port)
    }
}

public enum NvePairCrypto {
    public static let fnvOffsetBasis: UInt64 = 0xCBF2_9CE4_8422_2325
    public static let fnvPrime: UInt64 = 0x0000_0100_0000_01B3

    public static func fnv1a64(_ data: Data) -> UInt64 { fnv1a64Pair(data, Data()) }

    public static func fnv1a64Pair(_ a: Data, _ b: Data) -> UInt64 {
        a.withUnsafeBytes { ra in
            b.withUnsafeBytes { rb in
                var h = fnvOffsetBasis
                for byte in ra { h ^= UInt64(byte); h = h &* fnvPrime }
                for byte in rb { h ^= UInt64(byte); h = h &* fnvPrime }
                return h
            }
        }
    }

    public static func deriveTokenProof(pairingToken: String, schemaVersion: UInt16 = 1) -> Data {
        let tok = Data(pairingToken.utf8)
        var ver = Data(capacity: 2); appendLE(schemaVersion, to: &ver)
        var pre = Data("NVE-PAIR-A-v1:".utf8); pre.append(ver)
        var suf = Data(":NVE-PAIR-B-v1".utf8); suf.append(ver)
        var out = Data(capacity: 16)
        appendLE(fnv1a64Pair(pre, tok), to: &out); appendLE(fnv1a64Pair(tok, suf), to: &out)
        return out
    }

    public static func deriveSessionId(pairingToken: String, schemaVersion: UInt16 = 1, clientNonce: UInt64, serverNonce: UInt64) -> UInt32 {
        var mat = Data(capacity: 18)
        appendLE(schemaVersion, to: &mat); appendLE(clientNonce, to: &mat); appendLE(serverNonce, to: &mat)
        let h = fnv1a64Pair(Data(pairingToken.utf8), mat)
        let folded = UInt32(truncatingIfNeeded: h ^ (h >> 32)) ^ 0x5A3C_96F1
        return folded == 0 ? 1 : folded
    }

    public static func mixNonce(_ seed: UInt64) -> UInt64 {
        var x = seed &* 0x9E37_79B9_7F4A_7C15 &+ 0xA076_1D64_78BD_642F
        x ^= x >> 30; x = x &* 0xBF58_476D_1CE4_E5B9
        x ^= x >> 27; x = x &* 0x94D0_49BB_1331_11EB
        return x ^ (x >> 31)
    }
}

public struct NveTransportEnvelope: Equatable, Sendable {
    public let kind: NveTransportMsgKind
    public let sessionId: UInt32
    public let payload: Data

    public init(kind: NveTransportMsgKind, sessionId: UInt32, payload: Data = Data()) {
        self.kind = kind; self.sessionId = sessionId; self.payload = payload
    }

    public static func parse(_ data: Data) throws -> NveTransportEnvelope {
        guard data.count >= 8 else { throw NvePairProtocolError.truncatedFrame }
        return try data.withUnsafeBytes { raw in
            guard raw[0] == NvePairProtocol.transportMagic[0], raw[1] == NvePairProtocol.transportMagic[1] else {
                throw NvePairProtocolError.invalidTransportMagic(raw[0], raw[1])
            }
            guard raw[2] == NvePairProtocol.transportVersion else { throw NvePairProtocolError.unsupportedTransportVersion(raw[2]) }
            guard let kind = NveTransportMsgKind(rawValue: raw[3]) else { throw NvePairProtocolError.invalidTransportKind(raw[3]) }
            return NveTransportEnvelope(kind: kind, sessionId: readU32LE(raw, at: 4), payload: data[(data.startIndex + 8)...])
        }
    }

    public static func encode(kind: NveTransportMsgKind, sessionId: UInt32, payload: Data = Data()) -> Data {
        var out = Data(capacity: 8 + payload.count)
        out.append(contentsOf: NvePairProtocol.transportMagic); out.append(NvePairProtocol.transportVersion); out.append(kind.rawValue)
        appendLE(sessionId, to: &out); out.append(payload)
        return out
    }
}

public struct NveHelloPayload: Equatable, Sendable {
    public let schemaVersion: UInt16, maxWindow: UInt16, clientLastSeq: UInt32, clientNonce: UInt64, tokenProof: Data
    public init(schemaVersion: UInt16 = 1, maxWindow: UInt16 = 64, clientLastSeq: UInt32 = 0, clientNonce: UInt64, tokenProof: Data) {
        self.schemaVersion = schemaVersion; self.maxWindow = maxWindow; self.clientLastSeq = clientLastSeq
        self.clientNonce = clientNonce; self.tokenProof = tokenProof
    }
    public func encode() -> Data {
        var out = Data(capacity: 32)
        appendLE(schemaVersion, to: &out); appendLE(maxWindow, to: &out); appendLE(clientLastSeq, to: &out); appendLE(clientNonce, to: &out)
        out.append(tokenProof.prefix(16)); if tokenProof.count < 16 { out.append(Data(repeating: 0, count: 16 - tokenProof.count)) }
        return out
    }
    public static func decode(_ data: Data) throws -> NveHelloPayload {
        guard data.count == 32 else { throw NvePairProtocolError.malformedFrame("invalid Hello length \(data.count)") }
        return data.withUnsafeBytes { r in
            NveHelloPayload(schemaVersion: readU16LE(r, at: 0), maxWindow: readU16LE(r, at: 2), clientLastSeq: readU32LE(r, at: 4), clientNonce: readU64LE(r, at: 8), tokenProof: Data(r[16 ..< 32]))
        }
    }
}

public struct NveHelloAckPayload: Equatable, Sendable {
    public let schemaVersion: UInt16, serverMaxWindow: UInt16, serverSeq: UInt32, serverNonce: UInt64, resyncRequired: Bool
    public init(schemaVersion: UInt16, serverMaxWindow: UInt16, serverSeq: UInt32, serverNonce: UInt64, resyncRequired: Bool) {
        self.schemaVersion = schemaVersion; self.serverMaxWindow = serverMaxWindow; self.serverSeq = serverSeq
        self.serverNonce = serverNonce; self.resyncRequired = resyncRequired
    }
    public func encode() -> Data {
        var out = Data(capacity: 20)
        appendLE(schemaVersion, to: &out); appendLE(serverMaxWindow, to: &out); appendLE(serverSeq, to: &out); appendLE(serverNonce, to: &out)
        out.append(contentsOf: [resyncRequired ? 1 : 0, 0, 0, 0])
        return out
    }
    public static func decode(_ data: Data) throws -> NveHelloAckPayload {
        guard data.count == 20 else { throw NvePairProtocolError.malformedFrame("invalid HelloAck length \(data.count)") }
        return data.withUnsafeBytes { r in
            NveHelloAckPayload(schemaVersion: readU16LE(r, at: 0), serverMaxWindow: readU16LE(r, at: 2), serverSeq: readU32LE(r, at: 4), serverNonce: readU64LE(r, at: 8), resyncRequired: r[16] != 0)
        }
    }
}

public struct NveRejectPayload: Equatable, Sendable {
    public let reason: NveRejectReason, expectedSchema: UInt16
    public init(reason: NveRejectReason, expectedSchema: UInt16) { self.reason = reason; self.expectedSchema = expectedSchema }
    public func encode() -> Data { var out = Data([reason.rawValue]); appendLE(expectedSchema, to: &out); out.append(0); return out }
    public static func decode(_ data: Data) throws -> NveRejectPayload {
        guard data.count >= 3 else { throw NvePairProtocolError.malformedFrame("truncated Reject payload") }
        return try data.withUnsafeBytes { r in
            guard let reason = NveRejectReason(rawValue: r[0]) else { throw NvePairProtocolError.malformedFrame("unknown Reject reason \(r[0])") }
            return NveRejectPayload(reason: reason, expectedSchema: readU16LE(r, at: 1))
        }
    }
}

public enum NvePairCodec {
    public static func encodePing(sessionId: UInt32, timestampUs: UInt64) -> Data {
        var p = Data(capacity: 8); appendLE(timestampUs, to: &p)
        return NveTransportEnvelope.encode(kind: .ping, sessionId: sessionId, payload: p)
    }
    public static func encodePong(sessionId: UInt32, timestampUs: UInt64) -> Data {
        var p = Data(capacity: 8); appendLE(timestampUs, to: &p)
        return NveTransportEnvelope.encode(kind: .pong, sessionId: sessionId, payload: p)
    }
    public static func decodeTimestampUs(_ payload: Data) throws -> UInt64 {
        guard payload.count == 8 else { throw NvePairProtocolError.malformedFrame("invalid Ping/Pong payload") }
        return payload.withUnsafeBytes { readU64LE($0, at: 0) }
    }
    public static func encodeKeyframeRequest(sessionId: UInt32, lastSeq: UInt32, reason: UInt8) -> Data {
        var p = Data(capacity: 8); appendLE(lastSeq, to: &p); p.append(contentsOf: [reason, 0, 0, 0])
        return NveTransportEnvelope.encode(kind: .keyframeRequest, sessionId: sessionId, payload: p)
    }
    public static func decodeKeyframeRequest(_ payload: Data) throws -> (lastSeq: UInt32, reason: UInt8) {
        guard payload.count == 8 else { throw NvePairProtocolError.malformedFrame("invalid KeyframeRequest") }
        return payload.withUnsafeBytes { (readU32LE($0, at: 0), $0[4]) }
    }
    public static func encodeBye(sessionId: UInt32) -> Data { NveTransportEnvelope.encode(kind: .bye, sessionId: sessionId) }
    public static func encodeInputEvents(_ events: [NveEvent], sessionId: UInt32) throws -> Data {
        try encodeRawInputEvents(NveEventRingCodec.encode(events), sessionId: sessionId)
    }
    public static func encodeRawInputEvents(_ eventRing: Data, sessionId: UInt32) throws -> Data {
        if eventRing.count > NvePairProtocol.maxInputEventBytes {
            throw NvePairProtocolError.frameTooLarge(declared: eventRing.count, max: NvePairProtocol.maxInputEventBytes)
        }
        guard eventRing.count >= 4 else { throw NvePairProtocolError.malformedFrame("empty InputEvents") }
        do { _ = try NveEventRingCodec.decodeStrict(eventRing) } catch { throw NvePairProtocolError.malformedFrame("corrupt EventRing") }
        return NveTransportEnvelope.encode(kind: .inputEvents, sessionId: sessionId, payload: eventRing)
    }
}

public struct NveDeltaPacketHeader: Equatable, Sendable {
    public let version: UInt8, kind: NveDeltaPacketKind, flags: UInt16
    public let seq: UInt32, baseSeq: UInt32, ackSeq: UInt32, payloadLen: UInt32, checksum: UInt64
}

public struct NveDeltaPacket: Equatable, Sendable {
    public let header: NveDeltaPacketHeader, payload: Data

    public static func parse(_ data: Data) throws -> NveDeltaPacket {
        guard data.count >= 32 else { throw NvePairProtocolError.truncatedFrame }
        return try data.withUnsafeBytes { raw in
            for i in 0 ..< 4 where raw[i] != NvePairProtocol.deltaMagic[i] { throw NvePairProtocolError.invalidDeltaMagic }
            guard raw[4] == NvePairProtocol.deltaVersion else { throw NvePairProtocolError.unsupportedDeltaVersion(raw[4]) }
            guard let kind = NveDeltaPacketKind(rawValue: raw[5]) else { throw NvePairProtocolError.invalidDeltaKind(raw[5]) }
            let payloadLen = readU32LE(raw, at: 20), checksum = readU64LE(raw, at: 24), actual = data.count - 32
            guard actual == Int(payloadLen) else { throw NvePairProtocolError.lengthMismatch(declared: Int(payloadLen), actual: actual) }
            let payload = data[(data.startIndex + 32)...]
            let computed = NvePairCrypto.fnv1a64Pair(data[data.startIndex ..< (data.startIndex + 24)], payload)
            guard computed == checksum else { throw NvePairProtocolError.checksumMismatch(expected: checksum, actual: computed) }
            return NveDeltaPacket(
                header: NveDeltaPacketHeader(version: raw[4], kind: kind, flags: readU16LE(raw, at: 6), seq: readU32LE(raw, at: 8), baseSeq: readU32LE(raw, at: 12), ackSeq: readU32LE(raw, at: 16), payloadLen: payloadLen, checksum: checksum),
                payload: payload
            )
        }
    }

    public static func encode(kind: NveDeltaPacketKind, flags: UInt16 = 0, seq: UInt32 = 0, baseSeq: UInt32 = 0, ackSeq: UInt32 = 0, payload: Data = Data()) -> Data {
        var out = Data(capacity: 32 + payload.count)
        out.append(contentsOf: NvePairProtocol.deltaMagic); out.append(NvePairProtocol.deltaVersion); out.append(kind.rawValue)
        appendLE(flags, to: &out); appendLE(seq, to: &out); appendLE(baseSeq, to: &out); appendLE(ackSeq, to: &out)
        appendLE(UInt32(payload.count), to: &out); appendLE(NvePairCrypto.fnv1a64Pair(out, payload), to: &out); out.append(payload)
        return out
    }
}

public struct NveCommandBufferSummary: Equatable, Sendable {
    public let flags: UInt16, surfaceId: UInt32, frameId: UInt64, opCount: UInt32, byteCount: Int
}

public enum NveDeltaCodec {
    public static func decodeXorRleDelta(base: Data, deltaPayload: Data) throws -> Data {
        var pos = 0; let targetU64 = try readVarint(deltaPayload, pos: &pos)
        guard targetU64 <= UInt64(NvePairProtocol.maxDecodedFrameBytes) else {
            throw NvePairProtocolError.frameTooLarge(declared: Int(clamping: targetU64), max: NvePairProtocol.maxDecodedFrameBytes)
        }
        let targetLen = Int(targetU64), baseBytes = Array(base), pBytes = Array(deltaPayload)
        var out = Data(); out.reserveCapacity(targetLen)
        while out.count < targetLen {
            guard pos < pBytes.count else { throw NvePairProtocolError.truncatedFrame }
            let tag = pBytes[pos]; pos += 1
            if (tag & 0x80) == 0 {
                let runU64 = tag == 0 ? try readVarint(deltaPayload, pos: &pos) : UInt64(tag)
                guard runU64 > 0, runU64 <= UInt64(targetLen - out.count) else { throw NvePairProtocolError.corruptPayload("zero-run") }
                let s = out.count, e = s + Int(runU64), fb = min(e, baseBytes.count)
                if s < fb { out.append(contentsOf: baseBytes[s ..< fb]) }
                let pad = e - max(fb, s); if pad > 0 { out.append(contentsOf: repeatElement(UInt8(0), count: pad)) }
            } else {
                let litU64 = tag == 0x80 ? try readVarint(deltaPayload, pos: &pos) : UInt64(tag & 0x7F)
                guard litU64 > 0, litU64 <= UInt64(targetLen - out.count) else { throw NvePairProtocolError.corruptPayload("literal") }
                let litLen = Int(litU64); guard pBytes.count - pos >= litLen else { throw NvePairProtocolError.truncatedFrame }
                let bo = out.count
                for k in 0 ..< litLen { out.append(((bo + k) < baseBytes.count ? baseBytes[bo + k] : 0) ^ pBytes[pos + k]) }
                pos += litLen
            }
        }
        guard pos == pBytes.count else { throw NvePairProtocolError.corruptPayload("trailing xor-rle") }
        return out
    }

    public static func scanNveCommandSlices(_ data: Data) -> (NveCommandBufferSummary, [Range<Int>])? {
        guard data.count >= 24 else { return nil }
        return data.withUnsafeBytes { raw in
            guard readU32LE(raw, at: 0) == NvePairProtocol.nveAbiMagic, readU16LE(raw, at: 4) == NvePairProtocol.nveAbiVersion else { return nil }
            let flags = readU16LE(raw, at: 6), sid = readU32LE(raw, at: 8), fid = readU64LE(raw, at: 12), ops = readU32LE(raw, at: 20)
            var pos = 24, ranges: [Range<Int>] = []; ranges.reserveCapacity(min(Int(ops), 4096))
            func take(_ n: Int) -> Bool { guard n >= 0, n <= raw.count - pos else { return false }; pos += n; return true }
            func u16() -> UInt16? { guard pos + 2 <= raw.count else { return nil }; defer { pos += 2 }; return readU16LE(raw, at: pos) }
            func u32() -> UInt32? { guard pos + 4 <= raw.count else { return nil }; defer { pos += 4 }; return readU32LE(raw, at: pos) }
            for _ in 0 ..< ops {
                let start = pos; guard pos < raw.count else { return nil }; let op = raw[pos]; pos += 1
                switch op {
                case 1: guard take(6), let n = u16(), take(Int(n)) else { return nil }
                case 2: guard take(4) else { return nil }
                case 3: guard take(4), let c = u32() else { return nil }; let (b, ov) = Int(c).multipliedReportingOverflow(by: 4); guard !ov, take(b) else { return nil }
                case 4: guard take(4), let l = u32(), take(Int(l)) else { return nil }
                case 5: guard take(81) else { return nil }
                case 6: guard take(36) else { return nil }
                case 7:
                    guard take(4), let kl = u16(), take(Int(kl)), pos < raw.count else { return nil }
                    let vt = raw[pos]; pos += 1
                    switch vt {
                    case 0: break
                    case 1: guard take(1) else { return nil }
                    case 2, 3: guard take(8) else { return nil }
                    case 4, 5: guard let l = u32(), take(Int(l)) else { return nil }
                    default: return nil
                    }
                case 8: guard take(12) else { return nil }
                case 9: guard take(4), let nl = u16(), take(Int(nl)), let pl = u32(), take(Int(pl)) else { return nil }
                case 10: guard take(1) else { return nil }
                default: return nil
                }
                ranges.append(start ..< pos)
            }
            guard pos == raw.count else { return nil }
            return (NveCommandBufferSummary(flags: flags, surfaceId: sid, frameId: fid, opCount: ops, byteCount: raw.count), ranges)
        }
    }

    public static func decodeCommandOpDelta(base: Data, deltaPayload: Data) throws -> Data {
        guard let (_, prev) = scanNveCommandSlices(base) else { throw NvePairProtocolError.corruptPayload("invalid NVE1 base") }
        guard deltaPayload.count >= 2 else { throw NvePairProtocolError.truncatedFrame }
        let flags = deltaPayload.withUnsafeBytes { readU16LE($0, at: 0) }
        var pos = 2
        let sid = try readVarint(deltaPayload, pos: &pos), fid = try readVarint(deltaPayload, pos: &pos), ops = try readVarint(deltaPayload, pos: &pos)
        guard sid <= UInt64(UInt32.max), ops <= UInt64(UInt32.max) else { throw NvePairProtocolError.corruptPayload("header overflow") }
        var out = Data(capacity: min(max(base.count, 24), 64 * 1024))
        appendLE(NvePairProtocol.nveAbiMagic, to: &out); appendLE(NvePairProtocol.nveAbiVersion, to: &out)
        appendLE(flags, to: &out); appendLE(UInt32(sid), to: &out); appendLE(fid, to: &out); appendLE(UInt32(ops), to: &out)
        var emitted: UInt32 = 0; let pStart = deltaPayload.startIndex, bStart = base.startIndex
        while pos < deltaPayload.count {
            let tag = deltaPayload[pStart + pos]; pos += 1
            switch tag {
            case NvePairProtocol.cmdCopyRun:
                let s = try readVarint(deltaPayload, pos: &pos), c = try readVarint(deltaPayload, pos: &pos)
                guard c > 0, s <= UInt64(prev.count), c <= UInt64(prev.count) - s else { throw NvePairProtocolError.corruptPayload("copy_run") }
                for idx in Int(s) ..< (Int(s) + Int(c)) {
                    let r = prev[idx]
                    guard out.count + r.count <= NvePairProtocol.maxDecodedFrameBytes else { throw NvePairProtocolError.frameTooLarge(declared: out.count + r.count, max: NvePairProtocol.maxDecodedFrameBytes) }
                    out.append(base[(bStart + r.lowerBound) ..< (bStart + r.upperBound)]); emitted &+= 1
                }
            case NvePairProtocol.cmdLiteralOp:
                let l = try readVarint(deltaPayload, pos: &pos)
                guard l > 0, l <= UInt64(deltaPayload.count - pos) else { throw NvePairProtocolError.truncatedFrame }
                guard out.count + Int(l) <= NvePairProtocol.maxDecodedFrameBytes else { throw NvePairProtocolError.frameTooLarge(declared: out.count + Int(l), max: NvePairProtocol.maxDecodedFrameBytes) }
                out.append(deltaPayload[(pStart + pos) ..< (pStart + pos + Int(l))]); pos += Int(l); emitted &+= 1
            case NvePairProtocol.cmdXorOp:
                let pi = try readVarint(deltaPayload, pos: &pos), xl = try readVarint(deltaPayload, pos: &pos)
                guard pi < UInt64(prev.count) else { throw NvePairProtocolError.corruptPayload("xor_op index") }
                guard xl > 0, xl <= UInt64(deltaPayload.count - pos) else { throw NvePairProtocolError.truncatedFrame }
                let r = prev[Int(pi)]
                let dec = try decodeXorRleDelta(base: base[(bStart + r.lowerBound) ..< (bStart + r.upperBound)], deltaPayload: deltaPayload[(pStart + pos) ..< (pStart + pos + Int(xl))])
                pos += Int(xl)
                guard out.count + dec.count <= NvePairProtocol.maxDecodedFrameBytes else { throw NvePairProtocolError.frameTooLarge(declared: out.count + dec.count, max: NvePairProtocol.maxDecodedFrameBytes) }
                out.append(dec); emitted &+= 1
            default: throw NvePairProtocolError.corruptPayload("unknown tag \(tag)")
            }
        }
        guard emitted == UInt32(ops) else { throw NvePairProtocolError.corruptPayload("op_count mismatch") }
        return out
    }
}

public struct NveDeltaReceiveOutcome: Sendable {
    public let appliedFrames: [(seq: UInt32, frame: Data, kind: NveDeltaPacketKind)]
    public let ackPacket: Data?, nackPackets: [Data], gapDetected: Bool
}

public struct NveDeltaReceiver: Sendable {
    public private(set) var maxWindow: Int
    public private(set) var lastAppliedSeq: UInt32 = 0
    public private(set) var lastAppliedFrame = Data()
    private var reorderBuffer: [UInt32: Data] = [:]

    public init(maxWindow: Int = NvePairProtocol.defaultSyncWindow) {
        self.maxWindow = min(max(maxWindow, NvePairProtocol.minSyncWindow), NvePairProtocol.maxSyncWindow)
    }
    public var reorderBufferCount: Int { reorderBuffer.count }

    public mutating func reset(maxWindow: Int? = nil) {
        if let w = maxWindow { self.maxWindow = min(max(w, NvePairProtocol.minSyncWindow), NvePairProtocol.maxSyncWindow) }
        lastAppliedSeq = 0; lastAppliedFrame.removeAll(keepingCapacity: false); reorderBuffer.removeAll(keepingCapacity: false)
    }

    public mutating func receivePacket(_ packetBytes: Data) throws -> NveDeltaReceiveOutcome {
        let pkt = try NveDeltaPacket.parse(packetBytes); let hdr = pkt.header
        if hdr.kind == .ack || hdr.kind == .nack { return NveDeltaReceiveOutcome(appliedFrames: [], ackPacket: nil, nackPackets: [], gapDetected: false) }
        if hdr.seq <= lastAppliedSeq { return NveDeltaReceiveOutcome(appliedFrames: [], ackPacket: buildAck(), nackPackets: [], gapDetected: false) }
        var applied: [(seq: UInt32, frame: Data, kind: NveDeltaPacketKind)] = [], nacks: [Data] = [], gap = false
        let expected = lastAppliedSeq &+ 1
        if hdr.kind == .keyframe && hdr.seq >= expected {
            let f = Data(pkt.payload); lastAppliedSeq = hdr.seq; lastAppliedFrame = f
            applied.append((hdr.seq, f, .keyframe)); reorderBuffer = reorderBuffer.filter { $0.key > lastAppliedSeq }; try drain(&applied)
        } else if hdr.seq == expected {
            let f = try applySingle(pkt); lastAppliedSeq = hdr.seq; lastAppliedFrame = f
            applied.append((hdr.seq, f, hdr.kind)); try drain(&applied)
        } else {
            gap = true; if reorderBuffer.count < maxWindow { reorderBuffer[hdr.seq] = packetBytes }
            var m = expected; let end = min(hdr.seq, expected &+ 8)
            while m < end {
                if reorderBuffer[m] == nil { var p = Data(capacity: 4); appendLE(m, to: &p); nacks.append(NveDeltaPacket.encode(kind: .nack, ackSeq: lastAppliedSeq, payload: p)) }
                m &+= 1
            }
        }
        return NveDeltaReceiveOutcome(appliedFrames: applied, ackPacket: buildAck(), nackPackets: nacks, gapDetected: gap)
    }

    private func applySingle(_ pkt: NveDeltaPacket) throws -> Data {
        switch pkt.header.kind {
        case .keyframe: return Data(pkt.payload)
        case .xorRleDelta, .commandOpDelta:
            guard pkt.header.baseSeq == lastAppliedSeq else { throw NvePairProtocolError.baseSequenceMismatch(expected: lastAppliedSeq, actual: pkt.header.baseSeq) }
            return pkt.header.kind == .xorRleDelta
                ? try NveDeltaCodec.decodeXorRleDelta(base: lastAppliedFrame, deltaPayload: pkt.payload)
                : try NveDeltaCodec.decodeCommandOpDelta(base: lastAppliedFrame, deltaPayload: pkt.payload)
        case .ack, .nack: throw NvePairProtocolError.invalidDeltaKind(pkt.header.kind.rawValue)
        }
    }

    private mutating func drain(_ applied: inout [(seq: UInt32, frame: Data, kind: NveDeltaPacketKind)]) throws {
        while let raw = reorderBuffer.removeValue(forKey: lastAppliedSeq &+ 1) {
            let pkt = try NveDeltaPacket.parse(raw), f = try applySingle(pkt)
            lastAppliedSeq = pkt.header.seq; lastAppliedFrame = f; applied.append((pkt.header.seq, f, pkt.header.kind))
        }
    }

    private func buildAck() -> Data {
        var sack: UInt64 = 0
        for seq in reorderBuffer.keys where seq > lastAppliedSeq { let d = seq - lastAppliedSeq; if d >= 1 && d <= 64 { sack |= (1 << (d - 1)) } }
        var p = Data(capacity: 8); appendLE(sack, to: &p)
        return NveDeltaPacket.encode(kind: .ack, ackSeq: lastAppliedSeq, payload: p)
    }
}

@inline(__always) private func appendLE<T: FixedWidthInteger>(_ v: T, to data: inout Data) { withUnsafeBytes(of: v.littleEndian) { data.append(contentsOf: $0) } }
@inline(__always) private func readU16LE(_ r: UnsafeRawBufferPointer, at o: Int) -> UInt16 { UInt16(littleEndian: r.loadUnaligned(fromByteOffset: o, as: UInt16.self)) }
@inline(__always) private func readU32LE(_ r: UnsafeRawBufferPointer, at o: Int) -> UInt32 { UInt32(littleEndian: r.loadUnaligned(fromByteOffset: o, as: UInt32.self)) }
@inline(__always) private func readU64LE(_ r: UnsafeRawBufferPointer, at o: Int) -> UInt64 { UInt64(littleEndian: r.loadUnaligned(fromByteOffset: o, as: UInt64.self)) }

private func readVarint(_ data: Data, pos: inout Int) throws -> UInt64 {
    var shift: UInt64 = 0, result: UInt64 = 0; let start = data.startIndex
    for _ in 0 ..< 10 {
        guard pos < data.count else { throw NvePairProtocolError.truncatedFrame }
        let b = data[start + pos]; pos += 1; let low = UInt64(b & 0x7F)
        if shift == 63 && low > 1 { throw NvePairProtocolError.corruptPayload("varint overflow") }
        result |= low << shift; if (b & 0x80) == 0 { return result }; shift += 7
    }
    throw NvePairProtocolError.corruptPayload("unterminated varint")
}
