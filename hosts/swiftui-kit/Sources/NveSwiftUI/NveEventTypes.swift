import Foundation

// MARK: - Core Event & Backend Types

/// Native event kind mirroring `NativeEventKind` in `packages/core/abi/src/events.rs`.
public enum NveEventKind: UInt8, Sendable, Hashable {
    case press = 1
    case longPress = 2
    case scroll = 3
    case textChange = 4
    case focus = 5
    case blur = 6
    case layout = 7
    case custom = 8
    case compositionStart = 9
    case compositionUpdate = 10
    case compositionEnd = 11
    case keyDown = 12
    case keyUp = 13
    case submit = 14
    case pointerDown = 15
    case pointerMove = 16
    case pointerUp = 17
    case pointerEnter = 18
    case pointerLeave = 19
    case wheel = 20
}

/// Gesture arbitration outcome mirroring `GestureDisposition` in `packages/core/interop`.
public enum NveGestureDisposition: UInt8, Sendable, Hashable {
    case claim = 1
    case passThrough = 2
    case deferToParent = 3
}

/// Zero-allocation typed event payload replacing legacy JSON-encoded event dictionaries.
public enum NveEventPayload: Sendable, Equatable {
    case empty
    case point(x: Float, y: Float)
    case scroll(x: Float, y: Float, contentWidth: Float = 0, contentHeight: Float = 0)
    case text(String)
    case layout(x: Float = 0, y: Float = 0, width: Float, height: Float)
    case binValue(Data)
    case key(key: String, code: String = "", modifiers: UInt8 = 0, keyCode: UInt32 = 0, isComposing: Bool = false)
    case pointer(x: Float, y: Float, modifiers: UInt8 = 0, flags: UInt8 = 0)
    case wheel(deltaX: Float, deltaY: Float, modifiers: UInt8 = 0)

    /// Zero-copy view over `.binValue(Data)` payloads encoded in `NveBinValue` format.
    public func withBinValueView<R>(_ body: (NveBinValueView) throws -> R) throws -> R? {
        guard case let .binValue(data) = self, !data.isEmpty else {
            return nil
        }
        return try NveBinValueView.withParsed(data, body)
    }
}

/// Decoded outbound or inbound NVE event with strongly typed payload (binary).
public struct NveEvent: Sendable, Equatable {
    public let surfaceId: UInt32
    public let nodeId: UInt32
    public let timestampUs: UInt64
    public let kind: NveEventKind
    public let name: String?
    public let payload: NveEventPayload

    public init(
        surfaceId: UInt32,
        nodeId: UInt32,
        timestampUs: UInt64,
        kind: NveEventKind,
        name: String? = nil,
        payload: NveEventPayload = .empty
    ) {
        self.surfaceId = surfaceId
        self.nodeId = nodeId
        self.timestampUs = timestampUs
        self.kind = kind
        self.name = name
        self.payload = payload
    }

    /// Wall-clock timestamp in microseconds for host-originated events.
    public static func nowUs() -> UInt64 { UInt64(Date().timeIntervalSince1970 * 1_000_000) }

    /// Borrows a zero-copy ``NveBinValueView`` if `payload` holds a valid `NveBinValue` buffer.
    public func withBinValueView<R>(_ body: (NveBinValueView) throws -> R) throws -> R? {
        try payload.withBinValueView(body)
    }
}
