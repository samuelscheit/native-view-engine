struct HugeBytes: RandomAccessCollection {
    var startIndex: Int { 0 }
    var endIndex: Int { Int(UInt32.max) + 1 }
    subscript(_: Int) -> UInt8 { 0 }
}
