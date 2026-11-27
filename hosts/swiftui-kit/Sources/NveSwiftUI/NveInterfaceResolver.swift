import CNveAbi
import Darwin
import Foundation

// MARK: - Static + Dynamic `nve_get_interface` Resolver

/// Origin of a resolved `nve_get_interface` entry point.
public enum NveInterfaceResolutionSource: Equatable, Sendable {
    /// Caller supplied an explicit `NveGetInterfaceFn` C function pointer.
    case explicit
    /// Resolved from `NveStaticInterfaceTable` (App Store static `lib*.a` archives on iOS/tvOS/watchOS/visionOS).
    case staticTable
    /// Dynamically loaded from an explicit `.dylib` path via `dlopen` / `dlsym`.
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
    public let dylibHandle: UnsafeMutableRawPointer?
    public let source: NveInterfaceResolutionSource

    public init(getInterface: @escaping NveGetInterfaceFn, dylibHandle: UnsafeMutableRawPointer? = nil, source: NveInterfaceResolutionSource) {
        self.getInterface = getInterface
        self.dylibHandle = dylibHandle
        self.source = source
    }
}

/// Resolves an app's `nve_get_interface` across static archives (`NveStaticInterfaceTable`) and
/// dynamic libraries (`dlopen`/`dlsym`), preferring explicit and static registrations over `dlopen`.
public enum NveInterfaceResolver {
    /// Resolves `symbol` using precedence:
    /// 1. Explicit `getInterface` C function pointer.
    /// 2. `staticTable` registration (when `dylibPath` is nil or empty).
    /// 3. Dynamic `dlopen`/`dlsym` from `dylibPath` (dev hot reload / macOS) or process image.
    public static func resolve(
        symbol: String,
        getInterface: NveGetInterfaceFn? = nil,
        dylibPath: String? = nil,
        staticTable: NveStaticInterfaceTable = .shared
    ) -> NveResolvedInterface? {
        if let getInterface {
            return NveResolvedInterface(getInterface: getInterface, source: .explicit)
        }

        let hasDylibPath = dylibPath.map { !$0.isEmpty } ?? false
        if !hasDylibPath, let registered = staticTable.resolve(symbol: symbol) {
            return NveResolvedInterface(getInterface: registered, source: .staticTable)
        }

        if let dylibPath, !dylibPath.isEmpty {
            guard let lib = dlopen(dylibPath, RTLD_NOW | RTLD_LOCAL) else {
                return nil
            }
            guard let sym = dlsym(lib, symbol) else {
                dlclose(lib)
                return nil
            }
            return NveResolvedInterface(
                getInterface: unsafeBitCast(sym, to: NveGetInterfaceFn.self), dylibHandle: lib, source: .dynamicLibrary)
        }

        guard let processHandle = dlopen(nil, RTLD_NOW),
              let sym = dlsym(processHandle, symbol)
        else {
            return nil
        }
        return NveResolvedInterface(getInterface: unsafeBitCast(sym, to: NveGetInterfaceFn.self), source: .processSymbol)
    }
}
