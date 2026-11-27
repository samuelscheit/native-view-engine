import Foundation
import SwiftUI

/// SwiftUI `View` hosting an NVE app (`nve_get_interface`) on a `libnve_runtime` surface on every
/// Apple platform: paints the runtime scene with ``NveSceneCanvasView``, negotiates intrinsic size
/// through the app's `NveLayoutApi::measure`, pushes props (`set_props`, deduplicated), and
/// forwards outbound app events to `onEvent`.
public struct NveSurfaceRepresentable: View {
    private enum PropsPayload {
        case dynamic([String: Any])
        case direct((NveSurfaceController) -> Bool)
    }

    public let componentName: String
    public let vtableSymbol: String
    private let propsPayload: PropsPayload
    private let externalController: NveSurfaceController?
    private let onEvent: ((NveEvent) -> Void)?

    @StateObject private var owned = OwnedController()

    public init(
        componentName: String,
        vtableSymbol: String = "nve_get_interface",
        props: [String: Any] = [:],
        controller: NveSurfaceController? = nil,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.componentName = componentName
        self.vtableSymbol = vtableSymbol
        propsPayload = .dynamic(props)
        externalController = controller
        self.onEvent = onEvent
    }

    /// Typed props fast path (`NveDirectEncodable & Equatable`): no `[String: Any]` boxing.
    public init<P: NveDirectEncodable & Equatable>(
        componentName: String,
        vtableSymbol: String = "nve_get_interface",
        directProps: P,
        controller: NveSurfaceController? = nil,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.componentName = componentName
        self.vtableSymbol = vtableSymbol
        propsPayload = .direct { $0.setDirectProps(directProps) }
        externalController = controller
        self.onEvent = onEvent
    }

    public var body: some View {
        if let active = externalController ?? owned.controller(symbol: vtableSymbol) {
            NveIntrinsicLayout(controller: active) {
                NveSceneCanvasView(controller: active)
            }
            .accessibilityLabel(Text(componentName))
            .onAppear { sync(active) }
            .onChange(of: PropsToken()) { _ in sync(active) }
        } else {
            Color.clear.accessibilityLabel(Text(componentName))
        }
    }

    /// Props are applied after the view update (the runtime publishes a frame synchronously).
    private func sync(_ active: NveSurfaceController) {
        let payload = propsPayload
        let onEvent = onEvent
        DispatchQueue.main.async {
            active.onEvent = onEvent
            switch payload {
            case let .dynamic(props): _ = active.setProps(props)
            case let .direct(apply): _ = apply(active)
            }
        }
    }
}

/// Changes on every body evaluation so props are re-synced (the controller deduplicates).
private struct PropsToken: Equatable {
    private let id = UUID()
}

private final class OwnedController: ObservableObject {
    private var cached: NveSurfaceController?
    private var failed = false

    func controller(symbol: String) -> NveSurfaceController? {
        if cached == nil && !failed {
            cached = try? NveSurfaceController(vtableSymbol: symbol)
            failed = cached == nil
        }
        return cached
    }
}

/// Sizes its content with the app's `NveLayoutApi::measure` (SwiftUI `sizeThatFits`).
private struct NveIntrinsicLayout: Layout {
    let controller: NveSurfaceController

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        controller.measure(proposal: CGSize(width: proposal.width ?? .infinity, height: proposal.height ?? .infinity))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for subview in subviews {
            subview.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
        }
    }
}
