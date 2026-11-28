import CNveAbi
import CNveHostTestABI
import NveHost
import NveHostAbi
import XCTest

private func allocations(_ body: () throws -> Void) rethrows -> Int64? {
    guard nve_test_alloc_count_begin() != 0 else { return nil }
    do {
        try body()
    } catch {
        _ = nve_test_alloc_count_end()
        throw error
    }
    return nve_test_alloc_count_end()
}

final class EventCodecTests: XCTestCase {
    func testUniform64BEventBuilderAndNvevV2RoundTrip() throws {
        var b = EventBatchBuilder(eventCapacity: 12, byteCapacity: 256)
        b.pushPress(surface: 1, node: 10, timestamp: 100, x: 12.5, y: 34.25)
        b.pushLongPress(surface: 1, node: 11, timestamp: 101, x: -2, y: 99)
        b.pushFocus(surface: 1, node: 12, timestamp: 102)
        b.pushBlur(surface: 1, node: 12, timestamp: 103)
        // Inline UTF-8 key (<= 16 B, including multi-byte emoji) + physical code in arena.
        b.pushKeyDown(
            surface: 1, node: 13, timestamp: 104,
            key: "é🚀", code: "KeyE",
            flags: EventFlags.fromModifiers(meta: true, shift: true, repeat: true).union(.composing),
            keyCode: 69
        )
        // Exact 16-byte boundary inline key (still inline, no split).
        b.pushKeyUp(surface: 1, node: 13, timestamp: 105, key: "0123456789ABCDEF", code: "KeyA")
        // > 16 B key -> KEY_SPLIT_BYTES arena path.
        b.pushKeyDown(surface: 1, node: 13, timestamp: 106, key: "MediaTrackPreviousLongKey", code: "MediaTrackPrevious")
        // Pointer + button + capture + modifiers.
        let pFlags = EventFlags.fromModifiers(ctrl: true, alt: true).withPointerButton(2).union(.capture)
        b.pushPointer(surface: 1, node: 14, timestamp: 107, event: .pointerMove, x: 5.5, y: 6.5, pointerId: 7, flags: pFlags)
        b.pushWheel(surface: 1, node: 14, timestamp: 108, deltaX: -1.5, deltaY: 12.0, flags: .modShift)
        b.pushTextChange(surface: 1, node: 15, timestamp: 109, text: "hello 世界")
        b.pushSubmit(surface: 1, node: 15, timestamp: 110, text: "done")
        b.pushComposition(surface: 1, node: 15, timestamp: 111, event: .compositionUpdate, text: "にほ")
        b.pushScroll(surface: 1, node: 16, timestamp: 112, x: 0, y: 240, contentWidth: 320, contentHeight: 1200)
        b.pushLayout(surface: 1, node: 17, timestamp: 113, x: 10, y: 20, width: 300, height: 80)
        b.pushCustom(surface: 1, node: 18, timestamp: 114, name: "my.event", data: [0x00, 0xFF, 0xFE, 0x01])

        XCTAssertEqual(b.count, 15)
        let wire = b.toWireBytes()
        XCTAssertEqual(wire.count, EventWire.headerSize + 15 * 64 + b.bytes.count)
        // Verify NVEV v2 header bytes verbatim.
        XCTAssertEqual(Array(wire[0..<6]), [0x4E, 0x56, 0x45, 0x56, 0x02, 0x00])

        let decoded = try EventBatchBuilder.decodeWire(wire)
        XCTAssertEqual(decoded.toWireBytes(), wire)

        try EventBatchReader.withValidatedWire(wire) { r in
            XCTAssertEqual(r.count, 15)
            let e0 = try XCTUnwrap(r.record(at: 0))
            XCTAssertEqual(e0.interaction, .press)
            XCTAssertEqual(e0.x, 12.5)
            XCTAssertEqual(e0.y, 34.25)

            let e4 = try XCTUnwrap(r.record(at: 4))
            XCTAssertEqual(e4.interaction, .keyDown)
            XCTAssertTrue(e4.flags.contains(.inlineKeyUtf8))
            XCTAssertTrue(e4.flags.meta && e4.flags.shift && e4.flags.isRepeat && e4.flags.isComposing)
            XCTAssertEqual(e4.keyCode, 69)
            XCTAssertEqual(r.key(at: 4), "é🚀")
            XCTAssertEqual(r.code(at: 4), "KeyE")

            let e5 = try XCTUnwrap(r.record(at: 5))
            XCTAssertTrue(e5.flags.contains(.inlineKeyUtf8))
            XCTAssertEqual(r.key(at: 5), "0123456789ABCDEF")
            XCTAssertEqual(r.code(at: 5), "KeyA")

            let e6 = try XCTUnwrap(r.record(at: 6))
            XCTAssertTrue(e6.flags.contains(.keySplitBytes))
            XCTAssertEqual(r.key(at: 6), "MediaTrackPreviousLongKey")
            XCTAssertEqual(r.code(at: 6), "MediaTrackPrevious")

            let e7 = try XCTUnwrap(r.record(at: 7))
            XCTAssertEqual(e7.interaction, .pointerMove)
            XCTAssertEqual(e7.pointerId, 7)
            XCTAssertEqual(e7.flags.pointerButton, 2)
            XCTAssertTrue(e7.flags.isCapture && e7.flags.ctrl && e7.flags.alt && e7.flags.hasPrimaryModifier)

            XCTAssertEqual(r.record(at: 9)?.textInput, .change)
            XCTAssertEqual(r.text(at: 9), "hello 世界")
            XCTAssertEqual(r.record(at: 10)?.textInput, .submit)
            XCTAssertEqual(r.text(at: 10), "done")
            XCTAssertEqual(r.record(at: 11)?.textInput, .compositionUpdate)
            XCTAssertEqual(r.text(at: 11), "にほ")

            let e12 = try XCTUnwrap(r.record(at: 12))
            XCTAssertEqual(e12.scrollEvent, .scroll)
            XCTAssertEqual(e12.y, 240)
            XCTAssertEqual(e12.height, 1200)

            let e13 = try XCTUnwrap(r.record(at: 13))
            XCTAssertEqual(e13.layoutEvent, .layout)
            XCTAssertEqual([e13.x, e13.y, e13.width, e13.height], [10, 20, 300, 80])

            let e14 = try XCTUnwrap(r.record(at: 14))
            XCTAssertTrue(e14.isCustom)
            XCTAssertEqual(r.customName(at: 14), "my.event")
            XCTAssertEqual(Array(r.customData(at: 14)), [0x00, 0xFF, 0xFE, 0x01])
        }
    }

    func testCAbiEventBatchBridgeMatchesCFieldOffsets() throws {
        var b = EventBatchBuilder()
        b.pushKeyDown(surface: 0x0300_0002, node: 0x0500_0007, timestamp: 0x1122_3344_5566_7788,
                      key: "Enter", code: "Enter", flags: .modMeta, keyCode: 13)
        b.pushCustom(surface: 1, node: 2, timestamp: 999, name: "ev", data: [0xAB, 0xCD])

        try b.withRawBatch { batch in
            XCTAssertEqual(batch.event_count, 2)
            XCTAssertEqual(batch.bytes_len, UInt32(b.bytes.count))
            let first = try XCTUnwrap(batch.events).pointee
            // Verify C struct fields directly through ClangImporter layout!
            XCTAssertEqual(first.timestamp, 0x1122_3344_5566_7788)
            XCTAssertEqual(first.table, UInt64(NVE_TABLE_INTERACTION))
            XCTAssertEqual(first.surface, 0x0300_0002)
            XCTAssertEqual(first.node, 0x0500_0007)
            XCTAssertEqual(first.event, UInt32(NVE_INTERACTION_EVENT_KEY_DOWN.rawValue))
            XCTAssertEqual(first.flags, nveEventFlagModMeta | nveEventFlagInlineKeyUtf8 | nveEventFlagUtf8Bytes)
            XCTAssertEqual(first.payload.0, 13)
            XCTAssertEqual(first.payload.1, 5)
            XCTAssertEqual(first.bytes.offset, 0)
            XCTAssertEqual(first.bytes.len, 5)

            let validated = try EventBatchReader.validate(batch: batch)
            XCTAssertEqual(validated.key(at: 0), "Enter")
            XCTAssertEqual(validated.code(at: 0), "Enter")
            XCTAssertEqual(validated.customName(at: 1), "ev")
            XCTAssertEqual(Array(validated.customData(at: 1)), [0xAB, 0xCD])

            let owned = try EventBatchBuilder.copy(from: batch)
            XCTAssertEqual(owned.toWireBytes(), b.toWireBytes())
        }
    }

    func testStrictWireAndIngressRejections() throws {
        // Truncated header & bad magic & NVEV v1 rejected.
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire([0x4E, 0x56, 0x45])) {
            XCTAssertEqual($0 as? EventValidationError, .truncated)
        }
        let badMagic = [UInt8](repeating: 0, count: 14)
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(badMagic)) {
            XCTAssertEqual($0 as? EventValidationError, .invalidMagic(0))
        }
        var v1: [UInt8] = [0x4E, 0x56, 0x45, 0x56, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(v1)) {
            XCTAssertEqual($0 as? EventValidationError, .unsupportedVersion(1))
        }
        v1[4] = 2
        // Trailing bytes rejected.
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(v1 + [0x00])) {
            XCTAssertEqual($0 as? EventValidationError, .trailingBytes(1))
        }

        // Helper to mutate one 64-byte wire record and test decode rejection.
        func makeSingleRecordWire(flags: UInt32, p0: UInt32 = 0, p1: UInt32 = 0, p2: UInt32 = 0,
                                  off: UInt32 = 0, len: UInt32 = 0, arena: [UInt8] = []) -> [UInt8] {
            var out: [UInt8] = [0x4E, 0x56, 0x45, 0x56, 0x02, 0x00, 1, 0, 0, 0]
            withUnsafeBytes(of: UInt32(arena.count).littleEndian) { out.append(contentsOf: $0) }
            var rec = [UInt8](repeating: 0, count: 64)
            rec.withUnsafeMutableBytes { buf in
                buf.storeBytes(of: flags.littleEndian, toByteOffset: 28, as: UInt32.self)
                buf.storeBytes(of: p0.littleEndian, toByteOffset: 32, as: UInt32.self)
                buf.storeBytes(of: p1.littleEndian, toByteOffset: 36, as: UInt32.self)
                buf.storeBytes(of: p2.littleEndian, toByteOffset: 40, as: UInt32.self)
                buf.storeBytes(of: off.littleEndian, toByteOffset: 56, as: UInt32.self)
                buf.storeBytes(of: len.littleEndian, toByteOffset: 60, as: UInt32.self)
            }
            out.append(contentsOf: rec)
            out.append(contentsOf: arena)
            return out
        }

        // Span out of bounds (including u32 wraparound).
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(makeSingleRecordWire(flags: 0, off: 0xFFFF_FFFF, len: 2, arena: [1, 2]))) {
            XCTAssertEqual($0 as? EventValidationError, .spanOutOfBounds(index: 0, offset: 0xFFFF_FFFF, len: 2, arenaLen: 2))
        }
        // Conflicting framing flags.
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(
            makeSingleRecordWire(flags: nveEventFlagInlineKeyUtf8 | nveEventFlagKeySplitBytes)
        )) { XCTAssertEqual($0 as? EventValidationError, .conflictingFlags(index: 0)) }
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(
            makeSingleRecordWire(flags: nveEventFlagCustomNamed | nveEventFlagInlineKeyUtf8)
        )) { XCTAssertEqual($0 as? EventValidationError, .conflictingFlags(index: 0)) }
        // Invalid inline key length (> 16) and invalid inline UTF-8.
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(
            makeSingleRecordWire(flags: nveEventFlagInlineKeyUtf8, p1: 17)
        )) { XCTAssertEqual($0 as? EventValidationError, .invalidInlineKeyLen(index: 0, len: 17)) }
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(
            makeSingleRecordWire(flags: nveEventFlagInlineKeyUtf8, p1: 2, p2: 0x0000_AFC0)
        )) { XCTAssertEqual($0 as? EventValidationError, .invalidUtf8(index: 0)) }
        // Split key splitting a multi-byte UTF-8 character (`é` = [0xC3, 0xA9]).
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(
            makeSingleRecordWire(flags: nveEventFlagKeySplitBytes | nveEventFlagUtf8Bytes, p1: 1, off: 0, len: 2, arena: [0xC3, 0xA9])
        )) { XCTAssertEqual($0 as? EventValidationError, .invalidUtf8(index: 0)) }
        // Custom name exceeding slice and invalid UTF-8 in custom name.
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(
            makeSingleRecordWire(flags: nveEventFlagCustomNamed, p0: 5, off: 0, len: 2, arena: [0x61, 0x62])
        )) { XCTAssertEqual($0 as? EventValidationError, .invalidCustomNameLen(index: 0, nameLen: 5, bytesLen: 2)) }
        XCTAssertThrowsError(try EventBatchBuilder.decodeWire(
            makeSingleRecordWire(flags: nveEventFlagCustomNamed, p0: 2, off: 0, len: 3, arena: [0xC0, 0xAF, 0x00])
        )) { XCTAssertEqual($0 as? EventValidationError, .invalidUtf8(index: 0)) }
        // Null pointers with non-zero counts in C NveEventBatch.
        XCTAssertThrowsError(try EventBatchReader.validate(
            batch: CNveAbi.NveEventBatch(events: nil, bytes: nil, event_count: 1, bytes_len: 0)
        )) { XCTAssertEqual($0 as? EventValidationError, .nullEventsPointer) }
        XCTAssertThrowsError(try EventBatchReader.validate(
            batch: CNveAbi.NveEventBatch(events: nil, bytes: nil, event_count: 0, bytes_len: 1)
        )) { XCTAssertEqual($0 as? EventValidationError, .nullBytesPointer) }

        // SurfaceId / NodeId with index 0 and non-zero generation canonicalize to 0.
        var canonical = EventBatchBuilder()
        canonical.pushFocus(surface: 0x0700_0000, node: 0x0900_0000, timestamp: 1)
        canonical.withValidated { r in
            XCTAssertEqual(r.record(at: 0)?.surface, 0)
            XCTAssertEqual(r.record(at: 0)?.node, 0)
        }
    }

    func testWarmEventBuildWireAndValidateDoNotAllocate() throws {
        #if DEBUG
        try XCTSkipIf(true, "allocation counts are only meaningful in optimized builds (swift test -c release)")
        #endif
        var builder = EventBatchBuilder(eventCapacity: 8, byteCapacity: 256)
        var wire: [UInt8] = []
        wire.reserveCapacity(1024)
        let customPayload: [UInt8] = [0x01, 0x02, 0x03, 0x04]
        var checksum: UInt64 = 0

        func runRound() throws {
            builder.clear()
            builder.pushPress(surface: 1, node: 2, timestamp: 10, x: 1, y: 2)
            builder.pushKeyDown(surface: 1, node: 2, timestamp: 11, key: "Enter", code: "Enter", flags: .modMeta, keyCode: 13)
            builder.pushTextChange(surface: 1, node: 3, timestamp: 12, text: "steady é")
            builder.pushScroll(surface: 1, node: 4, timestamp: 13, x: 0, y: 50, contentWidth: 320, contentHeight: 900)
            builder.pushCustom(surface: 1, node: 5, timestamp: 14, name: "ping", data: customPayload)
            wire.removeAll(keepingCapacity: true)
            try builder.tryEncodeWire(into: &wire)
            try EventBatchReader.withValidatedWire(wire) { r in
                checksum &+= UInt64(r.count) &+ UInt64(r.keyBytes(at: 1)?.count ?? 0) &+ UInt64(r.customData(at: 4).count)
            }
            try builder.withRawBatch { batch in
                let r = try EventBatchReader.validate(batch: batch)
                checksum &+= UInt64(r.record(at: 0)?.timestamp ?? 0)
            }
        }

        try runRound()
        try runRound()
        let count = try XCTUnwrap(try allocations {
            for _ in 0..<200 { try runRound() }
        })
        XCTAssertGreaterThan(checksum, 0)
        XCTAssertEqual(count, 0, "warm EventBatchBuilder / NVEV v2 / C NveEventBatch path allocated \(count) times")
    }
}
