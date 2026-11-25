import CNveAbi
import Darwin
import Foundation

// MARK: - Static + Dynamic `nve_get_interface` Resolver

/// Origin of a resolved `NveModule` interface table.
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

/// Entry stored in `NveStaticInterfaceTable` for static-linked Rust libraries or Swift test doubles.
public enum NveStaticInterfaceEntry: Sendable {
    case cFunction(NveGetInterfaceFn)
    case closure(@Sendable (UInt32) -> UnsafeRawPointer?)
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
    private var entries: [String: NveStaticInterfaceEntry] = [:]

    public init() {}

    /// Registers a statically linked C `NveGetInterfaceFn` under `symbol`.
    public func register(symbol: String, getInterface: @escaping NveGetInterfaceFn) {
        let key = symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return }
        lock.lock()
        entries[key] = .cFunction(getInterface)
        lock.unlock()
    }

    /// Registers a closure-backed interface resolver under `symbol`.
    public func register(
        symbol: String,
        resolver: @escaping @Sendable (UInt32) -> UnsafeRawPointer?
    ) {
        let key = symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return }
        lock.lock()
        entries[key] = .closure(resolver)
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

    /// Looks up a registered static interface entry for `symbol`.
    public func resolve(symbol: String) -> NveStaticInterfaceEntry? {
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

/// Resolved module table paired with optional owned `dlopen` handle and provenance.
public struct NveResolvedInterface {
    public let module: NveModule
    public let dylibHandle: UnsafeMutableRawPointer?
    public let source: NveInterfaceResolutionSource

    public init(
        module: NveModule,
        dylibHandle: UnsafeMutableRawPointer? = nil,
        source: NveInterfaceResolutionSource
    ) {
        self.module = module
        self.dylibHandle = dylibHandle
        self.source = source
    }
}

/// Resolves an `NveModule` across static archives (`NveStaticInterfaceTable`) and dynamic
/// libraries (`dlopen`/`dlsym`), preferring explicit and static registrations over `dlopen`.
public enum NveInterfaceResolver {
    /// Resolves all 5 optional NVE C-ABI vtable pointers from a static entry.
    public static func resolveModule(using entry: NveStaticInterfaceEntry) -> NveModule {
        switch entry {
        case let .cFunction(fn):
            return nve_resolve_module(fn)
        case let .closure(resolver):
            return NveModule(
                surface: resolver(UInt32(NVE_IFACE_SURFACE))?.assumingMemoryBound(to: NveSurfaceApi.self),
                layout: resolver(UInt32(NVE_IFACE_LAYOUT))?.assumingMemoryBound(to: NveLayoutApi.self),
                component: resolver(UInt32(NVE_IFACE_COMPONENT))?.assumingMemoryBound(to: NveComponentApi.self),
                dev: resolver(UInt32(NVE_IFACE_DEV))?.assumingMemoryBound(to: NveDevApi.self),
                host: resolver(UInt32(NVE_IFACE_HOST))?.assumingMemoryBound(to: NveHostApi.self)
            )
        }
    }

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
            return NveResolvedInterface(
                module: nve_resolve_module(getInterface),
                dylibHandle: nil,
                source: .explicit
            )
        }

        let hasDylibPath = dylibPath.map { !$0.isEmpty } ?? false
        if !hasDylibPath, let staticEntry = staticTable.resolve(symbol: symbol) {
            return NveResolvedInterface(
                module: resolveModule(using: staticEntry),
                dylibHandle: nil,
                source: .staticTable
            )
        }

        if let dylibPath, !dylibPath.isEmpty {
            guard let lib = dlopen(dylibPath, RTLD_NOW | RTLD_LOCAL) else {
                return nil
            }
            guard let sym = dlsym(lib, symbol) else {
                dlclose(lib)
                return nil
            }
            let getIface = unsafeBitCast(sym, to: NveGetInterfaceFn.self)
            return NveResolvedInterface(
                module: nve_resolve_module(getIface),
                dylibHandle: lib,
                source: .dynamicLibrary
            )
        }

        guard let processHandle = dlopen(nil, RTLD_NOW),
              let sym = dlsym(processHandle, symbol)
        else {
            return nil
        }
        let getIface = unsafeBitCast(sym, to: NveGetInterfaceFn.self)
        return NveResolvedInterface(
            module: nve_resolve_module(getIface),
            dylibHandle: nil,
            source: .processSymbol
        )
    }
}
