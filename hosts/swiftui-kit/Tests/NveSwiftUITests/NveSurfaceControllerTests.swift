import CNveAbi
import Foundation
import NveSwiftUI
import SwiftUI
import XCTest

#if canImport(AppKit)
import AppKit
#endif

// MARK: - Tests

final class NveSurfaceControllerTests: XCTestCase {
    override func tearDown() {
        MockAbiRegistry.shared.resetNextCreate()
        super.tearDown()
    }

    private func makeMountedController(
        surfaceId: UInt32 = 42,
        backend: NveRenderBackend = .fabric,
        getInterface: NveGetInterfaceFn = mockFullInterface
    ) -> (NveSurfaceController, MockSurfaceState) {
        var captured: MockSurfaceState?
        MockAbiRegistry.shared.configureNextCreate { state in
            captured = state
        }
        let controller = NveSurfaceController(
            surfaceId: surfaceId,
            backend: backend,
            getInterface: getInterface
        )
        return (controller, captured!)
    }

    func testLifecycleMountDestroyReattachAndDeinit() {
        let c1 = NveSurfaceController(vtableSymbol: "nve_nonexistent_symbol_1")
        let c2 = NveSurfaceController(vtableSymbol: "nve_nonexistent_symbol_2")
        XCTAssertEqual(c2.surfaceId, c1.surfaceId + 1)
        XCTAssertFalse(c1.isMounted)
        XCTAssertFalse(c2.isMounted)

        var firstState: MockSurfaceState?
        var secondState: MockSurfaceState?
        do {
            let (controller, state1) = makeMountedController(surfaceId: 77, backend: .skia)
            firstState = state1
            XCTAssertTrue(controller.isMounted)
            XCTAssertEqual(controller.surfaceId, 77)
            XCTAssertEqual(controller.backend, .skia)
            XCTAssertEqual(state1.surfaceId, 77)

            MockAbiRegistry.shared.configureNextCreate { secondState = $0 }
            controller.attach(getInterface: mockFullInterface)
            XCTAssertTrue(state1.destroyed)
            XCTAssertEqual(state1.destroyCount, 1)
            XCTAssertTrue(controller.isMounted)
            XCTAssertNotNil(secondState)
            XCTAssertFalse(secondState?.destroyed ?? true)
        }
        XCTAssertEqual(firstState?.destroyCount, 1)
        XCTAssertTrue(secondState?.destroyed ?? false)
        XCTAssertEqual(secondState?.destroyCount, 1)

        let (controller, state) = makeMountedController(surfaceId: 88)
        controller.destroy()
        controller.destroy()
        XCTAssertFalse(controller.isMounted)
        XCTAssertEqual(state.destroyCount, 1)
    }

    func testAttachErrorStatesAndUnmountedSafeNoOps() {
        let missingSurface = NveSurfaceController(
            surfaceId: 10,
            getInterface: mockMissingSurfaceInterface
        )
        XCTAssertFalse(missingSurface.isMounted)

        let nilCreateFn = NveSurfaceController(
            surfaceId: 11,
            getInterface: mockNilCreateInterface
        )
        XCTAssertFalse(nilCreateFn.isMounted)

        MockAbiRegistry.shared.configureNextCreate(returnNil: true)
        let nullHandle = NveSurfaceController(
            surfaceId: 12,
            getInterface: mockFullInterface
        )
        XCTAssertFalse(nullHandle.isMounted)

        let badDylib = NveSurfaceController(
            surfaceId: 13,
            vtableSymbol: "nve_get_interface",
            dylibPath: "/nonexistent/path/libnve_missing.dylib"
        )
        XCTAssertFalse(badDylib.isMounted)

        XCTAssertFalse(badDylib.setProps(["title": "offline"]))
        XCTAssertTrue(badDylib.renderFrame().isEmpty)
        XCTAssertFalse(badDylib.applyHotPatch(nvp1DevStatus))
        XCTAssertTrue(badDylib.drainEmittedEvents().isEmpty)
        badDylib.dispatchCommand(name: "noop", args: ["a": 1])
    }

    func testSetPropsDirectDeduplicationDirtyRenderAndEventForwarding() throws {
        let (controller, state) = makeMountedController(surfaceId: 5)
        var forwarded: [NveEvent] = []
        controller.onEvent = { forwarded.append($0) }

        state.pendingOutboundEvents = [
            NveEvent(
                surfaceId: 5, nodeId: 1, timestampUs: 555, kind: .custom, name: "onPropsApplied",
                payload: .binValue(try NveBinValue.map([("applied", .bool(true))]).encode())
            ),
        ]

        XCTAssertTrue(controller.setProps(["b": 2, "a": 1]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(
            NveBinValue.map(state.setPropsDirectHistory[0]),
            NveBinValue.map([("a", .int64(1)), ("b", .int64(2))])
        )
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertEqual(forwarded.count, 1)
        XCTAssertEqual(forwarded[0].name, "onPropsApplied")
        let applied = try forwarded[0].withBinValueView { $0.mapValue(forKey: "applied")?.boolValue }
        XCTAssertEqual(applied, true)

        XCTAssertFalse(controller.setProps(["a": 1, "b": 2]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(state.renderedFrameIds, [1])

        XCTAssertFalse(controller.setProps(["bad": Date()]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)

        state.setPropsReturnValue = 0
        state.pendingOutboundEvents = [
            NveEvent(surfaceId: 5, nodeId: 2, timestampUs: 777, kind: .custom, name: "onCleanAck", payload: .binValue(Data())),
        ]
        XCTAssertFalse(controller.setProps(["a": 2]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 2)
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertEqual(forwarded.count, 2)
        XCTAssertEqual(forwarded[1].name, "onCleanAck")
    }

    func testMissingEntryPointsRejectCleanly() {
        let (controller, state) = makeMountedController(
            surfaceId: 8,
            getInterface: mockNilDirectFnsInterface
        )

        XCTAssertFalse(controller.setProps(["count": 42, "label": "bin"]))
        XCTAssertTrue(state.setPropsDirectHistory.isEmpty)
        XCTAssertFalse(
            controller.dispatchEvent(
                NveEvent(surfaceId: 8, nodeId: 1, timestampUs: 100, kind: .press, payload: .point(x: 4, y: 8))
            )
        )
        controller.dispatchCommand(name: "reset", args: ["animated": true])
        XCTAssertTrue(state.dispatchedDirectEvents.isEmpty)
        XCTAssertTrue(state.renderedFrameIds.isEmpty)
    }

    func testDispatchCommandDirectRenderFrameAndHotPatch() throws {
        let (controller, state) = makeMountedController(surfaceId: 9)
        var receivedEvents: [NveEvent] = []
        controller.onEvent = { receivedEvents.append($0) }

        state.pendingOutboundEvents = [
            NveEvent(
                surfaceId: 9, nodeId: 4, timestampUs: 1001, kind: .custom, name: "onResetComplete",
                payload: .binValue(try NveBinValue.map([("count", .int64(0))]).encode())
            ),
        ]
        controller.dispatchCommand(name: "reset", args: ["animated": true])
        XCTAssertEqual(state.dispatchedDirectEvents.count, 1)
        XCTAssertEqual(state.dispatchedDirectEvents[0].count, 1)
        let ev = state.dispatchedDirectEvents[0][0]
        XCTAssertEqual(ev.table, UInt64(NVE_TABLE_CUSTOM))
        XCTAssertEqual(ev.decoded.surfaceId, 9)
        XCTAssertEqual(ev.decoded.kind, .custom)
        XCTAssertEqual(ev.decoded.name, "reset")
        let animated = try ev.decoded.withBinValueView { $0.mapValue(forKey: "animated")?.boolValue }
        XCTAssertEqual(animated, true)
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertEqual(receivedEvents.count, 1)
        XCTAssertEqual(receivedEvents[0].name, "onResetComplete")

        // Clean command -> does not trigger render.
        state.dispatchEventsReturnValue = 0
        controller.dispatchCommand(name: "ping")
        XCTAssertEqual(state.dispatchedDirectEvents.count, 2)
        XCTAssertEqual(state.renderedFrameIds, [1])

        // Explicit renderFrame increments frame counter and returns slice bytes.
        state.renderOutput = Data([1, 2, 3, 4, 5])
        XCTAssertEqual(controller.renderFrame(), Data([1, 2, 3, 4, 5]))
        XCTAssertEqual(state.renderedFrameIds, [1, 2])

        state.renderOutput = Data()
        XCTAssertTrue(controller.renderFrame().isEmpty)
        XCTAssertEqual(state.renderedFrameIds, [1, 2, 3])

        // Hot patch: dirty renders, clean skips render.
        state.hotPatchReturnValue = 1
        XCTAssertTrue(controller.applyHotPatch(nvp1DevStatus))
        XCTAssertEqual(state.hotPatchHistory, [nvp1DevStatus], "NVP1 bytes forwarded unchanged")
        XCTAssertEqual(state.renderedFrameIds, [1, 2, 3, 4])

        state.hotPatchReturnValue = 0
        XCTAssertFalse(controller.applyHotPatch(Data(nvp1DevStatus.dropLast())))
        XCTAssertEqual(state.renderedFrameIds, [1, 2, 3, 4])
    }

    func testOptionalInterfacesMissingGracefulFallback() {
        let (controller, state) = makeMountedController(
            surfaceId: 33,
            getInterface: mockSurfaceOnlyInterface
        )
        XCTAssertTrue(controller.isMounted)
        XCTAssertFalse(controller.setProps(["k": "v"]))
        XCTAssertTrue(controller.drainEmittedEvents().isEmpty)
        XCTAssertFalse(controller.applyHotPatch(nvp1DevStatus))

        XCTAssertEqual(controller.renderFrame(), state.renderOutput)
        XCTAssertEqual(controller.measure(), CGSize(width: 240, height: 120))
    }

    func testLayoutConstraintNegotiationAndClamping() throws {
        let (controller, state) = makeMountedController(surfaceId: 14)

        state.intrinsicSize = NveIntrinsicSize(width: 180, height: 90)
        let normal = controller.measure(minWidth: 50, minHeight: 40, maxWidth: 300, maxHeight: 200)
        XCTAssertEqual(normal, CGSize(width: 180, height: 90))
        let constraints = try XCTUnwrap(state.lastConstraints)
        XCTAssertEqual(constraints.min_w, 50)
        XCTAssertEqual(constraints.min_h, 40)
        XCTAssertEqual(constraints.max_w, 300)
        XCTAssertEqual(constraints.max_h, 200)

        state.intrinsicSize = NveIntrinsicSize(width: 10, height: 5)
        let clampedUp = controller.measure(minWidth: 80, minHeight: 44, maxWidth: 300, maxHeight: 200)
        XCTAssertEqual(clampedUp, CGSize(width: 80, height: 44))

        state.intrinsicSize = NveIntrinsicSize(width: 900, height: 700)
        let clampedDown = controller.measure(minWidth: 0, minHeight: 0, maxWidth: 320, maxHeight: 160)
        XCTAssertEqual(clampedDown, CGSize(width: 320, height: 160))

        state.intrinsicSize = NveIntrinsicSize(width: 420, height: 210)
        let unbounded = controller.measure(minWidth: 100, minHeight: 50, maxWidth: .infinity, maxHeight: .infinity)
        XCTAssertEqual(unbounded, CGSize(width: 420, height: 210))
        let unboundedConstraints = try XCTUnwrap(state.lastConstraints)
        XCTAssertTrue(unboundedConstraints.max_w.isInfinite)
        XCTAssertTrue(unboundedConstraints.max_h.isInfinite)

        controller.destroy()
        XCTAssertEqual(controller.measure(), CGSize(width: 240, height: 120))
        controller.setFallbackIntrinsicSize(CGSize(width: 310, height: 140))
        XCTAssertEqual(controller.measure(), CGSize(width: 310, height: 140))
        XCTAssertEqual(
            controller.measure(minWidth: 350, minHeight: 200, maxWidth: .infinity, maxHeight: .infinity),
            CGSize(width: 350, height: 200)
        )
        XCTAssertEqual(
            controller.measure(minWidth: 100, minHeight: 50, maxWidth: 200, maxHeight: 80),
            CGSize(width: 200, height: 80)
        )
    }

    func testRepresentableCoordinatorOwnership() {
        let (externalController, state) = makeMountedController(surfaceId: 50)
        do {
            let representable = NveSurfaceRepresentable(
                componentName: "NveCounterCard",
                props: ["count": 1],
                controller: externalController
            )
            let coordinator = representable.makeCoordinator()
            _ = coordinator
        }
        XCTAssertTrue(externalController.isMounted)
        XCTAssertFalse(state.destroyed)

        do {
            let internalRepresentable = NveSurfaceRepresentable(
                componentName: "NveOwnedCard",
                vtableSymbol: "nve_missing_symbol_owned"
            )
            _ = internalRepresentable.makeCoordinator()
        }
    }

    #if canImport(AppKit)
    @MainActor
    func testMacOSHostContainerViewAndRepresentableLayout() {
        let (controller, state) = makeMountedController(surfaceId: 61)
        state.intrinsicSize = NveIntrinsicSize(width: 220, height: 96)

        let container = NveHostContainerView(frame: CGRect(x: 0, y: 0, width: 220, height: 96))
        container.controller = controller
        XCTAssertTrue(container.controller === controller)

        let view = NveSurfaceRepresentable(
            componentName: "NveCounterCard",
            props: ["title": "macOS Test", "count": 3],
            controller: controller
        )
        let hostingView = NSHostingView(rootView: view)
        hostingView.frame = CGRect(x: 0, y: 0, width: 300, height: 200)
        hostingView.layoutSubtreeIfNeeded()

        XCTAssertEqual(hostingView.fittingSize, CGSize(width: 220, height: 96))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(
            NveBinValue.map(state.setPropsDirectHistory[0]),
            NveBinValue.map([
                ("count", .int64(3)),
                ("title", .string("macOS Test")),
            ])
        )
    }

    @MainActor
    func testMacOSRepresentableDirectPropsZeroBoxingPath() {
        struct CardProps: Equatable, NveDirectEncodable {
            var title: String
            var count: Int64
            var directPropCount: Int { 2 }
            func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
                builder.write("title", title)
                builder.write("count", count)
            }
        }

        let (controller, state) = makeMountedController(surfaceId: 62)
        let view = NveSurfaceRepresentable(
            componentName: "NveCounterCard",
            directProps: CardProps(title: "TypedView", count: 11),
            controller: controller
        )
        let hostingView = NSHostingView(rootView: view)
        hostingView.frame = CGRect(x: 0, y: 0, width: 200, height: 100)
        hostingView.layoutSubtreeIfNeeded()

        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(
            NveBinValue.map(state.setPropsDirectHistory[0]),
            NveBinValue.map([("title", .string("TypedView")), ("count", .int64(11))])
        )
    }
    #endif

    func testSetDirectPropsEquatableDeduplicationCleanUpdateFallbackAndCacheInvalidation() throws {
        struct TypedProps: Equatable, NveDirectEncodable {
            var count: Int64
            var label: String
            var directPropCount: Int { 2 }
            func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
                builder.write("count", count)
                builder.write("label", label)
            }
        }

        let (controller, state) = makeMountedController(surfaceId: 71)
        let p1 = TypedProps(count: 1, label: "first")
        XCTAssertTrue(controller.setDirectProps(p1))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(state.renderedFrameIds, [1])

        // Identical Equatable value skips FFI & render completely.
        XCTAssertFalse(controller.setDirectProps(TypedProps(count: 1, label: "first")))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(state.renderedFrameIds, [1])

        // Clean update (return 0) still updates dedup state and skips render.
        state.setPropsReturnValue = 0
        let p2 = TypedProps(count: 2, label: "clean")
        XCTAssertFalse(controller.setDirectProps(p2))
        XCTAssertEqual(state.setPropsDirectHistory.count, 2)
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertFalse(controller.setDirectProps(p2))
        XCTAssertEqual(state.setPropsDirectHistory.count, 2)

        // Switching to dynamic setProps invalidates direct cache so re-sending p2 succeeds.
        state.setPropsReturnValue = 1
        XCTAssertTrue(controller.setProps(["count": 99]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 3)
        XCTAssertTrue(controller.setDirectProps(p2))
        XCTAssertEqual(state.setPropsDirectHistory.count, 4)

        let (missingDirectCtrl, missingDirectState) = makeMountedController(
            surfaceId: 72,
            getInterface: mockNilDirectFnsInterface
        )
        XCTAssertFalse(missingDirectCtrl.setDirectProps(TypedProps(count: 55, label: "no-fallback")))
        XCTAssertTrue(missingDirectState.setPropsDirectHistory.isEmpty)
    }
}
