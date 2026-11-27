// Swift <-> Rust end-to-end harness: the generated Swift clients of `realistic_hybrid.nve.rs` and
// `swift_edge_hybrid.nve.rs` (two generated files in one module), linked against the static
// `NveHost` / `NveHostAbi` runtime and the live Rust provider (`swift_e2e_provider.rs`), through
// every success and error path.
import Foundation
import NveHost

func check(_ ok: Bool, _ what: String, line: UInt = #line) {
    if !ok {
        FileHandle.standardError.write(Data("FAILED line \(line): \(what)\n".utf8))
        exit(1)
    }
}

func expectError<E: Error & Equatable>(_ expected: E, _ what: String, line: UInt = #line, _ body: () throws -> Void) {
    do {
        try body()
        check(false, "\(what): no error", line: line)
    } catch {
        check(error as? E == expected, "\(what): got \(error)", line: line)
    }
}

/// Pre-order `NveValue` tags the provider saw for prop `index` in the last `setProps`.
func propShape(_ index: UInt32) -> String {
    var buf = [UInt8](repeating: 0, count: 256)
    let n = buf.withUnsafeMutableBufferPointer { nve_test_prop_shape(index, $0.baseAddress, $0.count) }
    return n < 0 ? "<none>" : String(decoding: buf.prefix(n), as: UTF8.self)  // `usize::MAX` imports as -1
}

func dynamic(_ build: (inout BinWriter) throws -> Void) throws -> DynamicValue {
    var w = BinWriter()
    try build(&w)
    return try DynamicValue(bytes: w.bytes)
}

func run() throws {
    // Resolution and handshake failures.
    expectError(HybridError.nullInterfaceResolver, "nil resolver") { _ = try CryptoHybridClient.resolve(getInterface: nil) }
    expectError(HybridError.interfaceNotFound(CryptoHybridClient.interfaceId), "unknown interface") {
        _ = try CryptoHybridClient.resolve(getInterface: { _ in nil })
    }
    let rawCrypto = nve_get_interface(CryptoHybridClient.interfaceId)!.assumingMemoryBound(to: NveHybridCryptoVTable.self)
    var stale = rawCrypto.pointee
    stale.schema_hash ^= 1
    expectError(HybridError.schemaMismatch(expected: Key64(CryptoHybridClient.schemaHash), actual: Key64(stale.schema_hash)), "schema") {
        _ = try withUnsafePointer(to: &stale) { try CryptoHybridClient(vtable: $0) }
    }
    var future = rawCrypto.pointee
    future.abi_version += 1
    expectError(HybridError.abiVersionMismatch(expected: CryptoHybridFile.abiVersion, actual: future.abi_version), "abi version") {
        _ = try withUnsafePointer(to: &future) { try CryptoHybridClient(vtable: $0) }
    }
    var dead = rawCrypto.pointee
    dead.create = { nil }
    expectError(HybridError.nullHandle, "null handle") {
        _ = try withUnsafePointer(to: &dead) { try CryptoHybridClient(vtable: $0) }
    }
    var hollow = rawCrypto.pointee
    hollow.add = nil
    try withUnsafePointer(to: &hollow) { vtable in
        let partial = try CryptoHybridClient(vtable: vtable)
        expectError(HybridError.nullFunction("add"), "null vtable entry") { _ = try partial.add(a: 1, b: 2) }
        check(try partial.version() == "nve-hybrid-1.0", "other entries still work")
    }

    let crypto = try CryptoHybridClient.resolve(getInterface: nve_get_interface)
    check(try crypto.version() == "nve-hybrid-1.0", "version")
    check(try crypto.add(a: 19, b: 23) == 42, "add")
    check(try crypto.add(a: .min, b: 0) == .min, "add i64.min")
    check(try crypto.blake3(data: [0x10, 0x20, 0x30]) == [0x4A, 0x7A, 0x6A], "blake3")
    check(try crypto.blake3(data: []) == [], "blake3 empty")

    let header = try crypto.hashWith(algo: .sha512, data: [1, 2, 3])
    check(header == DigestHeader(algorithm: .sha512, byteLen: 3, verified: true, anchor: FramePoint(x: 12.5, y: -4.25)), "hashWith")

    check(try crypto.findNonce(label: "hello", hint: 100) == 105, "findNonce some")
    check(try crypto.findNonce(label: "héllo 🚀", hint: 0) == UInt64("héllo 🚀".utf8.count), "findNonce emoji")
    check(try crypto.findNonce(label: "", hint: nil) == nil, "findNonce none")

    var payload = SignedPayload(
        header: DigestHeader(algorithm: .sha256, byteLen: 8, verified: true, anchor: FramePoint(x: 1, y: 2)),
        label: "ok", digest: [1, 2, 3], tags: ["alpha", "beta"], nonce: 1337)
    check(try crypto.verifyPayload(payload: payload) == true, "verifyPayload")
    payload.nonce = nil
    check(try crypto.verifyPayload(payload: payload) == false, "verifyPayload without nonce")
    expectError(HybridError.remote("missing payload label"), "verifyPayload error") { _ = try crypto.verifyPayload(payload: SignedPayload()) }

    check(try crypto.lookupPayload(id: 0) == nil, "lookupPayload none")
    let found = try crypto.lookupPayload(id: 7)
    check(found == SignedPayload(
        header: DigestHeader(algorithm: .blake3, byteLen: 28, verified: true, anchor: FramePoint(x: 7, y: 8)),
        label: "rec-7", digest: [0xDE, 0xAD, 0xBE, 0xEF], tags: ["fast", "binary"], nonce: 4242), "lookupPayload some")

    let batch = try crypto.batchSign(inputs: ["one", "twö 🚀", ""], algo: .sha256)
    check(batch.map(\.label) == ["one", "twö 🚀", ""], "batchSign labels")
    check(batch.map(\.header.byteLen) == [3, UInt32("twö 🚀".utf8.count), 0], "batchSign byte lengths")
    expectError(HybridError.remote("empty batch"), "batchSign error") { _ = try crypto.batchSign(inputs: [], algo: .blake3) }
    try crypto.resetCache()

    // Dynamic values cross as `NveValue` trees (true tags) and come back byte-identical.
    let meta = try dynamic { w in
        let map = w.beginMap(2)
        try w.writeMapKey("k")
        let arr = w.beginArray(3)
        w.writeI64(1)
        try w.writeStr("x")
        w.writeNull()
        try w.end(arr)
        try w.writeMapKey("f")
        w.writeF64(2.5)
        try w.end(map)
    }
    let edgy = try dynamic { w in
        let map = w.beginMap(6)
        try w.writeMapKey("🚀")
        try w.writeStr("héllo 👩‍👩‍👧")
        try w.writeMapKey("neg")
        w.writeI64(.min)
        try w.writeMapKey("e")
        try w.end(w.beginArray(0))
        try w.writeMapKey("")
        try w.end(w.beginMap(0))
        try w.writeMapKey("b")
        try w.writeBytes([UInt8]())
        try w.writeMapKey("deep")
        let a = w.beginArray(1)
        let b = w.beginArray(1)
        let c = w.beginMap(1)
        try w.writeMapKey("x")
        w.writeBool(true)
        try w.end(c)
        try w.end(b)
        try w.end(a)
        try w.end(map)
    }
    for value in [meta, edgy, DynamicValue(), try dynamic { try $0.writeStr("") }, try dynamic { w in w.writeF64(-0.0) }] {
        check(try crypto.inspectDynamic(meta: value) == value, "inspectDynamic \(value.bytes)")
    }

    let video = try VideoSurfaceHybridClient.resolve(getInterface: nve_get_interface)
    var props = VideoSurfaceProps(sourceUrl: "https://nve.dev/clip.mp4", muted: true, quality: .uhd4k)
    try video.setProps(props)
    // source_url, muted, playback_rate, quality, poster_url, focus_point, drm_key, chapters, metadata
    check((0..<9).map(propShape) == ["4", "1", "3", "2", "0", "0", "5", "5", "0"], "default prop tags \((0..<9).map(propShape))")
    try video.seek(positionMs: 9876)
    check(try video.captureFrame(atPoint: FramePoint(x: 5, y: 9)) == [5, 9, 24], "captureFrame")
    expectError(HybridError.remote("bad \u{1F600}"), "captureFrame error") { _ = try video.captureFrame(atPoint: FramePoint(x: -1, y: 0)) }

    // Present options, `Bytes`, `Vec`, and `NveValue` props: the dynamic value keeps its own tags
    // (`Map` ... never `Bytes`), while `Struct` / `Vec` props stay one `Bytes` payload.
    props.posterUrl = "p.png"
    props.focusPoint = FramePoint(x: 7, y: 0)
    props.drmKey = [0xAA, 0xBB]
    props.chapters = [10, 20, 30]
    props.metadata = meta
    try video.setProps(props)
    check((4..<9).map(propShape) == ["4", "5", "5", "5", "762403"], "present prop tags \((4..<9).map(propShape))")
    check(try video.captureFrame(atPoint: FramePoint(x: 5, y: 9)) == [5, 9, 24, 7, 5, 0xAA, 0xBB, 10, 20, 30], "captureFrame with props")
    check(try video.currentMetadata() == meta, "metadata round trip")

    props.metadata = edgy
    try video.setProps(props)
    check(propShape(8) == "7426756671", "nested metadata tags \(propShape(8))")
    check(try video.currentMetadata() == edgy, "edge metadata round trip")

    props.posterUrl = nil
    props.focusPoint = nil
    props.drmKey = []
    props.chapters = []
    props.metadata = DynamicValue()
    try video.setProps(props)
    check(try video.captureFrame(atPoint: FramePoint(x: 5, y: 9)).count == 3, "cleared props")
    check(try video.currentMetadata().isNull, "cleared metadata")

    // Field names shadowing codec names (`w`, `mark`, `r`, `count`), two options, a nested sequence.
    let cursor = Cursor(mark: 1, w: [1, 2, 3], r: "ab", count: 4, tag: 42, grid: [[1, 2, 3], [4, 5, 6]])
    let t = try crypto.transpose(cursor: cursor)
    check(t == Cursor(mark: 2, w: [3, 2, 1], r: "AB", count: 8, tag: -42, grid: [[1, 4], [2, 5], [3, 6]]), "transpose")
    let empty = try crypto.transpose(cursor: Cursor())
    check(empty == Cursor(mark: 1, w: [], r: nil, count: 0, tag: nil, grid: []), "transpose empty")
}

func runEdge() throws {
    let edge = try EdgeHybridClient.resolve(getInterface: nve_get_interface)
    let meta = try dynamic { w in
        let map = w.beginMap(2)
        try w.writeMapKey("🚀")
        w.writeF64(.nan)
        try w.writeMapKey("min")
        w.writeI64(.min)
        try w.end(map)
    }
    let text = try dynamic { try $0.writeStr("é") }
    // Two pinned arguments and two dynamic values around a returned value: every nested scope returns.
    check(try edge.mix(a: "é🚀", b: [1, 2, 3], meta: meta, extra: text) == "é🚀|[1, 2, 3]|\(meta.bytes)|\(text.bytes)", "mix")
    check(try edge.mix(a: "", b: [], meta: DynamicValue(), extra: DynamicValue()) == "|[]|[0]|[0]", "mix empty")

    // Sibling sequences in one struct and in one method; NaN and Int64.min survive both directions.
    let pair = try edge.pair(xs: [1, -2, .max, .min], ys: ["a", "", "🚀"])
    check(pair.ints == [1, -2, .max, .min] && pair.names == ["a", "", "🚀"], "pair sequences")
    check(pair.blobs == [[], [1, 2]] && pair.notANumber.isNaN && pair.min == .min && pair.level == .high, "pair scalars")
    let echoed = try edge.echoBundle(bundle: pair)
    check(echoed.notANumber.bitPattern == pair.notANumber.bitPattern, "NaN bit pattern")
    var comparable = echoed
    comparable.notANumber = 0
    var expected = pair
    expected.notANumber = 0
    check(comparable == expected, "echoBundle")

    // C member names that are Swift keywords after `.` (`vtable.pointee.init`, `raw.init`).
    let keywords = try edge.`init`(k: Keywords(init: 41, `inout`: .min + 1, deinit: 1.5, subscript: false))
    check(keywords == Keywords(init: 42, `inout`: .max, deinit: -1.5, subscript: true), "init")

    // What no typed Rust value can send.
    check(try edge.level(raw: 7) == .high && edge.level(raw: 0) == .low, "level")
    expectError(HybridError.invalidEnum("Level", 3), "unknown enum value") { _ = try edge.level(raw: 3) }
    try edge.fail(code: 0)
    expectError(HybridError.status(.invalidArg), "bare status") { try edge.fail(code: 2) }
    expectError(HybridError.status(.err), "unknown status") { try edge.fail(code: 99) }
    expectError(BinError.trailingBytes(1), "trailing reply byte") { _ = try edge.junk(kind: 0) }
    expectError(BinError.truncated, "count past payload") { _ = try edge.junk(kind: 1) }
    expectError(BinError.maxDepth, "too deep") { _ = try edge.junk(kind: 2) }
    expectError(BinError.unexpectedTag(expected: .map, actual: .null), "wrong shape") { _ = try edge.junk(kind: 3) }
    expectError(BinError.truncated, "empty reply") { _ = try edge.junk(kind: 4) }
}

do {
    try run()
    try runEdge()
    print("swift e2e ok")
} catch {
    FileHandle.standardError.write(Data("FAILED: uncaught \(error)\n".utf8))
    exit(1)
}
