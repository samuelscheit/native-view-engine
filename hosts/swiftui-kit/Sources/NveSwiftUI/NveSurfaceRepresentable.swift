import Foundation
import SwiftUI

#if canImport(UIKit) && !os(watchOS)
import UIKit
public typealias NvePlatformView = UIView
#elseif canImport(AppKit)
import AppKit
public typealias NvePlatformView = NSView
#endif

// MARK: - Native Host Container View

#if (canImport(UIKit) && !os(watchOS)) || canImport(AppKit)
public final class NveHostContainerView: NvePlatformView {
    public var controller: NveSurfaceController?

    public override init(frame: CGRect) {
        super.init(frame: frame)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }
}
#endif

// MARK: - SwiftUI Representable (`UIViewRepresentable` / `NSViewRepresentable` / Pure-SwiftUI watchOS)

/// Cross-platform SwiftUI `View` hosting an NVE Rust component backed by `nve_get_interface`.
///
/// Supports:
/// - `sizeThatFits` intrinsic layout negotiation on iOS 16+ and macOS 13+
/// - Direct typed prop updates via `NveComponentApi.set_props`
/// - Imperative commands via `NveSurfaceController`
/// - Strongly typed outbound event callbacks via `onEvent`
public struct NveSurfaceRepresentable {
    private enum PropsPayload {
        case dynamic([String: Any])
        case direct((NveSurfaceController) -> Bool)
    }

    public let componentName: String
    public let vtableSymbol: String
    public let backend: NveRenderBackend
    public let props: [String: Any]
    private let propsPayload: PropsPayload
    public let controller: NveSurfaceController?
    public let onEvent: ((NveEvent) -> Void)?

    public init(
        componentName: String,
        vtableSymbol: String = "nve_get_interface",
        backend: NveRenderBackend = .fabric,
        props: [String: Any] = [:],
        controller: NveSurfaceController? = nil,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.componentName = componentName
        self.vtableSymbol = vtableSymbol
        self.backend = backend
        self.props = props
        self.propsPayload = .dynamic(props)
        self.controller = controller
        self.onEvent = onEvent
    }

    /// Zero-copy typed fast path for strongly typed `NveDirectEncodable & Equatable` prop structs:
    /// skips `[String: Any]` boxing, `NveBinValue` heap-tree construction, and key sorting on every
    /// SwiftUI update.
    public init<P: NveDirectEncodable & Equatable>(
        componentName: String,
        vtableSymbol: String = "nve_get_interface",
        backend: NveRenderBackend = .fabric,
        directProps: P,
        controller: NveSurfaceController? = nil,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.componentName = componentName
        self.vtableSymbol = vtableSymbol
        self.backend = backend
        self.props = [:]
        self.propsPayload = .direct { active in active.setDirectProps(directProps) }
        self.controller = controller
        self.onEvent = onEvent
    }

    public final class Coordinator {
        let controller: NveSurfaceController
        let ownsController: Bool

        init(
            externalController: NveSurfaceController?,
            vtableSymbol: String,
            backend: NveRenderBackend
        ) {
            if let externalController {
                self.controller = externalController
                self.ownsController = false
            } else {
                self.controller = NveSurfaceController(
                    vtableSymbol: vtableSymbol,
                    backend: backend
                )
                self.ownsController = true
            }
        }

        deinit {
            if ownsController {
                controller.destroy()
            }
        }
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(
            externalController: controller,
            vtableSymbol: vtableSymbol,
            backend: backend
        )
    }

    private func applyProps(to active: NveSurfaceController) {
        active.onEvent = onEvent
        switch propsPayload {
        case let .dynamic(dict):
            _ = active.setProps(dict)
        case let .direct(apply):
            _ = apply(active)
        }
    }

    #if (canImport(UIKit) && !os(watchOS)) || canImport(AppKit)
    private func sync(view: NveHostContainerView, coordinator: Coordinator) {
        let active = coordinator.controller
        view.controller = active
        applyProps(to: active)
    }
    #endif

    private func computeSizeThatFits(
        _ proposal: ProposedViewSize,
        coordinator: Coordinator
    ) -> CGSize {
        let maxW = proposal.width ?? .infinity
        let maxH = proposal.height ?? .infinity
        return coordinator.controller.measure(
            minWidth: 0,
            minHeight: 0,
            maxWidth: maxW,
            maxHeight: maxH
        )
    }
}

#if canImport(UIKit) && !os(watchOS)
extension NveSurfaceRepresentable: UIViewRepresentable {
    public func makeUIView(context: Context) -> NveHostContainerView {
        let view = NveHostContainerView(frame: .zero)
        sync(view: view, coordinator: context.coordinator)
        return view
    }

    public func updateUIView(_ uiView: NveHostContainerView, context: Context) {
        sync(view: uiView, coordinator: context.coordinator)
    }

    public func sizeThatFits(
        _ proposal: ProposedViewSize,
        uiView: NveHostContainerView,
        context: Context
    ) -> CGSize? {
        computeSizeThatFits(proposal, coordinator: context.coordinator)
    }

    public static func dismantleUIView(_ uiView: NveHostContainerView, coordinator: Coordinator) {
        uiView.controller = nil
        if coordinator.ownsController {
            coordinator.controller.destroy()
        }
    }
}
#elseif canImport(AppKit)
extension NveSurfaceRepresentable: NSViewRepresentable {
    public func makeNSView(context: Context) -> NveHostContainerView {
        let view = NveHostContainerView(frame: .zero)
        sync(view: view, coordinator: context.coordinator)
        return view
    }

    public func updateNSView(_ nsView: NveHostContainerView, context: Context) {
        sync(view: nsView, coordinator: context.coordinator)
    }

    public func sizeThatFits(
        _ proposal: ProposedViewSize,
        nsView: NveHostContainerView,
        context: Context
    ) -> CGSize? {
        computeSizeThatFits(proposal, coordinator: context.coordinator)
    }

    public static func dismantleNSView(_ nsView: NveHostContainerView, coordinator: Coordinator) {
        nsView.controller = nil
        if coordinator.ownsController {
            coordinator.controller.destroy()
        }
    }
}
#elseif os(watchOS)
/// Pure-SwiftUI watchOS fallback avoiding `UIViewRepresentable` (unavailable on watchOS).
extension NveSurfaceRepresentable: View {
    public var body: some View {
        NveWatchSurfaceHostView(
            componentName: componentName,
            vtableSymbol: vtableSymbol,
            backend: backend,
            externalController: controller,
            syncProps: { active in applyProps(to: active) }
        )
    }
}

private struct NveWatchSurfaceHostView: View {
    let componentName: String
    let vtableSymbol: String
    let backend: NveRenderBackend
    let externalController: NveSurfaceController?
    let syncProps: (NveSurfaceController) -> Void

    @Environment(\.displayScale) private var displayScale
    @StateObject private var store = NveSceneStore()
    @State private var ownedController: NveSurfaceController?

    private var activeController: NveSurfaceController? {
        externalController ?? ownedController
    }

    var body: some View {
        NveSceneCanvasView(
            store: store,
            surfaceId: activeController?.surfaceId ?? 1,
            controller: activeController
        )
        .accessibilityLabel(Text(componentName))
        .onAppear {
            if externalController == nil && ownedController == nil {
                ownedController = NveSurfaceController(vtableSymbol: vtableSymbol, backend: backend)
            }
            if let active = activeController {
                syncProps(active)
                _ = displayScale
            }
        }
        .onDisappear {
            ownedController?.destroy()
            ownedController = nil
        }
    }
}
#endif
