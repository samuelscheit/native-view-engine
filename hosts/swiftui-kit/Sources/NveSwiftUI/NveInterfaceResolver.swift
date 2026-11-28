import CNveAbi
import NveHost
import Darwin
import Foundation

// MARK: - Static + Dynamic `nve_get_interface` Resolver

/// Origin of a resolved `nve_get_interface` entry point.
public enum NveInterfaceResolutionSource: Equatable, Sendable {
    /// Caller supplied an explicit `NveGetInterfaceFn` C function pointer.
    case explicit
    /// Resolved from `NveStaticInterfaceTable` (App Store static `lib*.a` archives on iOS/tvOS/watchOS/visionOS).
    case staticTable
    /// A `.dylib` path the runtime loads (`load_app_module`), for dev hot reload / macOS.
    case dynamicLibrary
    /// Dynamically resolved from the current process image via `dlopen(nil, RTLD_NOW)` / `dlsym`.
    case processSymbol
}

/// Thread-safe static interface registry enabling App Store iOS, tvOS, watchOS, and visionOS apps
/// to link Rust `lib*.a` archives statically without `dlopen` of unsigned dynamic libraries.
///
/// Example for a statically linked Rust crate exporting `nve_get_interface`:
/// ```swift
/// @_silgen_name("nve_get_interface")
/// func my_nve_get_interface(_ interfaceId: UInt32) -> UnsafeRawPointer?
///
/// NveStaticInterfaceTable.shared.register(
///     symbol: "nve_get_interface",
///     getInterface: my_nve_get_interface
/// )
/// ```
public final class NveStaticInterfaceTable: @unchecked Sendable {
    public static let shared = NveStaticInterfaceTable()

    private let lock = NSLock()
    private var entries: [String: NveGetInterfaceFn] = [:]

    public init() {}

    /// Registers a statically linked C `NveGetInterfaceFn` under `symbol`.
    public func register(symbol: String, getInterface: @escaping NveGetInterfaceFn) {
        let key = symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return }
        lock.lock()
        entries[key] = getInterface
        lock.unlock()
    }

    /// Removes any static registration for `symbol`.
    @discardableResult
    public func unregister(symbol: String) -> Bool {
        lock.lock()
        let removed = entries.removeValue(forKey: symbol) != nil
        lock.unlock()
        return removed
    }

    /// Looks up the statically registered entry point for `symbol`.
    public func resolve(symbol: String) -> NveGetInterfaceFn? {
        lock.lock()
        let entry = entries[symbol]
        lock.unlock()
        return entry
    }

    /// Removes all static interface registrations.
    public func clear() {
        lock.lock()
        entries.removeAll()
        lock.unlock()
    }

    /// Sorted list of currently registered symbol names.
    public var registeredSymbols: [String] {
        lock.lock()
        let keys = entries.keys.sorted()
        lock.unlock()
        return keys
    }
}

/// Resolved app entry point (handed to `NveRuntimeApi.attach_app_interface`) paired with its
/// optional owned `dlopen` handle and provenance.
public struct NveResolvedInterface {
    public let getInterface: NveGetInterfaceFn
    public let source: NveInterfaceResolutionSource

    public init(getInterface: @escaping NveGetInterfaceFn, source: NveInterfaceResolutionSource) {
        self.getInterface = getInterface
        self.source = source
    }
}

/// Resolves an in-process app's `nve_get_interface`: explicit, a static archive
/// (`NveStaticInterfaceTable`), or the process image. App `.dylib`s are not resolved here: the
/// runtime loads them (`NveRuntimeSession.loadAppModule`), so the host never `dlopen`s app code.
public enum NveInterfaceResolver {
    /// Resolves `symbol` using precedence:
    /// 1. Explicit `getInterface` C function pointer.
    /// 2. `staticTable` registration.
    /// 3. The process image (`dlopen(nil)` / `dlsym`: an app linked into the host binary).
    public static func resolve(
        symbol: String,
        getInterface: NveGetInterfaceFn? = nil,
        staticTable: NveStaticInterfaceTable = .shared
    ) -> NveResolvedInterface? {
        if let getInterface {
            return NveResolvedInterface(getInterface: getInterface, source: .explicit)
        }
        if let registered = staticTable.resolve(symbol: symbol) {
            return NveResolvedInterface(getInterface: registered, source: .staticTable)
        }
        guard let processHandle = dlopen(nil, RTLD_NOW),
              let sym = dlsym(processHandle, symbol)
        else {
            return nil
        }
        return NveResolvedInterface(getInterface: unsafeBitCast(sym, to: NveGetInterfaceFn.self), source: .processSymbol)
    }
}
