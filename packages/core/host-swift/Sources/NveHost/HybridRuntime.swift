/// Hybrid FFI prelude shared by every generated Swift hybrid client.

/// Status of a fallible hybrid FFI trampoline (Rust `NveHybridStatus`, `#[repr(i32)]`).
public enum HybridStatus: Int32, Sendable {
    case ok = 0, err = 1, invalidArg = 2, schemaMismatch = 3, nullHandle = 4
}

/// Status of `NveHostCallbacks::invoke` / `channel_publish` (Rust `NveHostStatus`, `#[repr(i32)]`).
public enum HostStatus: Int32, Sendable {
    case ok = 0, pending = 1
    case bufferTooSmall = -1, unsupported = -2, permissionDenied = -3, invalidArg = -4
    case notFound = -5, cancelled = -6, ioError = -7, internalError = -8
}

public enum HybridError: Error, Equatable, Sendable {
    case nullInterfaceResolver
    case interfaceNotFound(UInt32)
    case schemaMismatch(expected: Key64, actual: Key64)
    case nullHandle
    case status(HybridStatus)
    case codec(BinError)
    case remote(String)

    /// Throws `.schemaMismatch` unless a peer's schema hash equals the generated one.
    public static func checkSchema(expected: Key64, actual: Key64) throws(HybridError) {
        guard expected == actual else { throw .schemaMismatch(expected: expected, actual: actual) }
    }
}

/// Owned dynamic value in the compact `NveBinValue` encoding; always exactly one valid value.
public struct DynamicValue: Hashable, Sendable {
    public let bytes: [UInt8]

    /// `null`.
    public init() { bytes = [BinTag.null.rawValue] }

    /// Validated copy of `bytes`.
    public init(bytes: [UInt8]) throws(BinError) {
        try BinReader.validate(bytes)
        self.bytes = bytes
    }

    public var tag: BinTag { BinTag(rawValue: bytes[0]) ?? .null }
    public var isNull: Bool { tag == .null }
    public var reader: BinReader { BinReader(bytes) }
}
