/// Hybrid FFI runtime shared by every generated Swift hybrid client (`nve codegen hybrid --swift`).
/// The parts that build C `NveValue` graphs live in `NveHostAbi`, which imports the generated C ABI.

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

extension HybridStatus {
    /// Throws `.status` unless `raw` is `ok`; an unknown code reads as `err`.
    public static func check(_ raw: Int32) throws(HybridError) {
        guard raw != HybridStatus.ok.rawValue else { return }
        throw .status(HybridStatus(rawValue: raw) ?? .err)
    }
}

/// C `nve_get_interface`: the vtable of `interfaceId`, or null when the peer does not export it.
public typealias HybridInterfaceResolver = @convention(c) (UInt32) -> UnsafeRawPointer?

public enum HybridError: Error, Equatable, Sendable {
    case nullInterfaceResolver
    case interfaceNotFound(UInt32)
    case abiVersionMismatch(expected: UInt32, actual: UInt32)
    case schemaMismatch(expected: Key64, actual: Key64)
    case nullHandle
    /// A vtable entry the client calls is null (its C field name).
    case nullFunction(String)
    /// A wire enum value with no case in the generated enum (enum name, raw value).
    case invalidEnum(String, UInt32)
    case status(HybridStatus)
    case codec(BinError)
    case remote(String)

    /// Throws `.schemaMismatch` unless a peer's schema hash equals the generated one.
    public static func checkSchema(expected: Key64, actual: Key64) throws(HybridError) {
        guard expected == actual else { throw .schemaMismatch(expected: expected, actual: actual) }
    }

    /// Throws `.abiVersionMismatch` unless a peer's hybrid ABI version equals the generated one.
    public static func checkAbiVersion(expected: UInt32, actual: UInt32) throws(HybridError) {
        guard expected == actual else { throw .abiVersionMismatch(expected: expected, actual: actual) }
    }
}

/// A generated hybrid enum: a `UInt32` C enum whose wire values may come from an untrusted peer.
public protocol HybridEnum: RawRepresentable, Hashable, Sendable where RawValue == UInt32 {}

extension HybridEnum {
    /// The case of `raw`, or `.invalidEnum` naming this type.
    public static func decode(_ raw: UInt32) throws(HybridError) -> Self {
        guard let value = Self(rawValue: raw) else { throw .invalidEnum(String(describing: Self.self), raw) }
        return value
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
