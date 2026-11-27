// `[T; N]` (Swift `[T]`) over the generated `NveBinValue` codec: lossless round trip, wrong-length
// inputs rejected on encode, and wrong-length arrays rejected on decode.
import NveHost

func require(_ cond: Bool, _ what: String, line: Int = #line) {
    if !cond { fatalError("FAILED line \(line): \(what)") }
}

func expect(_ error: BinError, _ what: String, line: Int = #line, _ body: () throws -> Void) {
    do {
        try body()
        fatalError("FAILED line \(line): \(what) did not throw")
    } catch let thrown as BinError {
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

let defaults = FixedBundle()
require(defaults.coords == [0, 0, 0, 0], "default coords hold exactly 4 zeros")
require(defaults.points == [FixedPoint(), FixedPoint()], "default points hold exactly 2 points")

var bundle = FixedBundle()
bundle.label = "fixed"
bundle.coords = [1, 0xFFFF_FFFF, 0, 42]
bundle.points = [FixedPoint(x: -7, y: 8), FixedPoint(x: .max, y: .min)]
bundle.tag = [9, 0, 255]

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

for (field, count, error) in [("coords", 3, BinError.truncated), ("coords", 5, .tooLarge)] {
    var bad = bundle
    bad.coords = Array(repeating: 1, count: count)
    var bw = BinWriter()
    expect(error, "encoding \(count) \(field)") { try bad.encodeBin(into: &bw) }
}
var shortPoints = bundle
shortPoints.points = [FixedPoint()]
var pw = BinWriter()
expect(.truncated, "encoding 1 point") { try shortPoints.encodeBin(into: &pw) }
var longTag = bundle
longTag.tag = [1, 2, 3, 4]
var tw = BinWriter()
expect(.tooLarge, "encoding a 4-byte tag") { try longTag.encodeBin(into: &tw) }

for (key, count, points, error): (String, UInt32, Bool, BinError?) in [
    ("coords", 4, false, nil), ("coords", 3, false, .truncated), ("coords", 0, false, .truncated),
    ("coords", 5, false, .tooLarge), ("points", 2, true, nil), ("points", 1, true, .truncated),
    ("points", 3, true, .tooLarge), ("tag", 2, false, .truncated), ("tag", 4, false, .tooLarge),
] {
    var br = BinReader(try bundleWith(key, count, points: points))
    if let error {
        expect(error, "decoding \(count) \(key)") { _ = try FixedBundle.decodeBin(from: &br) }
    } else {
        _ = try FixedBundle.decodeBin(from: &br)
    }
}

print("swift fixed array ok")
