/// Uniform 64-byte `NveEvent` value types, table-scoped event discriminants, `EventFlags`,
/// `EventWire` constants, and `EventValidationError` (mirrors `packages/core/abi/src/event/`).

/// Bitflags for `NveEvent.flags` (modifiers, pointer button, capture, IME, and ingress framing).
public struct EventFlags: OptionSet, Hashable, Sendable {
    public let rawValue: UInt32

    public init(rawValue: UInt32) { self.rawValue = rawValue }

    public static let modMeta = EventFlags(rawValue: nveEventFlagModMeta)
    public static let modCtrl = EventFlags(rawValue: nveEventFlagModCtrl)
    public static let modShift = EventFlags(rawValue: nveEventFlagModShift)
    public static let modAlt = EventFlags(rawValue: nveEventFlagModAlt)
    public static let modRepeat = EventFlags(rawValue: nveEventFlagModRepeat)
    public static let modMask = EventFlags(rawValue: 0x1F)
    public static let capture = EventFlags(rawValue: nveEventFlagCapture)
    public static let composing = EventFlags(rawValue: nveEventFlagComposing)
    public static let noBubble = EventFlags(rawValue: nveEventFlagNoBubble)
    public static let utf8Bytes = EventFlags(rawValue: nveEventFlagUtf8Bytes)
    public static let inlineKeyUtf8 = EventFlags(rawValue: nveEventFlagInlineKeyUtf8)
    public static let customNamed = EventFlags(rawValue: nveEventFlagCustomNamed)
    public static let keySplitBytes = EventFlags(rawValue: nveEventFlagKeySplitBytes)

    private static let internalFramingMask: UInt32 =
        nveEventFlagUtf8Bytes | nveEventFlagInlineKeyUtf8 | nveEventFlagCustomNamed | nveEventFlagKeySplitBytes

    public static func fromModifiers(
        meta: Bool = false, ctrl: Bool = false, shift: Bool = false, alt: Bool = false, repeat isRepeat: Bool = false
    ) -> EventFlags {
        var bits: UInt32 = 0
        if meta { bits |= nveEventFlagModMeta }
        if ctrl { bits |= nveEventFlagModCtrl }
        if shift { bits |= nveEventFlagModShift }
        if alt { bits |= nveEventFlagModAlt }
        if isRepeat { bits |= nveEventFlagModRepeat }
        return EventFlags(rawValue: bits)
    }

    public func withPointerButton(_ button: UInt8) -> EventFlags {
        let cleared = rawValue & ~nveEventFlagButtonMask
        return EventFlags(rawValue: cleared | ((UInt32(button) & 0x07) << nveEventFlagButtonShift))
    }

    public var pointerButton: UInt8 {
        UInt8(truncatingIfNeeded: (rawValue & nveEventFlagButtonMask) >> nveEventFlagButtonShift)
    }

    public var meta: Bool { contains(.modMeta) }
    public var ctrl: Bool { contains(.modCtrl) }
    public var shift: Bool { contains(.modShift) }
    public var alt: Bool { contains(.modAlt) }
    public var isRepeat: Bool { contains(.modRepeat) }
    public var hasPrimaryModifier: Bool { meta || ctrl }
    public var isCapture: Bool { contains(.capture) }
    public var isComposing: Bool { contains(.composing) }
    public var bubbles: Bool { !contains(.noBubble) }
    public func withBubbles(_ bubbles: Bool) -> EventFlags {
        bubbles ? subtracting(.noBubble) : union(.noBubble)
    }
    public var withoutInternalFraming: EventFlags { EventFlags(rawValue: rawValue & ~Self.internalFramingMask) }
}

/// Table-scoped event discriminant for `nve.interaction` (`NveInteractionEvent`).
public enum InteractionEventId: UInt32, Sendable, CaseIterable {
    case press = 0, longPress = 1, focus = 2, blur = 3, keyDown = 4, keyUp = 5
    case pointerDown = 6, pointerMove = 7, pointerUp = 8, pointerEnter = 9, pointerLeave = 10, wheel = 11

    /// Corresponding bit in `NveTables.Interaction.eventsWord` (`NveInteractionEventSet`).
    public var eventBit: UInt32 { UInt32(1) << rawValue }
}

/// Table-scoped event discriminant for `nve.text_input` (`NveTextInputEvent`).
public enum TextInputEventId: UInt32, Sendable, CaseIterable {
    case change = 0, submit = 1, compositionStart = 2, compositionUpdate = 3, compositionEnd = 4

    /// Corresponding bit in `NveTables.TextInput.eventsWord` (`NveTextInputEventSet`).
    public var eventBit: UInt32 { UInt32(1) << rawValue }
}

/// Table-scoped event discriminant for `nve.scroll` (`NveScrollEvent`).
public enum ScrollEventId: UInt32, Sendable, CaseIterable {
    case scroll = 0

    /// Corresponding bit in `NveTables.Scroll.eventsWord` (`NveScrollEventSet`).
    public var eventBit: UInt32 { UInt32(1) << rawValue }
}

/// Table-scoped event discriminant for `nve.layout` (`NveLayoutEvent`).
public enum LayoutEventId: UInt32, Sendable, CaseIterable {
    case layout = 0

    /// Corresponding bit in `NveTables.Layout.eventsWord` (`NveLayoutEventSet`).
    public var eventBit: UInt32 { UInt32(1) << rawValue }
}

/// Constants for the compact little-endian `NVEV` v2 binary wire format (`packages/core/abi/src/event/wire.rs`).
public enum EventWire {
    /// `"NVEV"` little-endian (`0x5645564E`).
    public static let magic: UInt32 = 0x5645_564E
    public static let version: UInt16 = 2
    /// `magic[4] + version:u16 + count:u32 + bytes_len:u32`.
    public static let headerSize: Int = 14
    public static let recordSize: Int = Int(nveEventSize)
}

/// Ingress and wire-codec validation errors (mirrors `EventValidationError` + `DecodeError`).
public enum EventValidationError: Error, Equatable, Sendable {
    case truncated
    case invalidMagic(UInt32)
    case unsupportedVersion(UInt16)
    case trailingBytes(Int)
    case nullEventsPointer
    case nullBytesPointer
    case eventsLengthOverflow
    case bytesLengthOverflow
    case spanOutOfBounds(index: Int, offset: UInt32, len: UInt32, arenaLen: Int)
    case invalidUtf8(index: Int)
    case invalidInlineKeyLen(index: Int, len: UInt32)
    case invalidSplitKeyLen(index: Int, keyLen: UInt32, bytesLen: Int)
    case invalidCustomNameLen(index: Int, nameLen: UInt32, bytesLen: Int)
    case conflictingFlags(index: Int)
}

/// Canonicalizes a raw generational handle (`SurfaceId` / `NodeId`) so index `0` is always `0`.
@inline(__always)
internal func canonicalHandle(_ raw: UInt32) -> UInt32 {
    (raw & NodeId.maxIndex) == 0 ? 0 : raw
}

/// Decoded value of one uniform 64-byte `NveEvent` record.
public struct EventRecord: Equatable, Sendable {
    public let timestamp: UInt64
    public let table: Key64
    public let surface: UInt32
    public let node: UInt32
    public let event: UInt32
    public let flags: EventFlags
    public let p0: UInt32
    public let p1: UInt32
    public let p2: UInt32
    public let p3: UInt32
    public let p4: UInt32
    public let p5: UInt32
    public let bytesOffset: UInt32
    public let bytesLen: UInt32

    public init(
        timestamp: UInt64, table: Key64, surface: UInt32, node: UInt32, event: UInt32,
        flags: EventFlags = [],
        p0: UInt32 = 0, p1: UInt32 = 0, p2: UInt32 = 0, p3: UInt32 = 0, p4: UInt32 = 0, p5: UInt32 = 0,
        bytesOffset: UInt32 = 0, bytesLen: UInt32 = 0
    ) {
        self.timestamp = timestamp
        self.table = table
        self.surface = canonicalHandle(surface)
        self.node = canonicalHandle(node)
        self.event = event
        self.flags = flags
        self.p0 = p0; self.p1 = p1; self.p2 = p2; self.p3 = p3; self.p4 = p4; self.p5 = p5
        self.bytesOffset = bytesOffset
        self.bytesLen = bytesLen
    }

    public var interaction: InteractionEventId? { table.value == nveTableInteraction ? InteractionEventId(rawValue: event) : nil }
    public var textInput: TextInputEventId? { table.value == nveTableTextInput ? TextInputEventId(rawValue: event) : nil }
    public var scrollEvent: ScrollEventId? { table.value == nveTableScroll ? ScrollEventId(rawValue: event) : nil }
    public var layoutEvent: LayoutEventId? { table.value == nveTableLayout ? LayoutEventId(rawValue: event) : nil }
    public var isCustom: Bool { table.value == nveTableCustom && flags.contains(.customNamed) }
    public var bubbles: Bool { flags.bubbles }

    public func payloadU32(_ index: Int) -> UInt32 {
        switch index {
        case 0: p0; case 1: p1; case 2: p2; case 3: p3; case 4: p4; case 5: p5
        default: 0
        }
    }

    public func payloadF32(_ index: Int) -> Float { Float(bitPattern: payloadU32(index)) }

    public func payloadU64(_ index: Int) -> UInt64 {
        UInt64(payloadU32(index)) | (UInt64(payloadU32(index + 1)) << 32)
    }

    public func payloadF64(_ index: Int) -> Double { Double(bitPattern: payloadU64(index)) }

    public var x: Float { Float(bitPattern: p0) }
    public var y: Float { Float(bitPattern: p1) }
    public var deltaX: Float { Float(bitPattern: p0) }
    public var deltaY: Float { Float(bitPattern: p1) }
    public var width: Float { Float(bitPattern: p2) }
    public var height: Float { Float(bitPattern: p3) }
    public var pointerId: UInt32 { p2 }
    public var keyCode: UInt32 { p0 }
}
