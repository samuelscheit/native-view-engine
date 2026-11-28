import Foundation
import NveHost
import SwiftUI

/// Deterministic prop-content equality token replacing per-render `UUID()` allocations so
/// unrelated SwiftUI body evaluations never schedule redundant `setProps` FFI calls.
internal enum NvePropsToken: Equatable {
    case dynamic(NveBinValue?)
    case direct(NveEquatableDirectProps)

    @discardableResult
    func apply(to controller: NveSurfaceController) -> Bool {
        switch self {
        case let .dynamic(.map(entries)):
            return controller.setProps(entries)
        case .dynamic:
            return false
        case let .direct(direct):
            return direct.apply(to: controller)
        }
    }
}

/// Zero-closure type-erased `NveDirectEncodable & Equatable` wrapper.
private protocol NveAnyDirectProps {
    func isEqual(to other: any NveAnyDirectProps) -> Bool
    func apply(to controller: NveSurfaceController) -> Bool
}

private struct NveDirectPropsStorage<P: NveDirectEncodable & Equatable>: NveAnyDirectProps {
    let props: P

    func isEqual(to other: any NveAnyDirectProps) -> Bool {
        (other as? NveDirectPropsStorage<P>)?.props == props
    }

    func apply(to controller: NveSurfaceController) -> Bool {
        controller.setDirectProps(props)
    }
}

internal struct NveEquatableDirectProps: Equatable {
    private let storage: any NveAnyDirectProps

    init<P: NveDirectEncodable & Equatable>(_ props: P) {
        storage = NveDirectPropsStorage(props: props)
    }

    func apply(to controller: NveSurfaceController) -> Bool {
        storage.apply(to: controller)
    }

    static func == (lhs: NveEquatableDirectProps, rhs: NveEquatableDirectProps) -> Bool {
        lhs.storage.isEqual(to: rhs.storage)
    }
}

/// SwiftUI `View` hosting an NVE app (`nve_get_interface`) on a `libnve_runtime` surface on every
/// Apple platform: paints the runtime scene with ``NveSceneCanvasView`` (which mounts platform
/// editable `TextInput` overlays), negotiates Model A intrinsic sizing and text baselines via
/// ABI v6 `measure_surface`, deduplicates prop pushes deterministically, and forwards outbound
/// app events to `onEvent`.
public struct NveSurfaceRepresentable: View {
    public let componentName: String
    public let vtableSymbol: String
    internal let propsToken: NvePropsToken
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
        let entries = try? NveBinValue.canonicalEntries(from: props)
        propsToken = .dynamic(entries.map(NveBinValue.map))
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
        propsToken = .direct(NveEquatableDirectProps(directProps))
        externalController = controller
        self.onEvent = onEvent
    }

    public var body: some View {
        if let active = externalController ?? owned.controller(symbol: vtableSymbol) {
            NveSurfaceBody(
                controller: active,
                store: active.store,
                componentName: componentName,
                propsToken: propsToken,
                onEvent: onEvent
            )
        } else {
            Color.clear.accessibilityLabel(Text(componentName))
        }
    }
}

private struct NveSurfaceBody: View {
    @ObservedObject var controller: NveSurfaceController
    @ObservedObject var store: NveSceneStore
    let componentName: String
    let propsToken: NvePropsToken
    let onEvent: ((NveEvent) -> Void)?

    @StateObject private var eventSink = EventHandlerBox()

    var body: some View {
        let _ = eventSink.update(onEvent)

        NveIntrinsicLayout(
            controller: controller,
            intrinsicRevision: controller.intrinsicSizeRevision,
            frameId: store.frameId
        ) {
            NveSceneCanvasView(controller: controller)
        }
        .accessibilityLabel(Text(componentName))
        .onAppear {
            attachEventForwarder()
            syncProps(propsToken)
        }
        .onDisappear {
            eventSink.update(nil)
            controller.onEvent = nil
        }
        .onChange(of: propsToken) { nextToken in
            syncProps(nextToken)
        }
    }

    private func attachEventForwarder() {
        controller.onEvent = { [weak sink = eventSink] event in sink?.handler?(event) }
    }

    /// Props are applied after the view update (the runtime publishes a frame synchronously).
    private func syncProps(_ token: NvePropsToken) {
        let active = controller
        let sink = eventSink
        DispatchQueue.main.async { [weak sink] in
            active.onEvent = { [weak sink] event in sink?.handler?(event) }
            _ = token.apply(to: active)
        }
    }
}

private final class EventHandlerBox: ObservableObject {
    var handler: ((NveEvent) -> Void)?
    func update(_ next: ((NveEvent) -> Void)?) { handler = next }
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

/// Model A SwiftUI `Layout` delegating `sizeThatFits` and `.firstTextBaseline` / `.lastTextBaseline`
/// `explicitAlignment` to Runtime ABI v6 `measure_surface`.
internal struct NveIntrinsicLayout: Layout {
    struct Cache {
        var lastProposal: ProposedViewSize?
        var lastRevision: UInt64 = .max
        var lastFrameId: UInt64 = .max
        var measurement: NveSurfaceMeasurement?
    }

    let controller: NveSurfaceController
    let intrinsicRevision: UInt64
    let frameId: UInt64

    init(controller: NveSurfaceController, intrinsicRevision: UInt64, frameId: UInt64 = 0) {
        self.controller = controller
        self.intrinsicRevision = intrinsicRevision
        self.frameId = frameId
    }

    func makeCache(subviews: Subviews) -> Cache {
        Cache()
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) -> CGSize {
        measure(proposal: proposal, cache: &cache).size
    }

    func explicitAlignment(
        of guide: HorizontalAlignment,
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout Cache
    ) -> CGFloat? {
        nil
    }

    func explicitAlignment(
        of guide: VerticalAlignment,
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout Cache
    ) -> CGFloat? {
        baselineCoordinate(for: guide, in: bounds, proposal: proposal, cache: &cache)
    }

    func baselineCoordinate(
        for guide: VerticalAlignment,
        in bounds: CGRect,
        proposal: ProposedViewSize,
        cache: inout Cache
    ) -> CGFloat? {
        guard guide == .firstTextBaseline || guide == .lastTextBaseline else { return nil }
        let m = measure(proposal: proposal, cache: &cache)
        let offset = guide == .firstTextBaseline ? m.firstBaseline : m.lastBaseline
        guard let offset, offset.isFinite else { return nil }
        return bounds.minY + offset
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) {
        for subview in subviews {
            subview.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
        }
    }

    func measure(proposal: ProposedViewSize, cache: inout Cache) -> NveSurfaceMeasurement {
        if let cached = cache.measurement,
           cache.lastRevision == intrinsicRevision,
           cache.lastFrameId == frameId,
           cache.lastProposal == proposal
        {
            return cached
        }
        let result = controller.measureSurface(proposal: proposal)
        cache.lastProposal = proposal
        cache.lastRevision = intrinsicRevision
        cache.lastFrameId = frameId
        cache.measurement = result
        return result
    }
}
