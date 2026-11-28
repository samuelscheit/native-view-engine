/// Single-pass ingress-validated `EventBatchReader` and reusable `EventBatchBuilder` for
/// uniform 64-byte `NveEvent` batches and the `NVEV` v2 binary wire format
/// (mirrors `packages/core/abi/src/event/{buf,validated,wire}.rs`).
///
/// Storage in `EventBatchBuilder` is a contiguous `[UInt64]` slab (8 `UInt64`s = 64 bytes per
/// record, 8-byte aligned, little-endian, matching `CNveAbi.NveEvent` layout byte-for-byte) plus
/// a flat `[UInt8]` byte arena. Reusing a builder with `clear()` keeps capacity and performs zero
/// heap allocations in steady state.

/// Zero-copy, zero-allocation ingress-validated view over a contiguous 64-byte `NveEvent` record
/// array and its shared byte arena (mirrors Rust `ValidatedEvents`).
public struct EventBatchReader {
    public let records: UnsafeRawBufferPointer
    public let count: Int
    public let arena: UnsafeRawBufferPointer

    public var isEmpty: Bool { count == 0 }

    /// Creates a pre-sealed reader (only for buffers whose invariants were sealed at construction).
    internal init(sealedRecords: UnsafeRawBufferPointer, count: Int, arena: UnsafeRawBufferPointer) {
        self.records = sealedRecords
        self.count = count
        self.arena = arena
    }

    /// Validates an `NVEV` v2 wire buffer in a single linear pass with zero heap allocations.
    public static func validate(wireBytes raw: UnsafeRawBufferPointer) throws(EventValidationError) -> EventBatchReader {
        guard raw.count >= EventWire.headerSize else { throw .truncated }
        let magic = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
        guard magic == EventWire.magic else { throw .invalidMagic(magic) }
        let version = UInt16(littleEndian: raw.loadUnaligned(fromByteOffset: 4, as: UInt16.self))
        guard version == EventWire.version else { throw .unsupportedVersion(version) }
        let countU32 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: 6, as: UInt32.self))
        let bytesLenU32 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: 10, as: UInt32.self))
        let evBytes64 = UInt64(countU32) * UInt64(EventWire.recordSize)
        let expected64 = UInt64(EventWire.headerSize) + evBytes64 + UInt64(bytesLenU32)
        guard UInt64(raw.count) >= expected64 else { throw .truncated }
        if UInt64(raw.count) > expected64 {
            throw .trailingBytes(raw.count - Int(expected64))
        }
        let count = Int(countU32)
        let arenaStart = EventWire.headerSize + Int(evBytes64)
        let recSlice = UnsafeRawBufferPointer(rebasing: raw[EventWire.headerSize..<arenaStart])
        let arenaSlice = UnsafeRawBufferPointer(rebasing: raw[arenaStart..<Int(expected64)])
        return try validate(records: recSlice, count: count, arena: arenaSlice)
    }

    /// Validates `count` contiguous 64-byte records against `arena` in one linear pass (zero alloc).
    public static func validate(
        records: UnsafeRawBufferPointer, count: Int, arena: UnsafeRawBufferPointer
    ) throws(EventValidationError) -> EventBatchReader {
        guard count >= 0, UInt64(count) <= UInt64(UInt32.max),
              let needed = TableReader.fitsAddressSpace(UInt64(count), stride: EventWire.recordSize),
              records.count >= needed * EventWire.recordSize
        else { throw .eventsLengthOverflow }
        guard UInt64(arena.count) <= UInt64(UInt32.max) else { throw .bytesLengthOverflow }
        for idx in 0..<count {
            try validateOneEvent(index: idx, records: records, arena: arena)
        }
        return EventBatchReader(
            sealedRecords: UnsafeRawBufferPointer(rebasing: records[0..<(count * EventWire.recordSize)]),
            count: count,
            arena: arena
        )
    }

    /// Convenience scoped validator over a `[UInt8]` `NVEV` v2 frame.
    public static func withValidatedWire<R>(
        _ wireBytes: [UInt8], _ body: (EventBatchReader) throws -> R
    ) throws -> R {
        try wireBytes.withUnsafeBytes { raw in
            let reader = try validate(wireBytes: raw)
            return try body(reader)
        }
    }

    private static func validateOneEvent(
        index: Int, records: UnsafeRawBufferPointer, arena: UnsafeRawBufferPointer
    ) throws(EventValidationError) {
        let base = index * EventWire.recordSize
        let flags = UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 28, as: UInt32.self))
        let p0 = UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 32, as: UInt32.self))
        let p1 = UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 36, as: UInt32.self))
        let off = UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 56, as: UInt32.self))
        let len = UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 60, as: UInt32.self))
        let end64 = UInt64(off) + UInt64(len)
        guard end64 <= UInt64(arena.count) else {
            throw .spanOutOfBounds(index: index, offset: off, len: len, arenaLen: arena.count)
        }
        let slice = UnsafeRawBufferPointer(rebasing: arena[Int(off)..<Int(end64)])

        let hasUtf8 = (flags & nveEventFlagUtf8Bytes) != 0
        let hasInlineKey = (flags & nveEventFlagInlineKeyUtf8) != 0
        let hasSplitKey = (flags & nveEventFlagKeySplitBytes) != 0
        let hasCustom = (flags & nveEventFlagCustomNamed) != 0

        if (hasInlineKey && hasSplitKey) || (hasCustom && (hasInlineKey || hasSplitKey)) {
            throw .conflictingFlags(index: index)
        }
        if hasUtf8 && !Utf8.isValid(slice) {
            throw .invalidUtf8(index: index)
        }
        if hasInlineKey {
            guard p1 <= nveEventInlineKeyCap else {
                throw .invalidInlineKeyLen(index: index, len: p1)
            }
            let inlineSlice = UnsafeRawBufferPointer(rebasing: records[(base + 40)..<(base + 40 + Int(p1))])
            if !Utf8.isValid(inlineSlice) { throw .invalidUtf8(index: index) }
        }
        if hasSplitKey {
            guard UInt64(p1) <= UInt64(slice.count) else {
                throw .invalidSplitKeyLen(index: index, keyLen: p1, bytesLen: slice.count)
            }
            let klen = Int(p1)
            if hasUtf8 {
                if klen < slice.count && (slice[klen] & 0xC0) == 0x80 {
                    throw .invalidUtf8(index: index)
                }
            } else {
                let kSlice = UnsafeRawBufferPointer(rebasing: slice[0..<klen])
                let cSlice = UnsafeRawBufferPointer(rebasing: slice[klen..<slice.count])
                if !Utf8.isValid(kSlice) || !Utf8.isValid(cSlice) { throw .invalidUtf8(index: index) }
            }
        }
        if hasCustom {
            guard UInt64(p0) <= UInt64(slice.count) else {
                throw .invalidCustomNameLen(index: index, nameLen: p0, bytesLen: slice.count)
            }
            let nSlice = UnsafeRawBufferPointer(rebasing: slice[0..<Int(p0)])
            if !Utf8.isValid(nSlice) { throw .invalidUtf8(index: index) }
        }
    }

    public func record(at index: Int) -> EventRecord? {
        guard (0..<count).contains(index) else { return nil }
        let base = index * EventWire.recordSize
        return EventRecord(
            timestamp: UInt64(littleEndian: records.loadUnaligned(fromByteOffset: base + 0, as: UInt64.self)),
            table: Key64(UInt64(littleEndian: records.loadUnaligned(fromByteOffset: base + 8, as: UInt64.self))),
            surface: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 16, as: UInt32.self)),
            node: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 20, as: UInt32.self)),
            event: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 24, as: UInt32.self)),
            flags: EventFlags(rawValue: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 28, as: UInt32.self))),
            p0: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 32, as: UInt32.self)),
            p1: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 36, as: UInt32.self)),
            p2: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 40, as: UInt32.self)),
            p3: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 44, as: UInt32.self)),
            p4: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 48, as: UInt32.self)),
            p5: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 52, as: UInt32.self)),
            bytesOffset: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 56, as: UInt32.self)),
            bytesLen: UInt32(littleEndian: records.loadUnaligned(fromByteOffset: base + 60, as: UInt32.self))
        )
    }

    public func forEach(_ visit: (Int, EventRecord) throws -> Void) rethrows {
        for i in 0..<count {
            if let rec = record(at: i) { try visit(i, rec) }
        }
    }

    /// Raw arena slice owned by event `index` (empty for out-of-range indices).
    public func bytes(at index: Int) -> UnsafeRawBufferPointer {
        guard let rec = record(at: index) else { return UnsafeRawBufferPointer(start: nil, count: 0) }
        let start = Int(rec.bytesOffset)
        return UnsafeRawBufferPointer(rebasing: arena[start..<(start + Int(rec.bytesLen))])
    }

    /// Pre-validated UTF-8 text payload slice (`nil` if not a non-split `UTF8_BYTES` event).
    public func textBytes(at index: Int) -> UnsafeRawBufferPointer? {
        guard let rec = record(at: index),
              rec.flags.contains(.utf8Bytes), !rec.flags.contains(.keySplitBytes)
        else { return nil }
        return bytes(at: index)
    }

    public func text(at index: Int) -> String {
        textBytes(at: index).map { String(decoding: $0, as: UTF8.self) } ?? ""
    }

    /// Pre-validated W3C key UTF-8 slice (`<= 16 B` inline or `KEY_SPLIT_BYTES` arena prefix).
    public func keyBytes(at index: Int) -> UnsafeRawBufferPointer? {
        guard let rec = record(at: index) else { return nil }
        if rec.flags.contains(.inlineKeyUtf8) {
            let base = index * EventWire.recordSize + 40
            return UnsafeRawBufferPointer(rebasing: records[base..<(base + Int(rec.p1))])
        }
        if rec.flags.contains(.keySplitBytes) {
            let slice = bytes(at: index)
            return UnsafeRawBufferPointer(rebasing: slice[0..<Int(rec.p1)])
        }
        return nil
    }

    public func key(at index: Int) -> String {
        keyBytes(at: index).map { String(decoding: $0, as: UTF8.self) } ?? ""
    }

    /// Pre-validated physical key code UTF-8 slice.
    public func codeBytes(at index: Int) -> UnsafeRawBufferPointer? {
        guard let rec = record(at: index) else { return nil }
        if rec.flags.contains(.inlineKeyUtf8) && rec.flags.contains(.utf8Bytes) {
            return bytes(at: index)
        }
        if rec.flags.contains(.keySplitBytes) {
            let slice = bytes(at: index)
            return UnsafeRawBufferPointer(rebasing: slice[Int(rec.p1)..<slice.count])
        }
        return nil
    }

    public func code(at index: Int) -> String {
        codeBytes(at: index).map { String(decoding: $0, as: UTF8.self) } ?? ""
    }

    /// Pre-validated custom event name UTF-8 slice (`NVE_TABLE_CUSTOM` + `CUSTOM_NAMED`).
    public func customNameBytes(at index: Int) -> UnsafeRawBufferPointer? {
        guard let rec = record(at: index), rec.flags.contains(.customNamed) else { return nil }
        let slice = bytes(at: index)
        return UnsafeRawBufferPointer(rebasing: slice[0..<Int(rec.p0)])
    }

    public func customName(at index: Int) -> String {
        customNameBytes(at: index).map { String(decoding: $0, as: UTF8.self) } ?? ""
    }

    /// Raw custom event payload bytes following the UTF-8 name prefix.
    public func customData(at index: Int) -> UnsafeRawBufferPointer {
        guard let rec = record(at: index) else { return UnsafeRawBufferPointer(start: nil, count: 0) }
        let slice = bytes(at: index)
        if rec.flags.contains(.customNamed) {
            return UnsafeRawBufferPointer(rebasing: slice[Int(rec.p0)..<slice.count])
        }
        return slice
    }
}

/// Reusable owned batch of uniform 64-byte `NveEvent` records + companion byte arena
/// (mirrors Rust `EventBatchBuf`). Seals every `EventBatchReader` invariant by construction.
public struct EventBatchBuilder: Sendable {
    /// 8 `UInt64`s (64 bytes, 8-byte aligned, little-endian) per record.
    private var recordWords: [UInt64] = []
    public private(set) var bytes: [UInt8] = []

    public init(eventCapacity: Int = 0, byteCapacity: Int = 0) {
        if eventCapacity > 0 { recordWords.reserveCapacity(eventCapacity * 8) }
        if byteCapacity > 0 { bytes.reserveCapacity(byteCapacity) }
    }

    public var count: Int { recordWords.count / 8 }
    public var isEmpty: Bool { recordWords.isEmpty }
    public var recordByteCount: Int { recordWords.count * 8 }

    public mutating func clear() {
        recordWords.removeAll(keepingCapacity: true)
        bytes.removeAll(keepingCapacity: true)
    }

    public func withRecordWords<R>(_ body: (UnsafeBufferPointer<UInt64>) throws -> R) rethrows -> R {
        try recordWords.withUnsafeBufferPointer(body)
    }

    public func withRecordBytes<R>(_ body: (UnsafeRawBufferPointer) throws -> R) rethrows -> R {
        try recordWords.withUnsafeBytes(body)
    }

    public func withArenaBytes<R>(_ body: (UnsafeRawBufferPointer) throws -> R) rethrows -> R {
        try bytes.withUnsafeBytes(body)
    }

    /// O(1) sealed zero-copy reader over this builder (no re-validation, zero allocations).
    public func withValidated<R>(_ body: (EventBatchReader) throws -> R) rethrows -> R {
        try recordWords.withUnsafeBytes { rawRecs in
            try bytes.withUnsafeBytes { rawArena in
                try body(EventBatchReader(sealedRecords: rawRecs, count: count, arena: rawArena))
            }
        }
    }

    /// Creates an owned copy of an already-validated reader (used by wire decode and C ABI bridge).
    public static func copy(from reader: EventBatchReader) -> EventBatchBuilder {
        var builder = EventBatchBuilder(eventCapacity: reader.count, byteCapacity: reader.arena.count)
        builder.bytes.append(contentsOf: reader.arena)
        for i in 0..<reader.count {
            if let rec = reader.record(at: i) {
                builder.appendRecord(rec)
            }
        }
        return builder
    }

    // MARK: low-level push

    private func checkRoom() throws(EventValidationError) {
        guard count < Int(UInt32.max) else { throw .eventsLengthOverflow }
    }

    private mutating func appendArena(_ data: UnsafeRawBufferPointer) throws(EventValidationError) -> (UInt32, UInt32) {
        guard !data.isEmpty else { return (0, 0) }
        guard let off = UInt32(exactly: bytes.count),
              let len = UInt32(exactly: data.count),
              off.addingReportingOverflow(len).overflow == false
        else { throw .bytesLengthOverflow }
        bytes.append(contentsOf: data)
        return (off, len)
    }

    private mutating func appendArenaPair(
        _ first: UnsafeRawBufferPointer, _ second: UnsafeRawBufferPointer
    ) throws(EventValidationError) -> (UInt32, UInt32) {
        let (totalInt, overflow) = first.count.addingReportingOverflow(second.count)
        guard !overflow, let total = UInt32(exactly: totalInt) else { throw .bytesLengthOverflow }
        guard total > 0 else { return (0, 0) }
        guard let off = UInt32(exactly: bytes.count),
              off.addingReportingOverflow(total).overflow == false
        else { throw .bytesLengthOverflow }
        if !first.isEmpty { bytes.append(contentsOf: first) }
        if !second.isEmpty { bytes.append(contentsOf: second) }
        return (off, total)
    }

    private mutating func appendRecord(_ r: EventRecord) {
        recordWords.append(r.timestamp.littleEndian)
        recordWords.append(r.table.value.littleEndian)
        recordWords.append((UInt64(r.surface) | (UInt64(r.node) << 32)).littleEndian)
        recordWords.append((UInt64(r.event) | (UInt64(r.flags.rawValue) << 32)).littleEndian)
        recordWords.append((UInt64(r.p0) | (UInt64(r.p1) << 32)).littleEndian)
        recordWords.append((UInt64(r.p2) | (UInt64(r.p3) << 32)).littleEndian)
        recordWords.append((UInt64(r.p4) | (UInt64(r.p5) << 32)).littleEndian)
        recordWords.append((UInt64(r.bytesOffset) | (UInt64(r.bytesLen) << 32)).littleEndian)
    }

    public mutating func pushTableEvent(
        table: Key64, surface: UInt32, node: UInt32, timestamp: UInt64, event: UInt32,
        flags: EventFlags = [],
        p0: UInt32 = 0, p1: UInt32 = 0, p2: UInt32 = 0, p3: UInt32 = 0, p4: UInt32 = 0, p5: UInt32 = 0,
        data: UnsafeRawBufferPointer = UnsafeRawBufferPointer(start: nil, count: 0)
    ) {
        guard (try? checkRoom()) != nil, let (off, len) = try? appendArena(data) else { return }
        appendRecord(EventRecord(
            timestamp: timestamp, table: table, surface: surface, node: node, event: event,
            flags: flags.withoutInternalFraming, p0: p0, p1: p1, p2: p2, p3: p3, p4: p4, p5: p5,
            bytesOffset: off, bytesLen: len
        ))
    }

    public mutating func pushEventText(
        table: Key64, surface: UInt32, node: UInt32, timestamp: UInt64, event: UInt32,
        flags: EventFlags = [],
        p0: UInt32 = 0, p1: UInt32 = 0, p2: UInt32 = 0, p3: UInt32 = 0, p4: UInt32 = 0, p5: UInt32 = 0,
        text: String
    ) {
        guard (try? checkRoom()) != nil else { return }
        var copy = text
        let span: (UInt32, UInt32)? = copy.withUTF8 { try? appendArena(UnsafeRawBufferPointer($0)) }
        guard let (off, len) = span else { return }
        appendRecord(EventRecord(
            timestamp: timestamp, table: table, surface: surface, node: node, event: event,
            flags: flags.withoutInternalFraming.union(.utf8Bytes),
            p0: p0, p1: p1, p2: p2, p3: p3, p4: p4, p5: p5,
            bytesOffset: off, bytesLen: len
        ))
    }

    // MARK: typed event helpers (match Rust EventBatchBuf)

    public mutating func pushPress(surface: UInt32, node: UInt32, timestamp: UInt64, x: Float, y: Float) {
        pushTableEvent(
            table: Key64(nveTableInteraction), surface: surface, node: node, timestamp: timestamp,
            event: InteractionEventId.press.rawValue, p0: x.bitPattern, p1: y.bitPattern
        )
    }

    public mutating func pushLongPress(surface: UInt32, node: UInt32, timestamp: UInt64, x: Float, y: Float) {
        pushTableEvent(
            table: Key64(nveTableInteraction), surface: surface, node: node, timestamp: timestamp,
            event: InteractionEventId.longPress.rawValue, p0: x.bitPattern, p1: y.bitPattern
        )
    }

    public mutating func pushFocus(surface: UInt32, node: UInt32, timestamp: UInt64) {
        pushTableEvent(
            table: Key64(nveTableInteraction), surface: surface, node: node, timestamp: timestamp,
            event: InteractionEventId.focus.rawValue
        )
    }

    public mutating func pushBlur(surface: UInt32, node: UInt32, timestamp: UInt64) {
        pushTableEvent(
            table: Key64(nveTableInteraction), surface: surface, node: node, timestamp: timestamp,
            event: InteractionEventId.blur.rawValue
        )
    }

    public mutating func pushKey(
        surface: UInt32, node: UInt32, timestamp: UInt64, event: InteractionEventId,
        key: String, code: String = "", flags: EventFlags = [], keyCode: UInt32 = 0
    ) {
        guard (try? checkRoom()) != nil else { return }
        let baseFlags = flags.withoutInternalFraming
        var keyCopy = key
        var codeCopy = code
        keyCopy.withUTF8 { keyUtf8 in
            codeCopy.withUTF8 { codeUtf8 in
                if keyUtf8.count <= Int(nveEventInlineKeyCap) {
                    var lanes: (UInt32, UInt32, UInt32, UInt32) = (0, 0, 0, 0)
                    withUnsafeMutableBytes(of: &lanes) { dst in
                        if let src = keyUtf8.baseAddress, !keyUtf8.isEmpty {
                            dst.baseAddress?.copyMemory(from: src, byteCount: keyUtf8.count)
                        }
                    }
                    guard let (off, len) = try? appendArena(UnsafeRawBufferPointer(codeUtf8)) else { return }
                    var f = baseFlags.union(.inlineKeyUtf8)
                    if !codeUtf8.isEmpty { f.insert(.utf8Bytes) }
                    appendRecord(EventRecord(
                        timestamp: timestamp, table: Key64(nveTableInteraction), surface: surface, node: node,
                        event: event.rawValue, flags: f,
                        p0: keyCode, p1: UInt32(keyUtf8.count),
                        p2: UInt32(littleEndian: lanes.0), p3: UInt32(littleEndian: lanes.1),
                        p4: UInt32(littleEndian: lanes.2), p5: UInt32(littleEndian: lanes.3),
                        bytesOffset: off, bytesLen: len
                    ))
                } else {
                    guard let klen = UInt32(exactly: keyUtf8.count),
                          let (off, len) = try? appendArenaPair(
                              UnsafeRawBufferPointer(keyUtf8), UnsafeRawBufferPointer(codeUtf8))
                    else { return }
                    appendRecord(EventRecord(
                        timestamp: timestamp, table: Key64(nveTableInteraction), surface: surface, node: node,
                        event: event.rawValue, flags: baseFlags.union([.keySplitBytes, .utf8Bytes]),
                        p0: keyCode, p1: klen, bytesOffset: off, bytesLen: len
                    ))
                }
            }
        }
    }

    public mutating func pushKeyDown(
        surface: UInt32, node: UInt32, timestamp: UInt64,
        key: String, code: String = "", flags: EventFlags = [], keyCode: UInt32 = 0
    ) {
        pushKey(surface: surface, node: node, timestamp: timestamp, event: .keyDown,
                key: key, code: code, flags: flags, keyCode: keyCode)
    }

    public mutating func pushKeyUp(
        surface: UInt32, node: UInt32, timestamp: UInt64,
        key: String, code: String = "", flags: EventFlags = [], keyCode: UInt32 = 0
    ) {
        pushKey(surface: surface, node: node, timestamp: timestamp, event: .keyUp,
                key: key, code: code, flags: flags, keyCode: keyCode)
    }

    public mutating func pushPointer(
        surface: UInt32, node: UInt32, timestamp: UInt64, event: InteractionEventId,
        x: Float, y: Float, pointerId: UInt32 = 0, flags: EventFlags = []
    ) {
        pushTableEvent(
            table: Key64(nveTableInteraction), surface: surface, node: node, timestamp: timestamp,
            event: event.rawValue, flags: flags, p0: x.bitPattern, p1: y.bitPattern, p2: pointerId
        )
    }

    public mutating func pushWheel(
        surface: UInt32, node: UInt32, timestamp: UInt64,
        deltaX: Float, deltaY: Float, flags: EventFlags = []
    ) {
        pushTableEvent(
            table: Key64(nveTableInteraction), surface: surface, node: node, timestamp: timestamp,
            event: InteractionEventId.wheel.rawValue, flags: flags,
            p0: deltaX.bitPattern, p1: deltaY.bitPattern
        )
    }

    public mutating func pushTextChange(surface: UInt32, node: UInt32, timestamp: UInt64, text: String) {
        pushEventText(
            table: Key64(nveTableTextInput), surface: surface, node: node, timestamp: timestamp,
            event: TextInputEventId.change.rawValue, text: text
        )
    }

    public mutating func pushSubmit(surface: UInt32, node: UInt32, timestamp: UInt64, text: String) {
        pushEventText(
            table: Key64(nveTableTextInput), surface: surface, node: node, timestamp: timestamp,
            event: TextInputEventId.submit.rawValue, text: text
        )
    }

    public mutating func pushComposition(
        surface: UInt32, node: UInt32, timestamp: UInt64, event: TextInputEventId, text: String
    ) {
        pushEventText(
            table: Key64(nveTableTextInput), surface: surface, node: node, timestamp: timestamp,
            event: event.rawValue, text: text
        )
    }

    public mutating func pushScroll(
        surface: UInt32, node: UInt32, timestamp: UInt64,
        x: Float, y: Float, contentWidth: Float = 0, contentHeight: Float = 0
    ) {
        pushTableEvent(
            table: Key64(nveTableScroll), surface: surface, node: node, timestamp: timestamp,
            event: ScrollEventId.scroll.rawValue,
            p0: x.bitPattern, p1: y.bitPattern, p2: contentWidth.bitPattern, p3: contentHeight.bitPattern
        )
    }

    public mutating func pushLayout(
        surface: UInt32, node: UInt32, timestamp: UInt64,
        x: Float, y: Float, width: Float, height: Float
    ) {
        pushTableEvent(
            table: Key64(nveTableLayout), surface: surface, node: node, timestamp: timestamp,
            event: LayoutEventId.layout.rawValue,
            p0: x.bitPattern, p1: y.bitPattern, p2: width.bitPattern, p3: height.bitPattern
        )
    }

    public mutating func tryPushCustom(
        surface: UInt32, node: UInt32, timestamp: UInt64, name: String, data: UnsafeRawBufferPointer,
        flags: EventFlags = []
    ) throws(EventValidationError) {
        try checkRoom()
        var nameCopy = name
        var nlen: UInt32 = 0
        let (off, len) = try nameCopy.withUTF8 { (nameUtf8) throws(EventValidationError) -> (UInt32, UInt32) in
            guard let nl = UInt32(exactly: nameUtf8.count) else { throw .bytesLengthOverflow }
            nlen = nl
            return try appendArenaPair(UnsafeRawBufferPointer(nameUtf8), data)
        }
        appendRecord(EventRecord(
            timestamp: timestamp, table: Key64(nveTableCustom), surface: surface, node: node,
            event: 0, flags: flags.withoutInternalFraming.union(.customNamed), p0: nlen,
            bytesOffset: off, bytesLen: len
        ))
    }

    public mutating func pushCustom<Bytes: Collection>(
        surface: UInt32, node: UInt32, timestamp: UInt64, name: String, data: Bytes,
        flags: EventFlags = []
    ) where Bytes.Element == UInt8 {
        let done: Void? = data.withContiguousStorageIfAvailable {
            try? tryPushCustom(surface: surface, node: node, timestamp: timestamp, name: name,
                               data: UnsafeRawBufferPointer($0), flags: flags)
        }
        if done == nil {
            Array(data).withUnsafeBytes {
                try? tryPushCustom(surface: surface, node: node, timestamp: timestamp, name: name,
                                   data: $0, flags: flags)
            }
        }
    }

    // MARK: NVEV v2 wire codec

    public func tryEncodeWire(into out: inout [UInt8]) throws(EventValidationError) {
        guard let countU32 = UInt32(exactly: count) else { throw .eventsLengthOverflow }
        guard let bytesLenU32 = UInt32(exactly: bytes.count) else { throw .bytesLengthOverflow }
        let total = EventWire.headerSize + recordByteCount + bytes.count
        if out.capacity - out.count < total { out.reserveCapacity(out.count + total) }
        withUnsafeBytes(of: EventWire.magic.littleEndian) { out.append(contentsOf: $0) }
        withUnsafeBytes(of: EventWire.version.littleEndian) { out.append(contentsOf: $0) }
        withUnsafeBytes(of: countU32.littleEndian) { out.append(contentsOf: $0) }
        withUnsafeBytes(of: bytesLenU32.littleEndian) { out.append(contentsOf: $0) }
        recordWords.withUnsafeBytes { out.append(contentsOf: $0) }
        bytes.withUnsafeBytes { out.append(contentsOf: $0) }
    }

    public func encodeWire(into out: inout [UInt8]) {
        try? tryEncodeWire(into: &out)
    }

    public func toWireBytes() -> [UInt8] {
        var out: [UInt8] = []
        encodeWire(into: &out)
        return out
    }

    public static func decodeWire(_ raw: UnsafeRawBufferPointer) throws(EventValidationError) -> EventBatchBuilder {
        copy(from: try EventBatchReader.validate(wireBytes: raw))
    }

    public static func decodeWire(_ raw: [UInt8]) throws(EventValidationError) -> EventBatchBuilder {
        try raw.withUnsafeBytes { buf -> Result<EventBatchBuilder, EventValidationError> in
            do throws(EventValidationError) { return .success(try decodeWire(buf)) }
            catch { return .failure(error) }
        }.get()
    }
}
