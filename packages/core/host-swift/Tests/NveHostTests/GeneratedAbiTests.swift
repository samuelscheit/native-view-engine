import CNveAbi
import NveHost
import XCTest

// The generated C structs satisfy the runtime protocols with empty extensions.
extension NveTableView: TableViewABI {}

/// `nve codegen abi` output against this package's static runtime: generated table keys must be
/// `xxh64(name)` and every field spec must fit its row.
final class GeneratedAbiTests: XCTestCase {
    func testGeneratedTablesMatchTheRuntime() {
        XCTAssertEqual(NveTables.all.count, Int(NVE_TABLE_COUNT))
        for t in NveTables.all {
            XCTAssertEqual(t.key, Key64.of(t.name), t.name)
            XCTAssertTrue((1...32).contains(t.words), t.name)
            for f in t.fields {
                XCTAssertLessThanOrEqual(Int(f.word) + (f.kind == .str ? 2 : 1), t.words, t.name)
            }
        }
        XCTAssertEqual(NveTables.Layout.key, Key64(NVE_TABLE_LAYOUT_KEY))
        XCTAssertEqual(NveTables.Layout.defaults.count, NveTables.Layout.words)
        // The pure-Swift generated constants match the C ABI header `CNveAbi` imports.
        XCTAssertEqual(nveRowIndexPageLen, UInt32(NVE_ROW_INDEX_PAGE_LEN))
        XCTAssertEqual(nveRowIndexPageBits, UInt32(NVE_ROW_INDEX_PAGE_BITS))
        XCTAssertEqual(nveRowIndexNoPage, UInt32(NVE_ROW_INDEX_NO_PAGE))
        XCTAssertEqual(nveRowIndexPageLen, 1 << nveRowIndexPageBits)
    }

    func testGeneratedStrFieldReadsItsSpan() throws {
        var row = [UInt32](repeating: 0, count: NveTables.Accessibility.words)
        let label = NveTables.Accessibility.label
        row[Int(label.word)] = 5
        row[Int(label.word) + 1] = 7
        try row.withUnsafeBufferPointer { rows in
            let table = try XCTUnwrap(TableReader(
                key: NveTables.Accessibility.key, slot: 0, words: rows.count, rowCount: 1, rows: rows))
            XCTAssertEqual(table.str(0, label), StrSpan(offset: 5, len: 7))
        }
    }

}
