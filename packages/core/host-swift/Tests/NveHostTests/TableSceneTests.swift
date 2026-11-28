import CNveHostTestABI
import NveHost
import XCTest

// The C-imported structs satisfy the runtime protocols with empty extensions — exactly what the
// generated `NveTableView` / `NveNodeRow` / `NveLayoutRect` / `NveSceneView` conformances look like.
extension NveTestTableView: TableViewABI {}
extension NveTestNodeRow: NodeRowABI {}
extension NveTestLayoutRect: LayoutRectABI {}
extension NveTestScrollOffset: ScrollOffsetABI {}
extension NveTestSceneView: SceneViewABI {}

final class TableSceneTests: XCTestCase {
    private let schema = Key64(0xF00D_FACE_C0FF_EE11)
    // Fixture specs are statically valid; `try!` documents that, and a typo fails loudly.
    private let opacity = try! FieldSpec(word: 0, shift: 0, bits: 8, kind: .u32)
    private let z = try! FieldSpec(word: 0, shift: 8, bits: 12, kind: .i32)
    private let visible = try! FieldSpec(word: 0, shift: 20, bits: 1, kind: .bool)
    private let width = try! FieldSpec(word: 1, kind: .length)
    private let color = try! FieldSpec(word: 2, kind: .color)
    private let label = try! FieldSpec(word: 3, kind: .str)
    private let scale = try! FieldSpec(word: 5, kind: .f32)

    private func scene() throws -> SceneReader<NveTestSceneView> {
        try SceneReader.open(nve_test_scene(), expectedSchema: schema)
    }

    func testFieldSpecExtraction() throws {
        XCTAssertEqual(try FieldSpec(word: 0, shift: 8, bits: 8, kind: .u32).extract(0xABCD_1234), 0x12)
        XCTAssertEqual(try FieldSpec(word: 0, shift: 0, bits: 32, kind: .u32).extract(0xABCD_1234), 0xABCD_1234)
        XCTAssertEqual(try FieldSpec(word: 0, shift: 0, bits: 3, kind: .i32).extractSigned(7), -1)
        XCTAssertEqual(try FieldSpec(word: 0, shift: 0, bits: 3, kind: .i32).extractSigned(4), -4)
        XCTAssertEqual(try FieldSpec(word: 0, shift: 0, bits: 32, kind: .i32).extractSigned(0x8000_0000), .min)
        XCTAssertEqual(try FieldSpec(word: 0, shift: 31, bits: 1, kind: .i32).extractSigned(0x8000_0000), -1)
        XCTAssertTrue(label.fits(words: 5))
        XCTAssertFalse(label.fits(words: 4))
        // Malformed specs cannot exist: construction throws instead of trapping in a later shift.
        for (word, shift, bits, kind) in [(0, 24, 16, FieldKind.u32), (0, 0, 0, .u32), (0, 0, 33, .i32),
                                          (0, 31, 2, .i32), (32, 0, 32, .u32), (31, 0, 32, .str), (255, 255, 255, .u32)] {
            XCTAssertThrowsError(try FieldSpec(word: UInt8(word), shift: UInt8(shift), bits: UInt8(bits), kind: kind))
        }
        XCTAssertEqual(LengthValue(word: 0x3F80_0000), LengthValue(unit: .points, value: 1))
        XCTAssertEqual(LengthValue(word: 0x4248_0001), LengthValue(unit: .percent, value: 50))
        XCTAssertEqual(LengthValue(word: 2), LengthValue(unit: .auto, value: 0))
        XCTAssertEqual(LengthValue(word: 0x3F80_0003), LengthValue(unit: .auto, value: 0))
        XCTAssertEqual(Rgba(packed: 0x8040_2010).r, 0x10)
        XCTAssertEqual(Rgba(packed: 0x8040_2010).a, 0x80)
    }

    func testTableReaderOverCMemory() throws {
        let s = try scene()
        let t = try XCTUnwrap(s.table(key: Key64.of("nve.visual")))
        XCTAssertEqual(t.words, 6)
        XCTAssertEqual(t.rowCount, 2)
        XCTAssertFalse(t.isSparse)
        XCTAssertEqual(t.u32(0, opacity), 200)
        XCTAssertEqual(t.i32(0, z), -3)
        XCTAssertEqual(t.i32(1, z), 2047)
        XCTAssertEqual(t.bool(0, visible), true)
        XCTAssertEqual(t.bool(1, visible), false)
        XCTAssertEqual(t.length(0, width), LengthValue(unit: .percent, value: 50))
        XCTAssertEqual(t.length(1, width)?.unit, .auto)
        XCTAssertEqual(t.color(0, color)?.g, 0x20)
        XCTAssertEqual(t.color(0, color)?.b, 0x40)
        XCTAssertEqual(t.str(0, label), StrSpan(offset: 0, len: 3))
        XCTAssertEqual(t.f32(0, scale), 1.5)
        XCTAssertEqual(t.f32(1, scale)?.sign, .minus)
        XCTAssertNil(t.u32(2, opacity))
        XCTAssertNil(t.word(0, 6))
        XCTAssertNil(t.str(0, try FieldSpec(word: 5, kind: .str)))
        XCTAssertEqual(t.row(of: 1), 1)
        XCTAssertNil(t.row(of: 2))
        XCTAssertEqual(t.row(1).count, 6)
        XCTAssertTrue(t.row(9).isEmpty)
        XCTAssertNil(s.table(key: Key64(1)))
        XCTAssertNil(s.table(1))
    }

    func testHostileViewsNeverTrap() throws {
        // `words` is checked before any multiplication: no overflow trap on 64- or 32-bit hosts.
        var huge = NveTestTableView()
        huge.words = 0x8000_0001
        huge.row_count = .max
        XCTAssertNil(TableReader(view: huge))
        huge.words = 32
        huge.dirty_count = .max
        XCTAssertNil(TableReader(view: huge))  // null pointers with huge counts
        // A non-null dirty pointer with a count past the address space is refused, not spanned.
        let rows: [UInt32] = [0]
        rows.withUnsafeBufferPointer { rp in
            var dirty = NveTestTableView()
            dirty.words = 1
            dirty.row_count = 1
            dirty.rows = rp.baseAddress
            dirty.dirty_rows = rp.baseAddress
            dirty.dirty_count = .max
            let fits = UInt64(UInt32.max) <= UInt64(Int.max / MemoryLayout<UInt32>.stride)
            XCTAssertEqual(TableReader(view: dirty) != nil, fits)
        }
        let base = try XCTUnwrap(nve_test_scene()).pointee
        var bad = try XCTUnwrap(base.tables).pointee
        bad.words = 0x8000_0001
        bad.row_count = .max
        try withUnsafePointer(to: bad) { tp in
            var v = base
            v.tables = tp
            XCTAssertThrowsError(try SceneReader.open(v, expectedSchema: schema)) {
                XCTAssertEqual($0 as? SceneError, .invalidTable(index: 0))
            }
            _ = try XCTUnwrap(Optional(tp))
        }
        let s = try scene()
        XCTAssertNil(s.text(offset: .max, len: .max))
        XCTAssertNil(s.text(offset: 1, len: .max))
        XCTAssertNil(s.children(offset: .max, count: .max))
        XCTAssertNil(s.children(offset: 2, count: .max))
        // A field past this table's 6-word row reads as nil (spec itself is valid for 32 words).
        let t = try XCTUnwrap(s.table(0))
        XCTAssertNil(t.row(of: .max))  // dense: compared as UInt64 (would trap via Int on arm64_32)
        XCTAssertNil(t.row(of: 0x8000_0000))
        XCTAssertNil(t.str(0, try FieldSpec(word: 3, kind: .u32)))  // not a string spec
        XCTAssertNil(t.str(0, try FieldSpec(word: 30, kind: .str)))  // past this 6-word row
        XCTAssertNotNil(t.str(0, label))
        XCTAssertNil(t.bits(0, try FieldSpec(word: 6, kind: .u32)))
        XCTAssertNil(t.i32(0, try FieldSpec(word: 31, kind: .i32)))
    }

    func testAddressSpaceBoundsWith32BitLimits() {
        // Parameterized, so this 64-bit run checks what an arm64_32 host would accept.
        let limit32 = Int(Int32.max)
        XCTAssertNil(TableReader.fitsAddressSpace(UInt64(UInt32.max), stride: 4, limit: limit32))
        XCTAssertNil(TableReader.fitsAddressSpace(0x2000_0000, stride: 4, limit: limit32))
        XCTAssertEqual(TableReader.fitsAddressSpace(0x1FFF_FFFF, stride: 4, limit: limit32), 0x1FFF_FFFF)
        XCTAssertNil(TableReader.fitsAddressSpace(UInt64(UInt32.max), stride: 64, limit: limit32))
        XCTAssertNil(TableReader.fitsAddressSpace(32 * UInt64(UInt32.max), stride: 4, limit: limit32))
        XCTAssertNil(TableReader.fitsAddressSpace(1, stride: 0))
        XCTAssertNil(TableReader.fitsAddressSpace(UInt64.max, stride: 1))
        XCTAssertEqual(TableReader.fitsAddressSpace(UInt64(UInt32.max), stride: 4), Int(exactly: UInt32.max)
            .flatMap { MemoryLayout<Int>.size == 8 ? $0 : nil })
        XCTAssertNil(TableReader.denseRow(.max, rowCount: limit32))
        XCTAssertNil(TableReader.denseRow(0x8000_0000, rowCount: limit32))
        XCTAssertEqual(TableReader.denseRow(0x7FFF_FFFE, rowCount: limit32), 0x7FFF_FFFE)
        XCTAssertNil(TableReader.denseRow(0, rowCount: 0))
        XCTAssertNil(TableReader.denseRow(0, rowCount: -1))
    }

    func testBomIsTextNotStripped() throws {
        let text: [UInt8] = [0xEF, 0xBB, 0xBF, 0xEF, 0xBB, 0xBF, 0x78]
        let base = try XCTUnwrap(nve_test_scene()).pointee
        try text.withUnsafeBufferPointer { bp in
            var v = base
            v.text_slab = bp.baseAddress
            v.text_slab_len = 7
            let s = try SceneReader.open(v, expectedSchema: schema)
            XCTAssertEqual(s.text(offset: 0, len: 7), "\u{FEFF}\u{FEFF}x")
            XCTAssertEqual(s.text(offset: 0, len: 3), "\u{FEFF}")
            _ = try XCTUnwrap(bp.baseAddress)
        }
    }

    func testSparseAndInvalidTables() throws {
        let rows: [UInt32] = [1, 2, 3, 4]
        let owners: [UInt32] = [0x0100_0007, 0x0200_0003]
        rows.withUnsafeBufferPointer { rp in
            owners.withUnsafeBufferPointer { op in
                // Sparse rows without a published row index could never resolve: rejected.
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 2, rows: rp, owners: op))
                XCTAssertNotNil(TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 0, rows: rp, owners: op))
                // Row index: page 0 maps node 7 -> row 0 and node 3 -> row 1 (`row + 1`).
                var pages = [UInt32](repeating: 0, count: Int(nveRowIndexPageLen))
                pages[7] = 1
                pages[3] = 2
                let dir: [UInt32] = [0, .max]
                dir.withUnsafeBufferPointer { dp in
                    pages.withUnsafeBufferPointer { pp in
                        let ix = TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 2, rows: rp, owners: op,
                                             rowIndexDir: dp, rowIndexPages: pp)
                        XCTAssertEqual(ix?.isSparse, true)
                        XCTAssertEqual(ix?.owner(0), 0x0100_0007)
                        XCTAssertEqual(ix?.row(ofNode: 3), 1)
                        XCTAssertEqual(ix?.row(ofNode: 7), 0)
                        XCTAssertNil(ix?.row(ofNode: 4))  // no row
                        XCTAssertNil(ix?.row(ofNode: 300))  // page `.max`
                        XCTAssertNil(ix?.row(ofNode: 600))  // past the directory
                        XCTAssertNil(ix?.row(ofNode: -1))
                        XCTAssertEqual(ix?.row(of: 0x0200_0003), 1)
                        XCTAssertNil(ix?.row(of: 0x0100_0003))  // stale generation of slot 3
                        XCTAssertNil(ix?.row(of: 1))
                        // A row past `rowCount` misses; a partial page alone is no row index.
                        let short = TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 1, rows: rp, owners: op,
                                                rowIndexDir: dp, rowIndexPages: pp)
                        XCTAssertNil(short?.row(ofNode: 3))
                        XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 2, rows: rp, owners: op,
                                                 rowIndexDir: dp, rowIndexPages: UnsafeBufferPointer(rebasing: pp[0..<255])))
                    }
                    // A partial tail page after whole ones is dropped (as in C++): unreachable.
                    (pages + [9, 9, 9]).withUnsafeBufferPointer { pp in
                        let tail = TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 2, rows: rp, owners: op,
                                               rowIndexDir: dp, rowIndexPages: pp)
                        XCTAssertEqual(tail?.rowIndexPages.count, Int(nveRowIndexPageLen))
                        XCTAssertEqual(tail?.row(ofNode: 3), 1)
                    }
                }
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 3, rows: rp))
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 33, rowCount: 0, rows: rp))
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 0, rowCount: 0, rows: rp))
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 2, rows: rp,
                                         owners: UnsafeBufferPointer(rebasing: op[0..<1])))
            }
        }
        var nullRows = NveTestTableView()
        nullRows.words = 6
        nullRows.row_count = 2
        XCTAssertNil(TableReader(view: nullRows))
    }

    func testWide64GridLineAndAllOptEncodings() throws {
        let u64Spec = try FieldSpec(word: 0, kind: .u64)
        let i64Spec = try FieldSpec(word: 2, kind: .i64)
        let f64Spec = try FieldSpec(word: 4, kind: .f64)
        let gridStart = try FieldSpec(word: 6, shift: 0, bits: 16, kind: .gridLine)
        let gridEnd = try FieldSpec(word: 6, shift: 16, bits: 16, kind: .gridLine)
        let optGrid = try FieldSpec(word: 7, shift: 0, bits: 16, kind: .gridLine, opt: .tagXor)
        let optLen = try FieldSpec(word: 8, shift: 0, bits: 32, kind: .length, opt: .tagXor)
        let optZero = try FieldSpec(word: 9, shift: 0, bits: 16, kind: .u32, opt: .zeroNiche)
        let optBit0Signed = try FieldSpec(word: 10, shift: 1, bits: 17, kind: .i32, opt: .bit0Present)
        let optBoolPlus = try FieldSpec(word: 10, shift: 18, bits: 2, kind: .enumeration, opt: .plusOne)
        let optWideF32 = try FieldSpec(word: 10, shift: 20, bits: 1, kind: .f32, opt: .wideBit, valWord: 11)
        let optWideColor = try FieldSpec(word: 10, shift: 21, bits: 1, kind: .color, opt: .wideBit, valWord: 12)

        // Row 0: all populated; Row 1: all optionals absent + edge GridLine values.
        let piBits = Double.pi.bitPattern
        let negI64 = UInt64(bitPattern: -0x0123_4567_89AB_CDEF)
        let gridLineNeg1: UInt32 = (1 << 14) | 0x3FFF          // Line(-1)
        let gridSpan3: UInt32 = (2 << 14) | 3                  // Span(3)
        let gridLineMin: UInt32 = (1 << 14) | 0x2000           // Line(-8192)
        let gridLineMax: UInt32 = (1 << 14) | 0x1FFF           // Line(8191)
        let optGridSome: UInt32 = gridLineNeg1 ^ 0xC000
        let optLenPoints: UInt32 = 0x4180_0000 ^ 0b11          // Some(points(16.0))
        let optTabNeg5: UInt32 = (UInt32(UInt16(bitPattern: -5)) << 1) | 1

        let words0: [UInt32] = [
            0xDEAD_BEEF, 0xCAFE_BABE,
            UInt32(truncatingIfNeeded: negI64), UInt32(truncatingIfNeeded: negI64 >> 32),
            UInt32(truncatingIfNeeded: piBits), UInt32(truncatingIfNeeded: piBits >> 32),
            gridLineNeg1 | (gridSpan3 << 16),
            optGridSome,
            optLenPoints,
            7,
            (optTabNeg5 << 1) | (2 << 18) | (1 << 20) | (1 << 21),
            Float(1.75).bitPattern,
            0xFF11_2233,
        ]
        let words1: [UInt32] = [
            0, 0, 0, 0, 0, 0,
            gridLineMin | (gridLineMax << 16),
            0x0042,          // tag bits 14..15 == 00 -> None
            0x1234_5600,     // tag bits 0..1 == 00 -> None
            0,               // zeroNiche -> None
            (3 << 18),       // bit0Present=0 -> None; plusOne bool=3 -> None; wideBit=0 -> None
            Float(99.0).bitPattern,
            0xFFFF_FFFF,
        ]
        let allRows = words0 + words1
        try allRows.withUnsafeBufferPointer { buf in
            let t = try XCTUnwrap(TableReader(key: Key64(42), slot: 0, words: 13, rowCount: 2, rows: buf))
            XCTAssertEqual(t.u64(0, u64Spec), 0xCAFE_BABE_DEAD_BEEF)
            XCTAssertEqual(t.i64(0, i64Spec), -0x0123_4567_89AB_CDEF)
            XCTAssertEqual(t.f64(0, f64Spec), Double.pi)
            XCTAssertEqual(t.gridLine(0, gridStart), .line(-1))
            XCTAssertEqual(t.gridLine(0, gridEnd), .span(3))
            XCTAssertEqual(t.gridLine(1, gridStart), .line(GridLineValue.minLine))
            XCTAssertEqual(t.gridLine(1, gridEnd), .line(GridLineValue.maxLine))
            XCTAssertEqual(t.gridLine(0, optGrid), .line(-1))
            XCTAssertNil(t.gridLine(1, optGrid))
            XCTAssertEqual(t.length(0, optLen), LengthValue(unit: .points, value: 16.0))
            XCTAssertNil(t.length(1, optLen))
            XCTAssertEqual(t.u32(0, optZero), 7)
            XCTAssertNil(t.u32(1, optZero))
            XCTAssertEqual(t.i32(0, optBit0Signed), -5)
            XCTAssertNil(t.i32(1, optBit0Signed))
            XCTAssertEqual(t.bool(0, optBoolPlus), true)
            XCTAssertNil(t.bool(1, optBoolPlus))
            XCTAssertEqual(t.f32(0, optWideF32), 1.75)
            XCTAssertNil(t.f32(1, optWideF32))
            XCTAssertEqual(t.color(0, optWideColor), Rgba(packed: 0xFF11_2233))
            XCTAssertNil(t.color(1, optWideColor))
            XCTAssertTrue(t.isPresent(0, optWideF32))
            XCTAssertFalse(t.isPresent(1, optWideF32))
            // Non-64-bit / non-gridLine specs return nil on mismatched accessors.
            XCTAssertNil(t.u64(0, optLen))
            XCTAssertNil(t.gridLine(0, optLen))
        }

        // Invalid FieldSpec combinations must throw at init.
        XCTAssertThrowsError(try FieldSpec(word: 31, kind: .u64))
        XCTAssertThrowsError(try FieldSpec(word: 0, shift: 1, bits: 32, kind: .f64))
        XCTAssertThrowsError(try FieldSpec(word: 0, shift: 0, bits: 2, kind: .f32, opt: .wideBit, valWord: 1))
        XCTAssertThrowsError(try FieldSpec(word: 0, shift: 0, bits: 1, kind: .f32, opt: .wideBit, valWord: 0))
        XCTAssertThrowsError(try FieldSpec(word: 0, shift: 0, bits: 1, kind: .f32, opt: .wideBit, valWord: noValWord))
        XCTAssertThrowsError(try FieldSpec(word: 0, shift: 0, bits: 1, kind: .u32, opt: .bit0Present))
        XCTAssertThrowsError(try FieldSpec(word: 0, shift: 0, bits: 16, kind: .length, opt: .tagXor))
        XCTAssertThrowsError(try FieldSpec(word: 0, shift: 0, bits: 32, kind: .u32, opt: .required, explicitBit: 0, inherits: true))
        XCTAssertThrowsError(try FieldSpec(word: 1, shift: 0, bits: 32, kind: .u32, opt: .required, explicitBit: noValWord, inherits: true))
    }

    func testEventSetAndInheritedTypographyResolution() throws {
        // Build a 3-node chain: root (1) -> parent (2) -> child (3), with Typography (slot 4)
        // and Interaction (slot 5) attached.
        let typoSlot: UInt32 = 4
        let interSlot: UInt32 = 5
        var typoRowRoot = NveTables.Typography.defaults
        typoRowRoot[0] = NveTables.Typography.color.explicitMask | NveTables.Typography.fontSize.explicitMask
        typoRowRoot[Int(NveTables.Typography.color.word)] = 0xFF11_2233
        typoRowRoot[Int(NveTables.Typography.fontSize.word)] = Float(20).bitPattern

        var typoRowChild = NveTables.Typography.defaults
        typoRowChild[0] = NveTables.Typography.fontSize.explicitMask | NveTables.Typography.maxLines.explicitMask
        typoRowChild[Int(NveTables.Typography.fontSize.word)] = Float(14).bitPattern
        typoRowChild[Int(NveTables.Typography.maxLines.word)] = 3 | (400 << 16)

        let typoRows = typoRowRoot + typoRowChild
        let typoOwners: [UInt32] = [NodeId.root.raw, NodeId(index: 3, generation: 0).raw]
        var typoPages = [UInt32](repeating: 0, count: Int(nveRowIndexPageLen))
        typoPages[1] = 1
        typoPages[3] = 2
        let typoDir: [UInt32] = [0]

        var interRow = NveTables.Interaction.defaults
        interRow[Int(NveTables.Interaction.eventsWord)] = NveInteractionEventSet.press | NveInteractionEventSet.keyDown
        let interOwners: [UInt32] = [NodeId(index: 3, generation: 0).raw]
        var interPages = [UInt32](repeating: 0, count: Int(nveRowIndexPageLen))
        interPages[3] = 1
        let interDir: [UInt32] = [0]

        let rows: [NveTestNodeRow] = [
            NveTestNodeRow(),
            NveTestNodeRow(table_set: (1 << typoSlot, 0, 0, 0), parent: 0, child_off: 0, child_len: 0, generation: 0, primary_slot: 0xFF, effects: 0, flags: 1),
            NveTestNodeRow(table_set: (0, 0, 0, 0), parent: NodeId.root.raw, child_off: 0, child_len: 0, generation: 0, primary_slot: 0xFF, effects: 0, flags: 1),
            NveTestNodeRow(table_set: ((1 << typoSlot) | (1 << interSlot), 0, 0, 0), parent: NodeId(index: 2, generation: 0).raw, child_off: 0, child_len: 0, generation: 0, primary_slot: 0xFF, effects: 0, flags: 1),
        ]
        let rects = [NveTestLayoutRect](repeating: NveTestLayoutRect(), count: 4)

        try typoRows.withUnsafeBufferPointer { trp in
            try typoOwners.withUnsafeBufferPointer { top in
                try typoDir.withUnsafeBufferPointer { tdp in
                    try typoPages.withUnsafeBufferPointer { tpp in
                        try interRow.withUnsafeBufferPointer { irp in
                            try interOwners.withUnsafeBufferPointer { iop in
                                try interDir.withUnsafeBufferPointer { idp in
                                    try interPages.withUnsafeBufferPointer { ipp in
                                        let tables: [NveTestTableView] = [
                                            NveTestTableView(
                                                key_lo: NveTables.Typography.key.lo, key_hi: NveTables.Typography.key.hi,
                                                slot: typoSlot, words: UInt32(NveTables.Typography.words),
                                                row_count: 2, dirty_count: 0, row_index_dir_len: 1, row_index_page_count: 1,
                                                rows: trp.baseAddress, owners: top.baseAddress, dirty_rows: nil,
                                                row_index_dir: tdp.baseAddress, row_index_pages: tpp.baseAddress),
                                            NveTestTableView(
                                                key_lo: NveTables.Interaction.key.lo, key_hi: NveTables.Interaction.key.hi,
                                                slot: interSlot, words: UInt32(NveTables.Interaction.words),
                                                row_count: 1, dirty_count: 0, row_index_dir_len: 1, row_index_page_count: 1,
                                                rows: irp.baseAddress, owners: iop.baseAddress, dirty_rows: nil,
                                                row_index_dir: idp.baseAddress, row_index_pages: ipp.baseAddress),
                                        ]
                                        try rows.withUnsafeBufferPointer { nrp in
                                            try rects.withUnsafeBufferPointer { lrp in
                                                try tables.withUnsafeBufferPointer { tvp in
                                                    var view = NveTestSceneView()
                                                    view.schema_hash = schema.value
                                                    view.frame_id = 1
                                                    view.node_capacity = 4
                                                    view.table_count = 2
                                                    view.rows = nrp.baseAddress
                                                    view.rects = lrp.baseAddress
                                                    view.tables = tvp.baseAddress
                                                    let s = try SceneReader.open(view, expectedSchema: schema)
                                                    let typo = try XCTUnwrap(s.table(key: NveTables.Typography.key))
                                                    let inter = try XCTUnwrap(s.table(key: NveTables.Interaction.key))

                                                    // Child overrides fontSize (14) and maxLines (3), inherits color from root (0xFF112233) across unattached parent (2).
                                                    let colorRow = try XCTUnwrap(s.inheritedRow(typo, forNode: 3, field: NveTables.Typography.color))
                                                    XCTAssertEqual(typo.color(colorRow, NveTables.Typography.color), Rgba(packed: 0xFF11_2233))
                                                    let sizeRow = try XCTUnwrap(s.inheritedRow(typo, forNode: 3, field: NveTables.Typography.fontSize))
                                                    XCTAssertEqual(typo.f32(sizeRow, NveTables.Typography.fontSize), 14)
                                                    let maxLinesRow = try XCTUnwrap(s.inheritedRow(typo, forNode: 3, field: NveTables.Typography.maxLines))
                                                    XCTAssertEqual(typo.u32(maxLinesRow, NveTables.Typography.maxLines), 3)
                                                    XCTAssertNil(s.inheritedRow(typo, forNode: 3, field: NveTables.Typography.letterSpacing))

                                                    // Single-pass forEachInheritedRow sees child's bits first, then root's remaining color bit.
                                                    var visited: [(Int, UInt32)] = []
                                                    s.forEachInheritedRow(typo, fromNode: 3) { r, fresh in visited.append((r, fresh)) }
                                                    XCTAssertEqual(visited.count, 2)
                                                    XCTAssertEqual(visited[0].0, 1)
                                                    XCTAssertEqual(visited[0].1, NveTables.Typography.fontSize.explicitMask | NveTables.Typography.maxLines.explicitMask)
                                                    XCTAssertEqual(visited[1].0, 0)
                                                    XCTAssertEqual(visited[1].1, NveTables.Typography.color.explicitMask)

                                                    // `noInheritMask`: bits set on node 3 (`depth == 0`) are still visited, while bits on
                                                    // ancestors (`depth > 0`) are masked out (skipping root when only masked bits remain).
                                                    var noInhVisited: [(Int, UInt32)] = []
                                                    let mask = NveTables.Typography.color.explicitMask | NveTables.Typography.maxLines.explicitMask
                                                    s.forEachInheritedRow(typo, fromNode: 3, noInheritMask: mask) { r, fresh in
                                                        noInhVisited.append((r, fresh))
                                                    }
                                                    XCTAssertEqual(noInhVisited.count, 1)
                                                    XCTAssertEqual(noInhVisited[0].0, 1)
                                                    XCTAssertEqual(noInhVisited[0].1, NveTables.Typography.fontSize.explicitMask | NveTables.Typography.maxLines.explicitMask)

                                                    // Per-table EventSet checks.
                                                    XCTAssertTrue(s.hasEvent(inter, forNode: 3, eventsWord: NveTables.Interaction.eventsWord, bit: NveInteractionEventSet.press))
                                                    XCTAssertTrue(s.hasEvent(inter, forNode: 3, eventsWord: NveTables.Interaction.eventsWord, bit: NveInteractionEventSet.keyDown))
                                                    XCTAssertFalse(s.hasEvent(inter, forNode: 3, eventsWord: NveTables.Interaction.eventsWord, bit: NveInteractionEventSet.wheel))
                                                    XCTAssertEqual(s.eventSet(inter, forNode: 1, eventsWord: NveTables.Interaction.eventsWord), 0)

                                                    #if !DEBUG
                                                    var acc: UInt64 = 0
                                                    func pass() {
                                                        s.forEachInheritedRow(typo, fromNode: 3) { r, fresh in
                                                            acc &+= UInt64(fresh) &+ UInt64(typo.u32(r, NveTables.Typography.maxLines) ?? 0)
                                                        }
                                                        acc &+= UInt64(s.eventSet(inter, forNode: 3, eventsWord: NveTables.Interaction.eventsWord))
                                                    }
                                                    pass()
                                                    if nve_test_alloc_count_begin() != 0 {
                                                        for _ in 0..<100 { pass() }
                                                        let allocs = nve_test_alloc_count_end()
                                                        XCTAssertGreaterThan(acc, 0)
                                                        XCTAssertEqual(allocs, 0, "inherited + EventSet reads allocated \(allocs) times")
                                                    }
                                                    #endif
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
