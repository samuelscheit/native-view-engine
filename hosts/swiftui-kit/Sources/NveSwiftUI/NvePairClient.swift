import Combine
import Foundation

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
    public var deltasReceived: UInt64 = 0
    public var keyframeRequestsSent: UInt64 = 0
    public var eventsSent: UInt64 = 0
    public var wireBytesRx: UInt64 = 0
    public var rawBytesRx: UInt64 = 0
    public var wireBytesTx: UInt64 = 0
    public var lastRttUs: UInt64?
    public var smoothedRttUs: UInt64?
    public var minRttUs: UInt64?
    public var fps: Double = 0
    public var bytesPerSecRx: Double = 0

    public init() {}
    public var compressionRatio: Double { wireBytesRx == 0 ? 1.0 : Double(rawBytesRx) / Double(wireBytesRx) }
    public var smoothedRttMs: Double? { smoothedRttUs.map { Double($0) / 1000.0 } }
}

public protocol NvePairTransport: AnyObject {
    func connect(
        url: URL,
        onOpen: @escaping @Sendable () -> Void,
        onBinary: @escaping @Sendable (Data) -> Void,
        onClose: @escaping @Sendable (Error?) -> Void
    )
    func send(_ data: Data)
    func close()
}

/// Zero-dependency Apple Foundation `URLSessionWebSocketTask` transport.
public final class NveURLSessionWebSocketTransport: NSObject, NvePairTransport, URLSessionWebSocketDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var session: URLSession?
    private var task: URLSessionWebSocketTask?
    private var onOpen: (@Sendable () -> Void)?
    private var onBinary: (@Sendable (Data) -> Void)?
    private var onClose: (@Sendable (Error?) -> Void)?
    private var closed = false

    public override init() { super.init() }

    public func connect(
        url: URL,
        onOpen: @escaping @Sendable () -> Void,
        onBinary: @escaping @Sendable (Data) -> Void,
        onClose: @escaping @Sendable (Error?) -> Void
    ) {
        close()
        let config = URLSessionConfiguration.default
        config.waitsForConnectivity = false
        let urlSession = URLSession(configuration: config, delegate: self, delegateQueue: nil)
        let wsTask = urlSession.webSocketTask(with: url)
        wsTask.maximumMessageSize = NvePairProtocol.maxWireFrameBytes
        lock.lock()
        closed = false; session = urlSession; task = wsTask
        self.onOpen = onOpen; self.onBinary = onBinary; self.onClose = onClose
        lock.unlock()
        wsTask.resume()
        receiveNext(on: wsTask)
    }

    public func send(_ data: Data) {
        lock.lock(); let currentTask = task; lock.unlock()
        currentTask?.send(.data(data)) { _ in }
    }

    public func close() {
        lock.lock()
        closed = true
        let currentTask = task; let currentSession = session
        task = nil; session = nil; onOpen = nil; onBinary = nil; onClose = nil
        lock.unlock()
        currentTask?.cancel(with: .normalClosure, reason: nil)
        currentSession?.invalidateAndCancel()
    }

    public func urlSession(_ session: URLSession, webSocketTask: URLSessionWebSocketTask, didOpenWithProtocol protocol: String?) {
        lock.lock(); let cb = onOpen; lock.unlock(); cb?()
    }

    public func urlSession(_ session: URLSession, webSocketTask: URLSessionWebSocketTask, didCloseWith closeCode: URLSessionWebSocketTask.CloseCode, reason: Data?) {
        notifyClosed(nil)
    }

    private func receiveNext(on wsTask: URLSessionWebSocketTask) {
        wsTask.receive { [weak self] result in
            guard let self else { return }
            switch result {
            case let .success(message):
                if case let .data(bytes) = message {
                    self.lock.lock(); let cb = self.onBinary; self.lock.unlock(); cb?(bytes)
                }
                self.receiveNext(on: wsTask)
            case let .failure(err):
                self.notifyClosed(err)
            }
        }
    }

    private func notifyClosed(_ error: Error?) {
        lock.lock()
        guard !closed else { lock.unlock(); return }
        closed = true; let cb = onClose; onOpen = nil; onBinary = nil; onClose = nil
        lock.unlock()
        cb?(error)
    }
}

@MainActor
public final class NvePairClient: ObservableObject {
    @Published public private(set) var state: NvePairConnectionState = .disconnected
    @Published public private(set) var telemetry = NveSyncTelemetry()
    @Published public private(set) var latestFrameSeq: UInt32 = 0
    @Published public private(set) var latestFrame: Data?
    @Published public private(set) var latestSummary: NveCommandBufferSummary?
    @Published public private(set) var activeTarget: NvePairUri?

    public var onFrame: ((UInt32, Data) -> Void)?
    public var autoReconnect: Bool
    public var maxReconnectAttempts: Int
    public var heartbeatIntervalSeconds: TimeInterval

    private let transportFactory: () -> NvePairTransport
    private let nowMicros: () -> UInt64
    private var transport: NvePairTransport?
    private var deltaReceiver = NveDeltaReceiver()
    private var pendingClientNonce: UInt64?
    private var nonceCounter: UInt64 = 1
    private var manualDisconnect = false
    private var reconnectAttempt = 0
    private var heartbeatTask: Task<Void, Never>?
    private var reconnectTask: Task<Void, Never>?
    private var rxByteSamples: [(tsUs: UInt64, bytes: Int)] = []
    private var frameSamples: [UInt64] = []

    public init(
        transportFactory: @escaping () -> NvePairTransport = { NveURLSessionWebSocketTransport() },
        autoReconnect: Bool = true,
        maxReconnectAttempts: Int = 5,
        heartbeatIntervalSeconds: TimeInterval = 1.0,
        nowMicros: @escaping () -> UInt64 = { UInt64(DispatchTime.now().uptimeNanoseconds / 1_000) }
    ) {
        self.transportFactory = transportFactory; self.autoReconnect = autoReconnect
        self.maxReconnectAttempts = maxReconnectAttempts; self.heartbeatIntervalSeconds = heartbeatIntervalSeconds
        self.nowMicros = nowMicros
    }

    deinit { heartbeatTask?.cancel(); reconnectTask?.cancel(); transport?.close() }

    public var sessionId: UInt32 {
        if case let .paired(sid) = state { return sid }
        return 0
    }

    public func connect(uriString: String, fallbackToken: String? = nil) throws {
        connect(target: try NvePairUri.parse(uriString, fallbackToken: fallbackToken))
    }

    public func connect(target: NvePairUri) {
        cancelTimers(); transport?.close()
        manualDisconnect = false; reconnectAttempt = 0; activeTarget = target
        deltaReceiver.reset(); telemetry = NveSyncTelemetry()
        latestFrameSeq = 0; latestFrame = nil; latestSummary = nil
        rxByteSamples.removeAll(); frameSamples.removeAll()
        openTransport(target: target, isReconnect: false)
    }

    public func disconnect() {
        manualDisconnect = true; cancelTimers()
        if case let .paired(sid) = state { sendEnvelope(NvePairCodec.encodeBye(sessionId: sid)) }
        transport?.close(); transport = nil; pendingClientNonce = nil; state = .disconnected
    }

    @discardableResult
    public func requestKeyframe(reason: UInt8 = 1) -> Data? {
        guard case let .paired(sid) = state else { return nil }
        let pkt = NvePairCodec.encodeKeyframeRequest(sessionId: sid, lastSeq: deltaReceiver.lastAppliedSeq, reason: reason)
        telemetry.keyframeRequestsSent &+= 1; sendEnvelope(pkt); return pkt
    }

    @discardableResult
    public func sendPing() -> Data? {
        guard case let .paired(sid) = state else { return nil }
        let pkt = NvePairCodec.encodePing(sessionId: sid, timestampUs: nowMicros())
        sendEnvelope(pkt); return pkt
    }

    @discardableResult
    public func sendInputEvents(_ events: [NveEvent]) throws -> Data? {
        guard !events.isEmpty else { return nil }
        guard case let .paired(sid) = state else { throw NvePairProtocolError.malformedFrame("not paired") }
        let pkt = try NvePairCodec.encodeInputEvents(events, sessionId: sid)
        telemetry.eventsSent &+= 1; sendEnvelope(pkt); return pkt
    }

    @discardableResult
    public func sendRawInputEvents(_ eventRing: Data) throws -> Data {
        guard case let .paired(sid) = state else { throw NvePairProtocolError.malformedFrame("not paired") }
        let pkt = try NvePairCodec.encodeRawInputEvents(eventRing, sessionId: sid)
        telemetry.eventsSent &+= 1; sendEnvelope(pkt); return pkt
    }

    @discardableResult
    public func beginHandshake() -> Data? {
        guard let target = activeTarget else { return nil }
        let seq = nonceCounter; nonceCounter = max(nonceCounter &+ 1, 1)
        let lastSeq = deltaReceiver.lastAppliedSeq
        let clientNonce = NvePairCrypto.mixNonce(seq ^ UInt64(lastSeq) ^ 0x5357_4946_5455_4901)
        pendingClientNonce = clientNonce
        let hello = NveHelloPayload(
            schemaVersion: target.schemaVersion, maxWindow: UInt16(NvePairProtocol.defaultSyncWindow),
            clientLastSeq: lastSeq, clientNonce: clientNonce,
            tokenProof: NvePairCrypto.deriveTokenProof(pairingToken: target.token, schemaVersion: target.schemaVersion)
        )
        let env = NveTransportEnvelope.encode(kind: .hello, sessionId: 0, payload: hello.encode())
        sendEnvelope(env); return env
    }

    @discardableResult
    public func handleInboundBinary(_ bytes: Data) throws -> [Data] {
        let now = nowMicros()
        telemetry.wireBytesRx &+= UInt64(bytes.count)
        rxByteSamples.append((now, bytes.count)); pruneWindow(nowUs: now)

        let env = try NveTransportEnvelope.parse(bytes)
        var outbound: [Data] = []

        switch env.kind {
        case .helloAck:
            guard let target = activeTarget, let clientNonce = pendingClientNonce else {
                throw NvePairProtocolError.malformedFrame("unexpected HelloAck")
            }
            let ack = try NveHelloAckPayload.decode(env.payload)
            guard ack.schemaVersion == target.schemaVersion else {
                let msg = "schema mismatch: expected \(target.schemaVersion), got \(ack.schemaVersion)"
                state = .error(msg); throw NvePairProtocolError.malformedFrame(msg)
            }
            let expectedSid = NvePairCrypto.deriveSessionId(
                pairingToken: target.token, schemaVersion: target.schemaVersion,
                clientNonce: clientNonce, serverNonce: ack.serverNonce
            )
            guard env.sessionId == expectedSid else {
                let msg = String(format: "session_id mismatch: expected 0x%08X, got 0x%08X", expectedSid, env.sessionId)
                state = .error(msg); throw NvePairProtocolError.malformedFrame(msg)
            }
            pendingClientNonce = nil; reconnectAttempt = 0
            deltaReceiver.reset(maxWindow: Int(ack.serverMaxWindow))
            telemetry.sessionId = env.sessionId; state = .paired(sessionId: env.sessionId)
            startHeartbeat()

        case .reject:
            let reject = try NveRejectPayload.decode(env.payload)
            let msg = "pairing rejected: \(reject.reason) (server schema v\(reject.expectedSchema))"
            pendingClientNonce = nil; manualDisconnect = true; cancelTimers()
            state = .error(msg); throw NvePairProtocolError.malformedFrame(msg)

        case .deltaSingle:
            let sid = try requirePairedSessionId(env.sessionId)
            do {
                let outcome = try deltaReceiver.receivePacket(env.payload)
                for entry in outcome.appliedFrames {
                    telemetry.framesReceived &+= 1; telemetry.rawBytesRx &+= UInt64(entry.frame.count)
                    if entry.kind == .keyframe { telemetry.keyframesReceived &+= 1 } else { telemetry.deltasReceived &+= 1 }
                    frameSamples.append(now)
                    latestFrameSeq = entry.seq; latestFrame = entry.frame
                    latestSummary = NveDeltaCodec.scanNveCommandSlices(entry.frame)?.0
                    onFrame?(entry.seq, entry.frame)
                }
                if let ack = outcome.ackPacket { outbound.append(NveTransportEnvelope.encode(kind: .deltaSingle, sessionId: sid, payload: ack)) }
                for nack in outcome.nackPackets { outbound.append(NveTransportEnvelope.encode(kind: .deltaSingle, sessionId: sid, payload: nack)) }
                if outcome.gapDetected {
                    telemetry.keyframeRequestsSent &+= 1
                    outbound.append(NvePairCodec.encodeKeyframeRequest(sessionId: sid, lastSeq: deltaReceiver.lastAppliedSeq, reason: 2))
                }
            } catch NvePairProtocolError.baseSequenceMismatch {
                telemetry.keyframeRequestsSent &+= 1
                outbound.append(NvePairCodec.encodeKeyframeRequest(sessionId: sid, lastSeq: deltaReceiver.lastAppliedSeq, reason: 3))
            }

        case .ping:
            let sid = try requirePairedSessionId(env.sessionId)
            outbound.append(NvePairCodec.encodePong(sessionId: sid, timestampUs: try NvePairCodec.decodeTimestampUs(env.payload)))

        case .pong:
            _ = try requirePairedSessionId(env.sessionId)
            let sentUs = try NvePairCodec.decodeTimestampUs(env.payload)
            if now >= sentUs {
                let rtt = now - sentUs
                telemetry.lastRttUs = rtt
                telemetry.minRttUs = telemetry.minRttUs.map { min($0, rtt) } ?? rtt
                telemetry.smoothedRttUs = telemetry.smoothedRttUs.map { ($0 * 7 + rtt) / 8 } ?? rtt
            }

        case .bye:
            _ = try requirePairedSessionId(env.sessionId)
            manualDisconnect = true; cancelTimers(); transport?.close(); transport = nil; state = .disconnected

        default:
            throw NvePairProtocolError.malformedFrame("unsupported inbound kind \(env.kind)")
        }

        for msg in outbound { sendEnvelope(msg) }
        refreshRates(nowUs: now)
        return outbound
    }

    private func openTransport(target: NvePairUri, isReconnect: Bool) {
        state = isReconnect ? .reconnecting(attempt: reconnectAttempt) : .connecting
        let next = transportFactory(); transport = next
        next.connect(
            url: target.wsUrl,
            onOpen: { [weak self] in Task { @MainActor [weak self] in self?.beginHandshake() } },
            onBinary: { [weak self] data in Task { @MainActor [weak self] in _ = try? self?.handleInboundBinary(data) } },
            onClose: { [weak self] err in Task { @MainActor [weak self] in self?.handleTransportClosed(err) } }
        )
    }

    private func handleTransportClosed(_ error: Error?) {
        heartbeatTask?.cancel(); heartbeatTask = nil
        if manualDisconnect { return }
        guard autoReconnect, let target = activeTarget, reconnectAttempt < maxReconnectAttempts else {
            state = .error(error?.localizedDescription ?? "connection closed"); return
        }
        reconnectAttempt += 1; let attempt = reconnectAttempt; state = .reconnecting(attempt: attempt)
        reconnectTask?.cancel()
        reconnectTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(min(attempt, 5)) * 400_000_000)
            guard let self, !Task.isCancelled, !self.manualDisconnect else { return }
            self.openTransport(target: target, isReconnect: true)
        }
    }

    private func startHeartbeat() {
        heartbeatTask?.cancel()
        guard heartbeatIntervalSeconds > 0 else { return }
        let ns = UInt64(max(heartbeatIntervalSeconds, 0.05) * 1_000_000_000)
        heartbeatTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: ns)
                guard let self, !Task.isCancelled, self.state.isPaired else { break }
                self.sendPing()
            }
        }
    }

    private func cancelTimers() { heartbeatTask?.cancel(); heartbeatTask = nil; reconnectTask?.cancel(); reconnectTask = nil }
    private func sendEnvelope(_ data: Data) { telemetry.wireBytesTx &+= UInt64(data.count); transport?.send(data) }

    private func requirePairedSessionId(_ incoming: UInt32) throws -> UInt32 {
        guard case let .paired(expected) = state else { throw NvePairProtocolError.malformedFrame("not paired") }
        guard incoming == expected else { throw NvePairProtocolError.malformedFrame("session_id mismatch") }
        return expected
    }

    private func pruneWindow(nowUs: UInt64) {
        let cutoff = nowUs > 1_000_000 ? nowUs - 1_000_000 : 0
        rxByteSamples.removeAll { $0.tsUs < cutoff }; frameSamples.removeAll { $0 < cutoff }
    }

    private func refreshRates(nowUs: UInt64) {
        pruneWindow(nowUs: nowUs)
        telemetry.bytesPerSecRx = Double(rxByteSamples.reduce(0) { $0 + $1.bytes })
        telemetry.fps = Double(frameSamples.count)
    }
}
