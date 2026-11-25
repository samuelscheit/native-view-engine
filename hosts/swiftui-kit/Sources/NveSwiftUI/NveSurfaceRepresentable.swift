import CNveAbi
import Darwin
import Foundation
import SwiftUI

#if canImport(UIKit)
import UIKit
public typealias NvePlatformView = UIView
#elseif canImport(AppKit)
import AppKit
public typealias NvePlatformView = NSView
#endif

// MARK: - Core Types

/// Rendering backend used for an embedded NVE surface inside SwiftUI.
public enum NveRenderBackend: String, Sendable, Hashable {
    case fabric
    case skia
}

/// Native event kind mirroring `NativeEventKind` in `crates/nve-abi/src/events.rs`.
public enum NveEventKind: UInt8, Sendable, Hashable {
    case press = 1
    case longPress = 2
    case scroll = 3
    case textChange = 4
    case focus = 5
    case blur = 6
    case layout = 7
    case custom = 8
}

/// Gesture arbitration outcome mirroring `GestureDisposition` in `crates/nve-interop`.
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

    /// Zero-copy view over `.binValue(Data)` payloads encoded in `NveBinValue` format.
    public func withBinValueView<R>(_ body: (NveBinValueView) throws -> R) throws -> R? {
        guard case let .binValue(data) = self, !data.isEmpty else {
            return nil
        }
        return try NveBinValueView.withParsed(data, body)
    }
}

/// Decoded outbound or inbound NVE event with strongly typed payload (zero JSON).
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

    /// Borrows a zero-copy ``NveBinValueView`` if `payload` holds a valid `NveBinValue` buffer.
    public func withBinValueView<R>(_ body: (NveBinValueView) throws -> R) throws -> R? {
        try payload.withBinValueView(body)
    }
}

/// Strict validation errors thrown by ``NveEventRingCodec/decodeStrict(_:)``.
public enum NveEventRingError: Error, Equatable, Sendable {
    case truncated
    case invalidEventKind(UInt8)
    case invalidPayloadTag(UInt8)
    case invalidUtf8
    case trailingBytes(Int)
}

// MARK: - Binary EventRing Codec (matches `crates/nve-abi/src/events.rs`, Zero JSON)

public enum NveEventRingCodec {
    /// Strictly decodes a binary `EventRing` buffer, throwing on truncation, unknown enums,
    /// invalid UTF-8, or trailing garbage bytes.
    public static func decodeStrict(_ data: Data) throws -> [NveEvent] {
        guard data.count >= 4 else {
            throw NveEventRingError.truncated
        }
        return try data.withUnsafeBytes { raw in
            var pos = 0

            func readU8() throws -> UInt8 {
                guard pos < raw.count else { throw NveEventRingError.truncated }
                let value = raw[pos]
                pos += 1
                return value
            }

            func readU16() throws -> UInt16 {
                guard pos + 2 <= raw.count else { throw NveEventRingError.truncated }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt16.self)
                pos += 2
                return UInt16(littleEndian: value)
            }

            func readU32() throws -> UInt32 {
                guard pos + 4 <= raw.count else { throw NveEventRingError.truncated }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt32.self)
                pos += 4
                return UInt32(littleEndian: value)
            }

            func readU64() throws -> UInt64 {
                guard pos + 8 <= raw.count else { throw NveEventRingError.truncated }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt64.self)
                pos += 8
                return UInt64(littleEndian: value)
            }

            func readF32() throws -> Float {
                Float(bitPattern: try readU32())
            }

            func readUtf8(length: Int) throws -> String {
                guard length >= 0, length <= raw.count - pos else {
                    throw NveEventRingError.truncated
                }
                let slice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + length)])
                pos += length
                guard let str = String(bytes: slice, encoding: .utf8) else {
                    throw NveEventRingError.invalidUtf8
                }
                return str
            }

            let count = Int(try readU32())
            // Each event header is at least 18 bytes; reject impossible counts immediately.
            guard count <= (raw.count - pos) / 18 else {
                throw NveEventRingError.truncated
            }

            var events: [NveEvent] = []
            events.reserveCapacity(min(count, 4096))

            for _ in 0 ..< count {
                let surfaceId = try readU32()
                let nodeId = try readU32()
                let timestampUs = try readU64()
                let kindByte = try readU8()
                guard let kind = NveEventKind(rawValue: kindByte) else {
                    throw NveEventRingError.invalidEventKind(kindByte)
                }
                let payloadTag = try readU8()

                var name: String?
                let payload: NveEventPayload

                switch payloadTag {
                case 0: // Empty
                    payload = .empty
                case 1: // Point
                    let x = try readF32()
                    let y = try readF32()
                    payload = .point(x: x, y: y)
                case 2: // Scroll
                    let x = try readF32()
                    let y = try readF32()
                    let cw = try readF32()
                    let ch = try readF32()
                    payload = .scroll(x: x, y: y, contentWidth: cw, contentHeight: ch)
                case 3: // Text
                    let len = Int(try readU32())
                    payload = .text(try readUtf8(length: len))
                case 4: // Layout
                    let x = try readF32()
                    let y = try readF32()
                    let w = try readF32()
                    let h = try readF32()
                    payload = .layout(x: x, y: y, width: w, height: h)
                case 5: // Custom
                    let nameLen = Int(try readU16())
                    name = try readUtf8(length: nameLen)
                    let dataLen = Int(try readU32())
                    guard dataLen <= raw.count - pos else {
                        throw NveEventRingError.truncated
                    }
                    let slice = Data(raw[pos ..< (pos + dataLen)])
                    pos += dataLen
                    payload = .binValue(slice)
                default:
                    throw NveEventRingError.invalidPayloadTag(payloadTag)
                }

                events.append(
                    NveEvent(
                        surfaceId: surfaceId,
                        nodeId: nodeId,
                        timestampUs: timestampUs,
                        kind: kind,
                        name: name,
                        payload: payload
                    )
                )
            }

            if pos != raw.count {
                throw NveEventRingError.trailingBytes(raw.count - pos)
            }
            return events
        }
    }

    /// Best-effort decoder that returns all valid events decoded before any truncation or corruption.
    public static func decode(_ data: Data) -> [NveEvent] {
        guard data.count >= 4 else { return [] }
        return data.withUnsafeBytes { raw in
            var pos = 0

            func readU8() -> UInt8? {
                guard pos < raw.count else { return nil }
                let value = raw[pos]
                pos += 1
                return value
            }

            func readU16() -> UInt16? {
                guard pos + 2 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt16.self)
                pos += 2
                return UInt16(littleEndian: value)
            }

            func readU32() -> UInt32? {
                guard pos + 4 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt32.self)
                pos += 4
                return UInt32(littleEndian: value)
            }

            func readU64() -> UInt64? {
                guard pos + 8 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt64.self)
                pos += 8
                return UInt64(littleEndian: value)
            }

            func readF32() -> Float? {
                readU32().map(Float.init(bitPattern:))
            }

            guard let count = readU32() else { return [] }
            var events: [NveEvent] = []
            events.reserveCapacity(Int(min(count, 4096)))

            for _ in 0 ..< count {
                guard
                    let surfaceId = readU32(),
                    let nodeId = readU32(),
                    let timestampUs = readU64(),
                    let kindByte = readU8(),
                    let payloadTag = readU8()
                else {
                    break
                }

                let kind = NveEventKind(rawValue: kindByte) ?? .custom
                var name: String?
                let payload: NveEventPayload

                switch payloadTag {
                case 0:
                    payload = .empty
                case 1:
                    guard let x = readF32(), let y = readF32() else { return events }
                    payload = .point(x: x, y: y)
                case 2:
                    guard let x = readF32(), let y = readF32(), let cw = readF32(), let ch = readF32() else {
                        return events
                    }
                    payload = .scroll(x: x, y: y, contentWidth: cw, contentHeight: ch)
                case 3:
                    guard let len = readU32(), Int(len) <= raw.count - pos else { return events }
                    let slice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + Int(len))])
                    pos += Int(len)
                    guard let text = String(bytes: slice, encoding: .utf8) else { return events }
                    payload = .text(text)
                case 4:
                    guard let x = readF32(), let y = readF32(), let w = readF32(), let h = readF32() else {
                        return events
                    }
                    payload = .layout(x: x, y: y, width: w, height: h)
                case 5:
                    guard let nameLen = readU16(), Int(nameLen) <= raw.count - pos else { return events }
                    let nameSlice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + Int(nameLen))])
                    pos += Int(nameLen)
                    guard let decodedName = String(bytes: nameSlice, encoding: .utf8) else {
                        return events
                    }
                    name = decodedName
                    guard let dataLen = readU32(), Int(dataLen) <= raw.count - pos else { return events }
                    payload = .binValue(Data(raw[pos ..< (pos + Int(dataLen))]))
                    pos += Int(dataLen)
                default:
                    return events
                }

                events.append(
                    NveEvent(
                        surfaceId: surfaceId,
                        nodeId: nodeId,
                        timestampUs: timestampUs,
                        kind: kind,
                        name: name,
                        payload: payload
                    )
                )
            }
            return events
        }
    }

    private static func appendLE<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { raw in
            data.append(contentsOf: raw)
        }
    }

    private static func appendF32(_ value: Float, to data: inout Data) {
        appendLE(value.bitPattern, to: &data)
    }

    /// Encodes a batch of typed events into the binary `EventRing` wire format.
    public static func encode(_ events: [NveEvent]) -> Data {
        var out = Data()
        out.reserveCapacity(4 + events.count * 32)
        appendLE(UInt32(events.count), to: &out)

        for event in events {
            appendLE(event.surfaceId, to: &out)
            appendLE(event.nodeId, to: &out)
            appendLE(event.timestampUs, to: &out)
            out.append(event.kind.rawValue)

            switch event.payload {
            case .empty:
                out.append(0)
            case let .point(x, y):
                out.append(1)
                appendF32(x, to: &out)
                appendF32(y, to: &out)
            case let .scroll(x, y, contentWidth, contentHeight):
                out.append(2)
                appendF32(x, to: &out)
                appendF32(y, to: &out)
                appendF32(contentWidth, to: &out)
                appendF32(contentHeight, to: &out)
            case let .text(str):
                out.append(3)
                let bytes = Data(str.utf8)
                appendLE(UInt32(bytes.count), to: &out)
                out.append(bytes)
            case let .layout(x, y, width, height):
                out.append(4)
                appendF32(x, to: &out)
                appendF32(y, to: &out)
                appendF32(width, to: &out)
                appendF32(height, to: &out)
            case let .binValue(payload):
                out.append(5)
                let nameBytes = Data((event.name ?? "").utf8)
                appendLE(UInt16(nameBytes.count), to: &out)
                out.append(nameBytes)
                appendLE(UInt32(payload.count), to: &out)
                out.append(payload)
            }
        }
        return out
    }

    public static func encodeCustomEvent(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        name: String,
        payload: Data,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) -> Data {
        encode([
            NveEvent(
                surfaceId: surfaceId,
                nodeId: nodeId,
                timestampUs: timestampUs,
                kind: .custom,
                name: name,
                payload: .binValue(payload)
            ),
        ])
    }

    public static func encodeCustomEvent(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        name: String,
        value: NveBinValue,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) throws -> Data {
        let encodedValue = try value.encode()
        return encodeCustomEvent(
            surfaceId: surfaceId,
            nodeId: nodeId,
            name: name,
            payload: encodedValue,
            timestampUs: timestampUs
        )
    }
}

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

    /// Publishes a structured binary `NveBinValue` payload to `topic` (zero JSON).
    public func publishValue(topic: String, value: NveBinValue, retain: Bool = false) throws {
        let data = try value.encode()
        publish(topic: topic, payload: data, retain: retain)
    }
}

// MARK: - Surface Controller (`nve_get_interface` Lifecycle + Direct FFI + Layout + Events)

/// Manages an instantiated `nve_get_interface` surface, zero-JSON direct props propagation,
/// direct event/command dispatch, intrinsic layout negotiation, and outbound event draining.
public final class NveSurfaceController: ObservableObject {
    private static var nextSurfaceId: UInt32 = 1
    private static let idLock = NSLock()

    public let surfaceId: UInt32
    public let vtableSymbol: String
    public let backend: NveRenderBackend

    private var module: NveModule = NveModule(surface: nil, layout: nil, component: nil, dev: nil)
    private var handle: NveAppHandle?
    private var dylibHandle: UnsafeMutableRawPointer?
    private var frameCounter: UInt64 = 0
    private var lastPropsValue: NveBinValue?
    private var fallbackIntrinsicSize: CGSize = CGSize(width: 240, height: 120)

    public var onEvent: ((NveEvent) -> Void)?

    public init(
        surfaceId: UInt32? = nil,
        vtableSymbol: String = "nve_get_interface",
        backend: NveRenderBackend = .fabric,
        getInterface: NveGetInterfaceFn? = nil,
        dylibPath: String? = nil
    ) {
        if let surfaceId {
            self.surfaceId = surfaceId
        } else {
            Self.idLock.lock()
            let id = Self.nextSurfaceId
            Self.nextSurfaceId &+= 1
            Self.idLock.unlock()
            self.surfaceId = id
        }
        self.vtableSymbol = vtableSymbol
        self.backend = backend

        if let getInterface {
            attach(getInterface: getInterface)
        } else {
            resolveAndAttach(symbol: vtableSymbol, dylibPath: dylibPath)
        }
    }

    deinit {
        destroy()
    }

    public var isMounted: Bool {
        handle != nil && module.surface != nil
    }

    private func resolveAndAttach(symbol: String, dylibPath: String?) {
        let lib: UnsafeMutableRawPointer?
        if let dylibPath, !dylibPath.isEmpty {
            lib = dlopen(dylibPath, RTLD_NOW | RTLD_LOCAL)
            dylibHandle = lib
        } else {
            lib = dlopen(nil, RTLD_NOW)
        }
        guard let lib, let sym = dlsym(lib, symbol) else {
            return
        }
        let getIface = unsafeBitCast(sym, to: NveGetInterfaceFn.self)
        attach(getInterface: getIface)
    }

    public func attach(getInterface: NveGetInterfaceFn) {
        destroy()
        let resolved = nve_resolve_module(getInterface)
        guard let surface = resolved.surface, let createFn = surface.pointee.create else { return }
        guard let created = createFn(surfaceId) else { return }
        self.module = resolved
        self.handle = created
        self.lastPropsValue = nil
    }

    /// Pushes typed props via `NveComponentApi.set_props_direct` (zero JSON), falling back to
    /// compact binary `NveBinValue` on `set_props` only if `set_props_direct == nil`.
    @discardableResult
    public func setProps(_ entries: [(String, NveBinValue)]) -> Bool {
        let canonical = NveBinValue.map(entries)
        if lastPropsValue == canonical {
            return false
        }
        guard let handle, let component = module.component?.pointee else {
            return false
        }

        let needsRender: UInt32
        if let directFn = component.set_props_direct {
            guard let status = try? NveDirectPropsWriter.withEntries(entries, { ptr, count in
                directFn(handle, ptr, count)
            }) else {
                return false
            }
            needsRender = status
        } else if let binFn = component.set_props {
            guard let binData = try? canonical.encode() else {
                return false
            }
            needsRender = binData.withUnsafeBytes { raw -> UInt32 in
                let slice = NveBufferSlice(
                    ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                    len: raw.count
                )
                return binFn(handle, slice)
            }
        } else {
            return false
        }

        lastPropsValue = canonical
        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
        return needsRender != 0
    }

    /// Pushes `[String: Any]` props into the mounted component via zero-JSON direct C-ABI structs.
    @discardableResult
    public func setProps(_ props: [String: Any]) -> Bool {
        guard let entries = try? NveBinValue.canonicalEntries(from: props) else {
            return false
        }
        return setProps(entries)
    }

    /// Dispatches a batch of typed events via `NveSurfaceApi.dispatch_events_direct` when
    /// available, falling back to binary `EventRing` on `dispatch_events`.
    @discardableResult
    public func dispatchEvents(_ events: [NveEvent]) -> Bool {
        guard !events.isEmpty, let handle, let surface = module.surface?.pointee else {
            return false
        }
        let needsRender: UInt32
        if let directFn = surface.dispatch_events_direct {
            guard let status = try? NveDirectEventWriter.withBatch(events, { batch in
                directFn(handle, batch)
            }) else {
                return false
            }
            needsRender = status
        } else if let ringFn = surface.dispatch_events {
            let encoded = NveEventRingCodec.encode(events)
            needsRender = encoded.withUnsafeBytes { raw -> UInt32 in
                let slice = NveBufferSlice(
                    ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                    len: raw.count
                )
                return ringFn(handle, slice)
            }
        } else {
            return false
        }

        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
        return needsRender != 0
    }

    /// Dispatches a single typed event.
    @discardableResult
    public func dispatchEvent(_ event: NveEvent) -> Bool {
        dispatchEvents([event])
    }

    /// Dispatches an imperative command with a typed `NveBinValue` argument directly over the C ABI.
    public func dispatchCommand(
        name: String,
        value: NveBinValue,
        nodeId: UInt32 = 1,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) {
        guard let handle, let surface = module.surface?.pointee else { return }
        let needsRender: UInt32
        if let directFn = surface.dispatch_events_direct {
            guard let status = try? NveDirectEventWriter.withCustomEvent(
                surfaceId: surfaceId,
                nodeId: nodeId,
                timestampUs: timestampUs,
                name: name,
                value: value,
                { batch in directFn(handle, batch) }
            ) else {
                return
            }
            needsRender = status
        } else if let ringFn = surface.dispatch_events {
            guard let encoded = try? NveEventRingCodec.encodeCustomEvent(
                surfaceId: surfaceId,
                nodeId: nodeId,
                name: name,
                value: value,
                timestampUs: timestampUs
            ) else {
                return
            }
            needsRender = encoded.withUnsafeBytes { raw -> UInt32 in
                let slice = NveBufferSlice(
                    ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                    len: raw.count
                )
                return ringFn(handle, slice)
            }
        } else {
            return
        }

        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
    }

    /// Dispatches an imperative command with `[String: Any]` arguments (converted directly to
    /// `NveValue` / `NveBinValue`, zero JSON).
    public func dispatchCommand(name: String, args: [String: Any] = [:]) {
        guard let entries = try? NveBinValue.canonicalEntries(from: args) else { return }
        dispatchCommand(name: name, value: .map(entries))
    }

    /// Triggers a single frame render and returns the raw CommandBuffer bytes.
    @discardableResult
    public func renderFrame() -> Data {
        guard let handle, let renderFn = module.surface?.pointee.render else { return Data() }
        frameCounter += 1
        let slice = renderFn(handle, frameCounter)
        guard let ptr = slice.ptr, slice.len > 0 else { return Data() }
        return Data(bytes: ptr, count: slice.len)
    }

    /// Applies a Tier-1 RSX hot-reload patch via `NveDevApi.apply_hot_patch`.
    @discardableResult
    public func applyHotPatch(json: String) -> Bool {
        guard let handle, let patchFn = module.dev?.pointee.apply_hot_patch else { return false }
        let data = Data(json.utf8)
        let dirty = data.withUnsafeBytes { raw -> UInt32 in
            let slice = NveBufferSlice(
                ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                len: raw.count
            )
            return patchFn(handle, slice)
        }
        if dirty != 0 {
            renderFrame()
        }
        return dirty != 0
    }

    /// Drains all queued outbound events via `NveComponentApi.drain_emitted_events`.
    public func drainEmittedEvents() -> [NveEvent] {
        guard let handle, let drainFn = module.component?.pointee.drain_emitted_events else { return [] }
        let slice = drainFn(handle)
        guard let ptr = slice.ptr, slice.len > 0 else { return [] }
        return NveEventRingCodec.decode(Data(bytes: ptr, count: slice.len))
    }

    public func drainAndForwardEvents() {
        guard let onEvent else { return }
        for event in drainEmittedEvents() {
            onEvent(event)
        }
    }

    /// Negotiates intrinsic size for SwiftUI's `sizeThatFits` using zero-cost `NveLayoutApi.measure`.
    public func measure(
        minWidth: CGFloat = 0,
        minHeight: CGFloat = 0,
        maxWidth: CGFloat = .infinity,
        maxHeight: CGFloat = .infinity
    ) -> CGSize {
        if let handle, let measureFn = module.layout?.pointee.measure {
            let constraints = NveSizeConstraints(
                min_w: Float(minWidth),
                min_h: Float(minHeight),
                max_w: maxWidth.isFinite ? Float(maxWidth) : .infinity,
                max_h: maxHeight.isFinite ? Float(maxHeight) : .infinity
            )
            let measured = measureFn(handle, constraints)
            let clampedW = max(minWidth, min(maxWidth, CGFloat(measured.width)))
            let clampedH = max(minHeight, min(maxHeight, CGFloat(measured.height)))
            return CGSize(width: clampedW, height: clampedH)
        }
        let width = maxWidth.isFinite ? max(minWidth, maxWidth) : max(minWidth, fallbackIntrinsicSize.width)
        let height = maxHeight.isFinite ? max(minHeight, maxHeight) : max(minHeight, fallbackIntrinsicSize.height)
        return CGSize(width: width, height: height)
    }

    public func setFallbackIntrinsicSize(_ size: CGSize) {
        fallbackIntrinsicSize = size
    }

    public func destroy() {
        if let handle, let destroyFn = module.surface?.pointee.destroy {
            destroyFn(handle)
        }
        handle = nil
        lastPropsValue = nil
        module = NveModule(surface: nil, layout: nil, component: nil, dev: nil)
        if let dylibHandle {
            dlclose(dylibHandle)
            self.dylibHandle = nil
        }
    }
}

// MARK: - Native Host Container View

public final class NveHostContainerView: NvePlatformView {
    public var controller: NveSurfaceController?

    public override init(frame: CGRect) {
        super.init(frame: frame)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }
}

// MARK: - SwiftUI Representable (`UIViewRepresentable` / `NSViewRepresentable`)

/// Cross-platform SwiftUI `View` hosting an NVE Rust component backed by `nve_get_interface`.
///
/// Supports:
/// - `sizeThatFits` intrinsic layout negotiation on iOS 16+ and macOS 13+
/// - Zero-JSON direct prop updates via `set_props_direct` (with binary `NveBinValue` fallback)
/// - Imperative commands via `NveSurfaceController`
/// - Strongly typed outbound event callbacks via `onEvent`
public struct NveSurfaceRepresentable {
    public let componentName: String
    public let vtableSymbol: String
    public let backend: NveRenderBackend
    public let props: [String: Any]
    public let controller: NveSurfaceController?
    public let onEvent: ((NveEvent) -> Void)?

    public init(
        componentName: String,
        vtableSymbol: String = "nve_get_interface",
        backend: NveRenderBackend = .fabric,
        props: [String: Any] = [:],
        controller: NveSurfaceController? = nil,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.componentName = componentName
        self.vtableSymbol = vtableSymbol
        self.backend = backend
        self.props = props
        self.controller = controller
        self.onEvent = onEvent
    }

    public final class Coordinator {
        let controller: NveSurfaceController
        let ownsController: Bool

        init(
            externalController: NveSurfaceController?,
            vtableSymbol: String,
            backend: NveRenderBackend
        ) {
            if let externalController {
                self.controller = externalController
                self.ownsController = false
            } else {
                self.controller = NveSurfaceController(
                    vtableSymbol: vtableSymbol,
                    backend: backend
                )
                self.ownsController = true
            }
        }

        deinit {
            if ownsController {
                controller.destroy()
            }
        }
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(
            externalController: controller,
            vtableSymbol: vtableSymbol,
            backend: backend
        )
    }

    private func sync(view: NveHostContainerView, coordinator: Coordinator) {
        let active = coordinator.controller
        active.onEvent = onEvent
        view.controller = active
        _ = active.setProps(props)
    }

    private func computeSizeThatFits(
        _ proposal: ProposedViewSize,
        coordinator: Coordinator
    ) -> CGSize {
        let maxW = proposal.width ?? .infinity
        let maxH = proposal.height ?? .infinity
        return coordinator.controller.measure(
            minWidth: 0,
            minHeight: 0,
            maxWidth: maxW,
            maxHeight: maxH
        )
    }
}

#if canImport(UIKit)
extension NveSurfaceRepresentable: UIViewRepresentable {
    public func makeUIView(context: Context) -> NveHostContainerView {
        let view = NveHostContainerView(frame: .zero)
        sync(view: view, coordinator: context.coordinator)
        return view
    }

    public func updateUIView(_ uiView: NveHostContainerView, context: Context) {
        sync(view: uiView, coordinator: context.coordinator)
    }

    public func sizeThatFits(
        _ proposal: ProposedViewSize,
        uiView: NveHostContainerView,
        context: Context
    ) -> CGSize? {
        computeSizeThatFits(proposal, coordinator: context.coordinator)
    }

    public static func dismantleUIView(_ uiView: NveHostContainerView, coordinator: Coordinator) {
        uiView.controller = nil
        if coordinator.ownsController {
            coordinator.controller.destroy()
        }
    }
}
#elseif canImport(AppKit)
extension NveSurfaceRepresentable: NSViewRepresentable {
    public func makeNSView(context: Context) -> NveHostContainerView {
        let view = NveHostContainerView(frame: .zero)
        sync(view: view, coordinator: context.coordinator)
        return view
    }

    public func updateNSView(_ nsView: NveHostContainerView, context: Context) {
        sync(view: nsView, coordinator: context.coordinator)
    }

    public func sizeThatFits(
        _ proposal: ProposedViewSize,
        nsView: NveHostContainerView,
        context: Context
    ) -> CGSize? {
        computeSizeThatFits(proposal, coordinator: context.coordinator)
    }

    public static func dismantleNSView(_ nsView: NveHostContainerView, coordinator: Coordinator) {
        nsView.controller = nil
        if coordinator.ownsController {
            coordinator.controller.destroy()
        }
    }
}
#endif
