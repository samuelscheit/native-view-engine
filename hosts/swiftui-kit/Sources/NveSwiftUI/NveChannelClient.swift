import Foundation

// MARK: - Shared Channel Bus Client

/// Swift client bridging `NveChannelBus` topics (`theme`, `locale`, `auth`, `navigation`, custom).
public final class NveChannelClient: @unchecked Sendable {
    public struct Message: Sendable {
        public let topic: String
        public let payload: Data
        public let seq: UInt64

        /// Zero-copy `NveBinValueView` accessor over binary-encoded channel payloads.
        public func withBinValueView<R>(_ body: (NveBinValueView) throws -> R) throws -> R {
            try NveBinValueView.withParsed(payload, body)
        }
    }

    private let lock = NSLock()
    private var seq: UInt64 = 0
    private var retained: [String: Message] = [:]
    private var handlers: [String: [UUID: (Message) -> Void]] = [:]

    public init() {}

    @discardableResult
    public func subscribe(
        topic: String,
        replayRetained: Bool = true,
        handler: @escaping (Message) -> Void
    ) -> UUID {
        let token = UUID()
        let snapshot: Message?
        lock.lock()
        handlers[topic, default: [:]][token] = handler
        snapshot = replayRetained ? retained[topic] : nil
        lock.unlock()
        if let snapshot {
            handler(snapshot)
        }
        return token
    }

    public func unsubscribe(topic: String, token: UUID) {
        lock.lock()
        handlers[topic]?.removeValue(forKey: token)
        lock.unlock()
    }

    public func publish(topic: String, payload: Data, retain: Bool = false) {
        let message: Message
        let callbacks: [(Message) -> Void]
        lock.lock()
        seq += 1
        message = Message(topic: topic, payload: payload, seq: seq)
        if retain {
            retained[topic] = message
        }
        callbacks = Array(handlers[topic]?.values ?? [:].values)
        lock.unlock()
        for callback in callbacks {
            callback(message)
        }
    }

    /// Publishes a structured binary `NveBinValue` payload to `topic` (binary).
    public func publishValue(topic: String, value: NveBinValue, retain: Bool = false) throws {
        let data = try value.encode()
        publish(topic: topic, payload: data, retain: retain)
    }
}
