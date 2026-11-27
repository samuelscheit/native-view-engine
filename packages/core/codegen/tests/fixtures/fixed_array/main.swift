// `[T; N]` (Swift `[T]`) over the generated Swift bindings, against the live Rust provider: the
// `NveBinValue` codec (lossless; wrong lengths rejected on encode and decode), POD C tuples (nested
// too) in `toAbi` / `fromAbi`, `Option<[T; N]>` / `Result<Option<[T; N]>, String>` returns, and
// `[T; N]` view props.
import NveHost

func require(_ cond: Bool, _ what: String, line: Int = #line) {
    if !cond { fatalError("FAILED line \(line): \(what)") }
}

func expect<E: Error & Equatable>(_ error: E, _ what: String, line: Int = #line, _ body: () throws -> Void) {
    do {
        try body()
        fatalError("FAILED line \(line): \(what) did not throw")
    } catch let thrown as E {
        require(thrown == error, "\(what) threw \(thrown), expected \(error)", line: line)
    } catch {
        fatalError("FAILED line \(line): \(what) threw \(error)")
    }
}

/// `{"<key>": [count x value]}`: a bundle whose `key` array has `count` entries.
func bundleWith(_ key: String, _ count: UInt32, points: Bool = false) throws -> [UInt8] {
    var w = BinWriter()
    let map = w.beginMap(1)
    try w.writeMapKey(key)
    let arr = w.beginArray(count)
    for i in 0..<count {
        if points {
            try FixedPoint(x: Int32(i), y: -1).encodeBin(into: &w)
        } else {
            w.writeI64(Int64(i))
        }
    }
    try w.end(arr)
    try w.end(map)
    return w.bytes
}

func sampleBundle() -> FixedBundle {
    FixedBundle(
        label: "fixed", coords: [1, 0xFFFF_FFFF, 0, 42],
        points: [FixedPoint(x: -7, y: 8), FixedPoint(x: .max, y: .min)],
        matrix: [[1, 2], [3, 4], [5, 6]], tag: [9, 0, 255])
}

func samplePod() -> FixedPod {
    var flags = Array(repeating: Array(repeating: false, count: 33), count: 2)
    flags[0][0] = true
    flags[0][32] = true
    flags[1][5] = true
    return FixedPod(
        v: [1.5, -2, 0, 3.25], points: [FixedPoint(x: 1, y: 2), FixedPoint(x: 3, y: 4)],
        kinds: [.gamma, .beta], grid: [[10, 11], [12, 13], [14, 15]], flags: flags)
}

// Defaults hold exactly N elements.
let defaults = FixedBundle()
require(defaults.coords == [0, 0, 0, 0] && defaults.matrix == [[0, 0], [0, 0], [0, 0]], "default lengths")
require(FixedPod().flags.map(\.count) == [33, 33], "nested default lengths")

// Binary codec: lossless round trip, absent option, wrong lengths on encode and decode.
var bundle = sampleBundle()
var w = BinWriter()
try bundle.encodeBin(into: &w)
var r = BinReader(w.bytes)
require(try FixedBundle.decodeBin(from: &r) == bundle, "lossless round trip")
require(r.isAtEnd, "round trip consumes every byte")
bundle.tag = nil
var w2 = BinWriter()
try bundle.encodeBin(into: &w2)
var r2 = BinReader(w2.bytes)
require(try FixedBundle.decodeBin(from: &r2) == bundle, "absent optional fixed array")

for (count, error) in [(3, BinError.truncated), (5, .tooLarge)] {
    var bad = sampleBundle()
    bad.coords = Array(repeating: 1, count: count)
    var bw = BinWriter()
    expect(error, "encoding \(count) coords") { try bad.encodeBin(into: &bw) }
}
var shortRow = sampleBundle()
shortRow.matrix[1] = [1]
var mw = BinWriter()
expect(BinError.truncated, "encoding a 1-element matrix row") { try shortRow.encodeBin(into: &mw) }
var longTag = sampleBundle()
longTag.tag = [1, 2, 3, 4]
var tw = BinWriter()
expect(BinError.tooLarge, "encoding a 4-byte tag") { try longTag.encodeBin(into: &tw) }

for (key, count, points, error): (String, UInt32, Bool, BinError?) in [
    ("coords", 4, false, nil), ("coords", 3, false, .truncated), ("coords", 0, false, .truncated),
    ("coords", 5, false, .tooLarge), ("points", 2, true, nil), ("points", 1, true, .truncated),
    ("points", 3, true, .tooLarge), ("matrix", 2, false, .truncated), ("tag", 2, false, .truncated),
    ("tag", 4, false, .tooLarge),
] {
    var br = BinReader(try bundleWith(key, count, points: points))
    if let error {
        expect(error, "decoding \(count) \(key)") { _ = try FixedBundle.decodeBin(from: &br) }
    } else {
        _ = try FixedBundle.decodeBin(from: &br)
    }
}
var truncated = BinReader([7])
expect(BinError.truncated, "1-byte map") { _ = try FixedBundle.decodeBin(from: &truncated) }

// POD C tuples, nested dimensions included.
let pod = samplePod()
let raw = try pod.toAbi()
require(raw.v.3 == 3.25 && raw.points.1.y == 4 && raw.grid.2.1 == 15, "toAbi copies elements")
require(raw.kinds.0 == FixedKind.gamma.rawValue && raw.flags.0.32 && raw.flags.1.5 && !raw.flags.1.4, "toAbi nested")
require(try FixedPod.fromAbi(raw) == pod, "fromAbi restores every element")
var badPod = samplePod()
badPod.grid[2] = [1, 2, 3]
expect(BinError.tooLarge, "toAbi of a 3-element grid row") { _ = try badPod.toAbi() }
badPod = samplePod()
badPod.v = [1]
expect(BinError.truncated, "toAbi of a 1-element v") { _ = try badPod.toAbi() }
var badKind = raw
badKind.kinds.1 = 99
expect(HybridError.invalidEnum("FixedKind", 99), "fromAbi of an undeclared kind") { _ = try FixedPod.fromAbi(badKind) }

// Live calls into the Rust provider.
let codec = try FixedCodecHybridClient.resolve(getInterface: nve_get_interface)
let echoed = try codec.echo(bundle: sampleBundle())
require(echoed.label == "fixed!" && echoed.coords == [42, 0, 0xFFFF_FFFF, 1], "echo coords")
require(echoed.points == [FixedPoint(x: .max, y: .min), FixedPoint(x: -7, y: 8)], "echo points")
require(echoed.matrix == [[2, 1], [4, 3], [6, 5]] && echoed.tag == [255, 0, 9], "echo matrix and tag")
require(try codec.swap(coords: [1, 2, 3, 4]) == [4, 3, 2, 1], "swap")
expect(BinError.truncated, "swap with 3 coords") { _ = try codec.swap(coords: [1, 2, 3]) }
require(try codec.tag(present: true) == [7, 8, 255], "tag present")
require(try codec.tag(present: false) == nil, "tag absent")
require(try codec.tryTag(present: true) == [1, 2, 3], "tryTag ok")
expect(HybridError.remote("no tag"), "tryTag error") { _ = try codec.tryTag(present: false) }
let mirrored = try codec.mirror(pod: samplePod())
require(mirrored.v == [-1.5, 2, -0.0, -3.25] && mirrored.kinds == [.beta, .gamma], "mirror v and kinds")
require(mirrored.points == [FixedPoint(x: 3, y: 4), FixedPoint(x: 1, y: 2)], "mirror points")
require(mirrored.grid == [[11, 12], [13, 14], [15, 16]], "mirror grid")
require(!mirrored.flags[0][0] && !mirrored.flags[0][32] && mirrored.flags[0][1] && !mirrored.flags[1][5], "mirror flags")

let view = try FixedViewHybridClient.resolve(getInterface: nve_get_interface)
var props = FixedViewProps(coords: [5, 6, 7, 8], tag: [4, 5, 6], pod: samplePod())
try view.setProps(props)
let summary = try view.summary()
require(summary.label == "view:3" && summary.coords == [5, 6, 7, 8], "view coords")
require(summary.matrix == samplePod().grid && summary.tag == [4, 5, 6], "view pod and tag")
props.tag = nil
try view.setProps(props)
require(try view.summary().tag == nil, "view absent tag")
props.coords = [1]
expect(BinError.truncated, "setProps with 1 coord") { try view.setProps(props) }

print("swift fixed array ok")
