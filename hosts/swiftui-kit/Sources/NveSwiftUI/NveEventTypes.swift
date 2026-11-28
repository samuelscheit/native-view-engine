import CNveAbi
import Foundation
import NveHost
import NveHostAbi

// MARK: - Core Event & Gesture Types

/// High-level SwiftUI event kind bridged over `NveHost` uniform 64-byte `NveEvent` batches.
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
    case pointerCancel = 21
}

/// Gesture arbitration outcome mirroring `GestureDisposition` in `packages/core/interop`.
public enum NveGestureDisposition: UInt8, Sendable, Hashable {
    case claim = 1
    case passThrough = 2
    case deferToParent = 3
}

/// Typed event payload carried by ``NveEvent`` (binary, zero JSON).
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

// MARK: - `NveHost.EventBatchBuilder` / `EventBatchReader` Bridge

public extension EventBatchBuilder {
    /// Builds a reusable uniform 64-byte event batch from Swift ``NveEvent`` values.
    init(events: [NveEvent]) {
        self.init(eventCapacity: events.count)
        for event in events {
            push(event)
        }
    }

    /// Appends a high-level ``NveEvent`` using `NveHost`'s typed `EventBatchBuilder` encoders.
    mutating func push(_ event: NveEvent) {
        let s = event.surfaceId
        let n = event.nodeId
        let ts = event.timestampUs
        switch event.kind {
        case .press:
            let (x, y) = pointPayload(event.payload)
            pushPress(surface: s, node: n, timestamp: ts, x: x, y: y)
        case .longPress:
            let (x, y) = pointPayload(event.payload)
            pushLongPress(surface: s, node: n, timestamp: ts, x: x, y: y)
        case .focus:
            pushFocus(surface: s, node: n, timestamp: ts)
        case .blur:
            pushBlur(surface: s, node: n, timestamp: ts)
        case .keyDown, .keyUp:
            let id: InteractionEventId = event.kind == .keyDown ? .keyDown : .keyUp
            if case let .key(key, code, modifiers, keyCode, isComposing) = event.payload {
                var flags = EventFlags(rawValue: UInt32(modifiers) & EventFlags.modMask.rawValue)
                if isComposing { flags.insert(.composing) }
                pushKey(surface: s, node: n, timestamp: ts, event: id, key: key, code: code, flags: flags, keyCode: keyCode)
            } else {
                pushKey(surface: s, node: n, timestamp: ts, event: id, key: "")
            }
        case .pointerDown, .pointerMove, .pointerUp, .pointerEnter, .pointerLeave, .pointerCancel:
            let id: InteractionEventId
            switch event.kind {
            case .pointerDown: id = .pointerDown
            case .pointerMove: id = .pointerMove
            case .pointerUp: id = .pointerUp
            case .pointerEnter: id = .pointerEnter
            case .pointerCancel: id = .pointerCancel
            default: id = .pointerLeave
            }
            if case let .pointer(x, y, modifiers, pflags) = event.payload {
                var flags = EventFlags(rawValue: UInt32(modifiers) & EventFlags.modMask.rawValue)
                    .withPointerButton(pflags & 0x07)
                if (pflags & 0x08) != 0 { flags.insert(.capture) }
                pushPointer(surface: s, node: n, timestamp: ts, event: id, x: x, y: y, flags: flags)
            } else {
                pushPointer(surface: s, node: n, timestamp: ts, event: id, x: 0, y: 0)
            }
        case .wheel:
            if case let .wheel(dx, dy, modifiers) = event.payload {
                let flags = EventFlags(rawValue: UInt32(modifiers) & EventFlags.modMask.rawValue)
                pushWheel(surface: s, node: n, timestamp: ts, deltaX: dx, deltaY: dy, flags: flags)
            } else {
                pushWheel(surface: s, node: n, timestamp: ts, deltaX: 0, deltaY: 0)
            }
        case .textChange:
            pushTextChange(surface: s, node: n, timestamp: ts, text: textPayload(event.payload))
        case .submit:
            pushSubmit(surface: s, node: n, timestamp: ts, text: textPayload(event.payload))
        case .compositionStart:
            pushComposition(surface: s, node: n, timestamp: ts, event: .compositionStart, text: textPayload(event.payload))
        case .compositionUpdate:
            pushComposition(surface: s, node: n, timestamp: ts, event: .compositionUpdate, text: textPayload(event.payload))
        case .compositionEnd:
            pushComposition(surface: s, node: n, timestamp: ts, event: .compositionEnd, text: textPayload(event.payload))
        case .scroll:
            if case let .scroll(x, y, cw, ch) = event.payload {
                pushScroll(surface: s, node: n, timestamp: ts, x: x, y: y, contentWidth: cw, contentHeight: ch)
            } else {
                pushScroll(surface: s, node: n, timestamp: ts, x: 0, y: 0)
            }
        case .layout:
            if case let .layout(x, y, w, h) = event.payload {
                pushLayout(surface: s, node: n, timestamp: ts, x: x, y: y, width: w, height: h)
            } else {
                pushLayout(surface: s, node: n, timestamp: ts, x: 0, y: 0, width: 0, height: 0)
            }
        case .custom:
            let data: Data
            if case let .binValue(d) = event.payload { data = d } else { data = Data() }
            pushCustom(surface: s, node: n, timestamp: ts, name: event.name ?? "", data: data)
        }
    }

    /// Encodes a structured ``NveBinValue`` payload and pushes a named custom event.
    mutating func pushCustomEvent(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        timestampUs: UInt64 = NveEvent.nowUs(),
        name: String,
        value: NveBinValue
    ) throws {
        let encoded = try value.encode()
        try encoded.withUnsafeBytes { raw in
            try tryPushCustom(surface: surfaceId, node: nodeId, timestamp: timestampUs, name: name, data: raw)
        }
    }

    private func pointPayload(_ payload: NveEventPayload) -> (Float, Float) {
        if case let .point(x, y) = payload { return (x, y) }
        return (0, 0)
    }

    private func textPayload(_ payload: NveEventPayload) -> String {
        if case let .text(text) = payload { return text }
        return ""
    }
}

public extension EventBatchReader {
    /// Decodes a single validated 64-byte record into a high-level ``NveEvent`` (`nil` for unknown tables).
    func decodeEvent(at index: Int) -> NveEvent? {
        guard let rec = record(at: index) else { return nil }
        if rec.table.value == nveTableCustom {
            let name = rec.flags.contains(.customNamed) ? customName(at: index) : ""
            return NveEvent(
                surfaceId: rec.surface,
                nodeId: rec.node,
                timestampUs: rec.timestamp,
                kind: .custom,
                name: name,
                payload: .binValue(Data(customData(at: index)))
            )
        }
        if let id = rec.interaction {
            let kind: NveEventKind
            let payload: NveEventPayload
            switch id {
            case .press:
                kind = .press
                payload = .point(x: rec.x, y: rec.y)
            case .longPress:
                kind = .longPress
                payload = .point(x: rec.x, y: rec.y)
            case .focus:
                kind = .focus
                payload = .empty
            case .blur:
                kind = .blur
                payload = .empty
            case .keyDown, .keyUp:
                kind = (id == .keyDown) ? .keyDown : .keyUp
                payload = .key(
                    key: key(at: index),
                    code: code(at: index),
                    modifiers: UInt8(rec.flags.rawValue & EventFlags.modMask.rawValue),
                    keyCode: rec.keyCode,
                    isComposing: rec.flags.isComposing
                )
            case .pointerDown, .pointerMove, .pointerUp, .pointerEnter, .pointerLeave, .pointerCancel:
                switch id {
                case .pointerDown: kind = .pointerDown
                case .pointerMove: kind = .pointerMove
                case .pointerUp: kind = .pointerUp
                case .pointerEnter: kind = .pointerEnter
                case .pointerCancel: kind = .pointerCancel
                default: kind = .pointerLeave
                }
                let pflags = rec.flags.pointerButton | (rec.flags.isCapture ? 0x08 : 0)
                payload = .pointer(
                    x: rec.x,
                    y: rec.y,
                    modifiers: UInt8(rec.flags.rawValue & EventFlags.modMask.rawValue),
                    flags: pflags
                )
            case .wheel:
                kind = .wheel
                payload = .wheel(
                    deltaX: rec.deltaX,
                    deltaY: rec.deltaY,
                    modifiers: UInt8(rec.flags.rawValue & EventFlags.modMask.rawValue)
                )
            }
            return NveEvent(surfaceId: rec.surface, nodeId: rec.node, timestampUs: rec.timestamp, kind: kind, payload: payload)
        }
        if let id = rec.textInput {
            let kind: NveEventKind
            switch id {
            case .change: kind = .textChange
            case .submit: kind = .submit
            case .compositionStart: kind = .compositionStart
            case .compositionUpdate: kind = .compositionUpdate
            case .compositionEnd: kind = .compositionEnd
            }
            return NveEvent(
                surfaceId: rec.surface,
                nodeId: rec.node,
                timestampUs: rec.timestamp,
                kind: kind,
                payload: .text(text(at: index))
            )
        }
        if rec.scrollEvent == .scroll {
            return NveEvent(
                surfaceId: rec.surface,
                nodeId: rec.node,
                timestampUs: rec.timestamp,
                kind: .scroll,
                payload: .scroll(x: rec.x, y: rec.y, contentWidth: rec.width, contentHeight: rec.height)
            )
        }
        if rec.layoutEvent == .layout {
            return NveEvent(
                surfaceId: rec.surface,
                nodeId: rec.node,
                timestampUs: rec.timestamp,
                kind: .layout,
                payload: .layout(x: rec.x, y: rec.y, width: rec.width, height: rec.height)
            )
        }
        return nil
    }

    /// Decodes all recognized events in this validated batch into ``NveEvent`` values.
    func decodeEvents() -> [NveEvent] {
        var events: [NveEvent] = []
        events.reserveCapacity(count)
        for i in 0 ..< count {
            if let ev = decodeEvent(at: i) {
                events.append(ev)
            }
        }
        return events
    }
}
