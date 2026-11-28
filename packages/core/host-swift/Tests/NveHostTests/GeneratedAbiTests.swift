import CNveAbi
import NveHost
import NveHostAbi
import XCTest

// The generated C structs satisfy the runtime protocols with empty extensions.
extension NveTableView: TableViewABI {}

/// `nve codegen abi` output against this package's static runtime: generated table keys must be
/// `xxh64(name)`, every field spec must fit its row, and Track 1B-1d event/inheritance/optional
/// constants must agree between pure-Swift `NveHost` and C `CNveAbi`.
final class GeneratedAbiTests: XCTestCase {
    func testGeneratedTablesMatchTheRuntime() {
        XCTAssertEqual(NveTables.all.count, Int(NVE_TABLE_COUNT))
        for t in NveTables.all {
            XCTAssertEqual(t.key, Key64.of(t.name), t.name)
            XCTAssertTrue((1...maxTableWords).contains(t.words), t.name)
            if t.eventsWord != noValWord {
                XCTAssertLessThan(Int(t.eventsWord), t.words, t.name)
                XCTAssertFalse(t.inherited, t.name)
            }
            for f in t.fields {
                XCTAssertTrue(f.fits(words: t.words), t.name)
                if t.inherited {
                    XCTAssertLessThan(f.explicitBit, 32, t.name)
                } else {
                    XCTAssertEqual(f.explicitBit, noValWord, t.name)
                    XCTAssertFalse(f.inherits, t.name)
                }
            }
        }
        XCTAssertEqual(NveTables.Layout.key, Key64(NVE_TABLE_LAYOUT_KEY))
        XCTAssertEqual(NveTables.Layout.defaults.count, NveTables.Layout.words)
        XCTAssertEqual(NveTables.Layout.eventsWord, 0)
        XCTAssertEqual(NveTables.Interaction.eventsWord, 0)
        XCTAssertEqual(NveTables.Scroll.eventsWord, 0)
        XCTAssertEqual(NveTables.TextInput.eventsWord, 0)

        // Typography is #[nve_table(inherited)]: word 0 is __explicit, no separate TypographySet field.
        XCTAssertTrue(NveTables.Typography.inherited)
        XCTAssertEqual(NveTables.Typography.eventsWord, noValWord)
        XCTAssertEqual(NveTables.Typography.color.explicitBit, 0)
        XCTAssertTrue(NveTables.Typography.color.inherits)
        XCTAssertEqual(NveTables.Typography.maxLines.opt, .zeroNiche)
        XCTAssertEqual(NveTables.Layout.aspectRatio.opt, .wideBit)
        XCTAssertEqual(NveTables.Interaction.tabIndex.opt, .bit0Present)
        XCTAssertEqual(NveTables.Accessibility.role.opt, .plusOne)
        XCTAssertEqual(NveTables.GridItem.columnStart.kind, .gridLine)

        // The pure-Swift generated constants match the C ABI header `CNveAbi` imports.
        XCTAssertEqual(nveRowIndexPageLen, UInt32(NVE_ROW_INDEX_PAGE_LEN))
        XCTAssertEqual(nveRowIndexPageBits, UInt32(NVE_ROW_INDEX_PAGE_BITS))
        XCTAssertEqual(nveRowIndexNoPage, UInt32(NVE_ROW_INDEX_NO_PAGE))
        XCTAssertEqual(nveRowIndexPageLen, 1 << nveRowIndexPageBits)

        // Uniform 64 B NveEvent + NveEventBatch ABI parity between Swift and C.
        XCTAssertEqual(nveEventSize, UInt32(NVE_EVENT_SIZE))
        XCTAssertEqual(nveEventAlign, UInt32(NVE_EVENT_ALIGN))
        XCTAssertEqual(nveEventInlineKeyCap, UInt32(NVE_EVENT_INLINE_KEY_CAP))
        XCTAssertEqual(MemoryLayout<CNveAbi.NveEvent>.size, 64)
        XCTAssertEqual(MemoryLayout<CNveAbi.NveEvent>.stride, 64)
        XCTAssertEqual(MemoryLayout<CNveAbi.NveEventBatch>.size, MemoryLayout<Int>.size == 8 ? 24 : 16)
        XCTAssertEqual(nveTableInteraction, UInt64(NVE_TABLE_INTERACTION))
        XCTAssertEqual(nveTableTextInput, UInt64(NVE_TABLE_TEXT_INPUT))
        XCTAssertEqual(nveTableScroll, UInt64(NVE_TABLE_SCROLL))
        XCTAssertEqual(nveTableLayout, UInt64(NVE_TABLE_LAYOUT))
        XCTAssertEqual(nveTableCustom, UInt64(NVE_TABLE_CUSTOM))
        XCTAssertEqual(nveEventFlagModMeta, UInt32(NVE_EVENT_FLAG_MOD_META))
        XCTAssertEqual(nveEventFlagUtf8Bytes, UInt32(NVE_EVENT_FLAG_UTF8_BYTES))
        XCTAssertEqual(nveEventFlagInlineKeyUtf8, UInt32(NVE_EVENT_FLAG_INLINE_KEY_UTF8))
        XCTAssertEqual(nveEventFlagCustomNamed, UInt32(NVE_EVENT_FLAG_CUSTOM_NAMED))
        XCTAssertEqual(nveEventFlagKeySplitBytes, UInt32(NVE_EVENT_FLAG_KEY_SPLIT_BYTES))

        // EventEnum discriminants match generated EventSet bits.
        XCTAssertEqual(InteractionEventId.press.eventBit, NveInteractionEventSet.press)
        XCTAssertEqual(InteractionEventId.wheel.eventBit, NveInteractionEventSet.wheel)
        XCTAssertEqual(TextInputEventId.change.eventBit, NveTextInputEventSet.change)
        XCTAssertEqual(TextInputEventId.compositionEnd.eventBit, NveTextInputEventSet.compositionEnd)
        XCTAssertEqual(ScrollEventId.scroll.eventBit, NveScrollEventSet.scroll)
        XCTAssertEqual(LayoutEventId.layout.eventBit, NveLayoutEventSet.layout)
    }

    func testGeneratedStrAndOptionalFieldsReadCorrectly() throws {
        var row = [UInt32](repeating: 0, count: NveTables.Accessibility.words)
        let label = NveTables.Accessibility.label
        row[Int(label.word)] = 5
        row[Int(label.word) + 1] = 7
        // role (.plusOne): 0 = nil, NveAriaRole.button (0) -> stored as 1.
        row[Int(NveTables.Accessibility.role.word)] |= (NveAriaRole.button.rawValue + 1) << NveTables.Accessibility.role.shift
        // level (.bit0Present): Some(3) -> (3 << 1) | 1 = 7.
        row[Int(NveTables.Accessibility.level.word)] |= ((3 << 1) | 1) << NveTables.Accessibility.level.shift
        // valueNow (.wideBit): presence bit set, valWord = Float(42.5).bitPattern.
        let vn = NveTables.Accessibility.valueNow
        row[Int(vn.word)] |= 1 << vn.shift
        row[Int(vn.valWord)] = Float(42.5).bitPattern

        try row.withUnsafeBufferPointer { rows in
            let table = try XCTUnwrap(TableReader(
                key: NveTables.Accessibility.key, slot: 0, words: rows.count, rowCount: 1, rows: rows))
            XCTAssertEqual(table.str(0, label), StrSpan(offset: 5, len: 7))
            XCTAssertEqual(table.enumValue(0, NveTables.Accessibility.role), NveAriaRole.button.rawValue)
            XCTAssertEqual(table.u32(0, NveTables.Accessibility.level), 3)
            XCTAssertEqual(table.f32(0, vn), 42.5)
            XCTAssertNil(table.f32(0, NveTables.Accessibility.valueMin))
        }
    }
}
