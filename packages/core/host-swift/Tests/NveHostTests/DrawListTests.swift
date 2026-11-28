import NveHost
import XCTest

private func packWords(_ words: [UInt32]) -> [UInt8] {
    var out: [UInt8] = []
    out.reserveCapacity(words.count * 4)
    for w in words {
        let le = w.littleEndian
        withUnsafeBytes(of: le) { out.append(contentsOf: $0) }
    }
    return out
}

private func packRgba(_ r: UInt8, _ g: UInt8, _ b: UInt8, _ a: UInt8 = 255) -> UInt32 {
    UInt32(r) | (UInt32(g) << 8) | (UInt32(b) << 16) | (UInt32(a) << 24)
}

private struct RecordingDrawListSink: NveDrawListSink {
    var ops: [String] = []
    var points: [NveDrawListPoint] = []

    mutating func onClear(color: UInt32) { ops.append("clear:\(color)") }
    mutating func onFillRect(x: Float, y: Float, w: Float, h: Float, color: UInt32) {
        ops.append("rect:\(x),\(y),\(w),\(h),\(color)")
    }
    mutating func onFillRoundedRect(x: Float, y: Float, w: Float, h: Float, radius: Float, color: UInt32) {
        ops.append("rrect:\(x),\(y),\(w),\(h),\(radius),\(color)")
    }
    mutating func onFillCircle(cx: Float, cy: Float, radius: Float, color: UInt32) {
        ops.append("circle:\(cx),\(cy),\(radius),\(color)")
    }
    mutating func onStrokeLine(x0: Float, y0: Float, x1: Float, y1: Float, width: Float, color: UInt32) {
        ops.append("line:\(x0),\(y0),\(x1),\(y1),\(width),\(color)")
    }
    mutating func onStrokePolyline(width: Float, color: UInt32, points slice: NveDrawListPointSlice) {
        ops.append("poly:\(slice.count),\(width),\(color)")
        for i in 0 ..< slice.count {
            points.append(slice[i])
        }
    }
    mutating func onClipRect(x: Float, y: Float, w: Float, h: Float) {
        ops.append("clip:\(x),\(y),\(w),\(h)")
    }
    mutating func onRestoreClip() { ops.append("restore") }
}

final class DrawListTests: XCTestCase {
    private func buildAll8Ops() -> [UInt8] {
        let c1 = packRgba(1, 2, 3, 4)
        let white = packRgba(255, 255, 255)
        return packWords([
            8,
            NveDrawListOpcode.clear.rawValue, c1,
            NveDrawListOpcode.fillRect.rawValue,
            Float(1).bitPattern, Float(2).bitPattern, Float(3).bitPattern, Float(4).bitPattern, white,
            NveDrawListOpcode.fillRoundedRect.rawValue,
            Float(0).bitPattern, Float(0).bitPattern, Float(10).bitPattern, Float(10).bitPattern, Float(2.5).bitPattern, white,
            NveDrawListOpcode.fillCircle.rawValue,
            Float(5).bitPattern, Float(6).bitPattern, Float(3).bitPattern, white,
            NveDrawListOpcode.strokeLine.rawValue,
            Float(0).bitPattern, Float(0).bitPattern, Float(3).bitPattern, Float(4).bitPattern, Float(2).bitPattern, white,
            NveDrawListOpcode.strokePolyline.rawValue,
            Float(1.5).bitPattern, white, 3,
            Float(0).bitPattern, Float(0).bitPattern,
            Float(5).bitPattern, Float(7.5).bitPattern,
            Float(-2).bitPattern, Float(12).bitPattern,
            NveDrawListOpcode.clipRect.rawValue,
            Float(1).bitPattern, Float(2).bitPattern, Float(30).bitPattern, Float(40).bitPattern,
            NveDrawListOpcode.restoreClip.rawValue,
        ])
    }

    func testGoldenAll8OpsAndUnalignedSlice() {
        XCTAssertEqual(NveDrawList.kind, Key64.of("nve.res.draw_list"))
        XCTAssertEqual(NveDrawList.headerSize, 4)
        XCTAssertEqual(NveDrawList.maxDrawOps, 65_536)
        XCTAssertEqual(NveDrawList.maxPolylinePoints, 16_384)
        XCTAssertEqual(NveDrawList.maxClipDepth, 64)

        let all8 = buildAll8Ops()
        XCTAssertEqual(all8.count, 44 * 4)

        var padded = [UInt8](repeating: 0xAA, count: all8.count + 5)
        for i in 0 ..< all8.count { padded[i + 3] = all8[i] }

        var sink = RecordingDrawListSink()
        let err = padded.withUnsafeBytes { raw -> NveDrawListError? in
            let sub = UnsafeRawBufferPointer(rebasing: raw[3 ..< (3 + all8.count)])
            return NveDrawList.decode(sub, into: &sink)
        }
        XCTAssertNil(err)
        XCTAssertEqual(sink.ops.count, 8)
        XCTAssertEqual(sink.points, [
            NveDrawListPoint(x: 0, y: 0),
            NveDrawListPoint(x: 5, y: 7.5),
            NveDrawListPoint(x: -2, y: 12),
        ])

        XCTAssertNil(NveDrawList.validate(packWords([0])))
        XCTAssertNil(NveDrawList.validate(packWords([1, NveDrawListOpcode.strokePolyline.rawValue, Float(-0.0).bitPattern, 0, 0])))
        XCTAssertNil(NveDrawList.validate(packWords([1, NveDrawListOpcode.fillRect.rawValue, Float(-1).bitPattern, Float(-2).bitPattern, Float(-3).bitPattern, Float(-4).bitPattern, 0])))
    }

    func testRejectsMalformedAndNegativePayloadsWithoutCallbacks() {
        let all8 = buildAll8Ops()
        for cut in 0 ..< all8.count {
            XCTAssertEqual(NveDrawList.validate(Array(all8[..<cut])), .truncated, "cut=\(cut)")
        }
        for extra in 1 ... 4 {
            var withExtra = all8
            withExtra.append(contentsOf: [UInt8](repeating: 0, count: extra))
            XCTAssertEqual(NveDrawList.validate(withExtra), .trailingBytes(extra))
        }
        XCTAssertEqual(NveDrawList.validate(packWords([NveDrawList.maxDrawOps + 1])), .opCountExceedsLimit(NveDrawList.maxDrawOps + 1))
        XCTAssertEqual(NveDrawList.validate(packWords([UInt32.max])), .opCountExceedsLimit(UInt32.max))
        XCTAssertEqual(NveDrawList.validate(packWords([1, 0])), .unknownOpcode(0))
        XCTAssertEqual(NveDrawList.validate(packWords([1, 9])), .unknownOpcode(9))
        XCTAssertEqual(NveDrawList.validate(packWords([1, 0x0001_0001])), .unknownOpcode(0x0001_0001))
        XCTAssertEqual(
            NveDrawList.validate(packWords([1, NveDrawListOpcode.strokePolyline.rawValue, Float(1).bitPattern, 0, NveDrawList.maxPolylinePoints + 1])),
            .sliceLenExceedsLimit(NveDrawList.maxPolylinePoints + 1)
        )
        XCTAssertEqual(
            NveDrawList.validate(packWords([1, NveDrawListOpcode.fillRect.rawValue, Float.nan.bitPattern, 0, 0, 0, 0])),
            .nonFiniteFloat
        )
        XCTAssertEqual(
            NveDrawList.validate(packWords([1, NveDrawListOpcode.fillCircle.rawValue, 0, 0, (-Float.infinity).bitPattern, 0])),
            .nonFiniteFloat
        )
        XCTAssertEqual(
            NveDrawList.validate(packWords([1, NveDrawListOpcode.fillRoundedRect.rawValue, 0, 0, Float(10).bitPattern, Float(10).bitPattern, Float(-0.5).bitPattern, 0])),
            .negativeFloat
        )
        XCTAssertEqual(
            NveDrawList.validate(packWords([1, NveDrawListOpcode.fillCircle.rawValue, 0, 0, Float(-1).bitPattern, 0])),
            .negativeFloat
        )
        XCTAssertEqual(
            NveDrawList.validate(packWords([1, NveDrawListOpcode.strokeLine.rawValue, 0, 0, Float(1).bitPattern, Float(1).bitPattern, Float(-0.25).bitPattern, 0])),
            .negativeFloat
        )
        XCTAssertEqual(
            NveDrawList.validate(packWords([1, NveDrawListOpcode.strokePolyline.rawValue, Float(-2).bitPattern, 0, 0])),
            .negativeFloat
        )

        var zeroSink = RecordingDrawListSink()
        let bad2 = packWords([2, NveDrawListOpcode.clear.rawValue, 1, NveDrawListOpcode.fillCircle.rawValue, 0, 0, Float(-1).bitPattern, 0])
        XCTAssertEqual(NveDrawList.decode(bad2, into: &zeroSink), .negativeFloat)
        XCTAssertTrue(zeroSink.ops.isEmpty)
    }

    func testClipBalance66NestedClipsAnd2Restores() {
        var words: [UInt32] = [NveDrawList.maxClipDepth + 2 + 2 + 1]
        for _ in 0 ..< (NveDrawList.maxClipDepth + 2) {
            words.append(contentsOf: [
                NveDrawListOpcode.clipRect.rawValue,
                0, 0, Float(10).bitPattern, Float(10).bitPattern,
            ])
        }
        words.append(NveDrawListOpcode.restoreClip.rawValue)
        words.append(NveDrawListOpcode.restoreClip.rawValue)
        words.append(NveDrawListOpcode.clear.rawValue)
        words.append(7)

        var sink = RecordingDrawListSink()
        XCTAssertNil(NveDrawList.decode(packWords(words), into: &sink))
        let cap = Int(NveDrawList.maxClipDepth)
        XCTAssertEqual(sink.ops.count, cap * 2 + 1)
        XCTAssertEqual(sink.ops[cap], "clear:7")
        XCTAssertTrue(sink.ops[..<cap].allSatisfy { $0.hasPrefix("clip:") })
        XCTAssertTrue(sink.ops[(cap + 1)...].allSatisfy { $0 == "restore" })

        // Unmatched leading restore ignored, 3rd restore pops clip #64 before clear.
        var words3: [UInt32] = [1 + NveDrawList.maxClipDepth + 2 + 3 + 1]
        words3.append(NveDrawListOpcode.restoreClip.rawValue)
        for _ in 0 ..< (NveDrawList.maxClipDepth + 2) {
            words3.append(contentsOf: [
                NveDrawListOpcode.clipRect.rawValue,
                0, 0, Float(10).bitPattern, Float(10).bitPattern,
            ])
        }
        words3.append(NveDrawListOpcode.restoreClip.rawValue)
        words3.append(NveDrawListOpcode.restoreClip.rawValue)
        words3.append(NveDrawListOpcode.restoreClip.rawValue)
        words3.append(NveDrawListOpcode.clear.rawValue)
        words3.append(9)

        var sink3 = RecordingDrawListSink()
        XCTAssertNil(NveDrawList.decode(packWords(words3), into: &sink3))
        XCTAssertEqual(sink3.ops.count, cap * 2 + 1)
        XCTAssertEqual(sink3.ops[cap], "restore")
        XCTAssertEqual(sink3.ops[cap + 1], "clear:9")
    }
}
