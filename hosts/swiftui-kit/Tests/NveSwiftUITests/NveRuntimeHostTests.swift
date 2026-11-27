import CNveAbi
import NveHost
import SwiftUI
import XCTest
@testable import NveSwiftUI

private func noApp(_ id: UInt32) -> UnsafeRawPointer? { nil }

private final class RecordingMeasurer: NveTextMeasuring {
    var calls: [(String, CGFloat, UInt16, CGFloat)] = []
    func measure(text: String, fontSize: CGFloat, fontWeight: UInt16, maxWidth: CGFloat) -> CGSize {
        calls.append((text, fontSize, fontWeight, maxWidth))
        return CGSize(width: 12.5, height: 30)
    }
}

final class NveRuntimeHostTests: XCTestCase {
    private var m: MockRuntime { MockRuntime.shared }

    override func setUp() {
        super.setUp()
        MockRuntime.shared.reset()
    }

    /// root(0,0,200,300) → [card(bg red, radius 8, z 1), title(text "Hello", inherits typography),
    ///   scroll(0,100,200,100) → [row(0,100,200,400)], input(secure), image]
    private func buildScene() -> (root: NodeId, card: NodeId, title: NodeId, scroll: NodeId, input: NodeId, image: NodeId) {
        var s = SceneBuilder()
        let root = s.node(rect: CGRect(x: 0, y: 0, width: 200, height: 300))
        let card = s.node(parent: root, rect: CGRect(x: 10, y: 10, width: 50, height: 50))
        let title = s.node(parent: root, rect: CGRect(x: 0, y: 60, width: 200, height: 30), generation: 3)
        let scroll = s.node(parent: root, rect: CGRect(x: 0, y: 100, width: 200, height: 100))
        let row = s.node(parent: scroll, rect: CGRect(x: 0, y: 100, width: 200, height: 400))
        let input = s.node(parent: root, rect: CGRect(x: 0, y: 210, width: 200, height: 40))
        let image = s.node(parent: root, rect: CGRect(x: 0, y: 250, width: 50, height: 50))
        s.setChildren(root, [card, title, scroll, input, image])
        s.setChildren(scroll, [row])
        s.visual(root) { SceneBuilder.put(&$0, NveTables.Visual.background, SceneBuilder.rgba(255, 255, 255)) }
        s.visual(card) {
            SceneBuilder.put(&$0, NveTables.Visual.background, SceneBuilder.rgba(255, 0, 0))
            SceneBuilder.put(&$0, NveTables.Visual.borderRadius, Float(8))
            SceneBuilder.put(&$0, NveTables.Visual.zIndex, UInt32(1))
        }
        s.visual(row) { SceneBuilder.put(&$0, NveTables.Visual.opacity, Float(0.5)) }
        s.typography(root) {
            SceneBuilder.put(&$0, NveTables.Typography.set, NveTypographySet.color | NveTypographySet.fontSize)
            SceneBuilder.put(&$0, NveTables.Typography.color, SceneBuilder.rgba(0, 0, 255))
            SceneBuilder.put(&$0, NveTables.Typography.fontSize, Float(24))
            SceneBuilder.put(&$0, NveTables.Typography.fontSize, Float(24))
        }
        s.typography(title) {
            SceneBuilder.put(&$0, NveTables.Typography.set, NveTypographySet.fontWeight | NveTypographySet.textAlign)
            SceneBuilder.put(&$0, NveTables.Typography.fontWeight, UInt32(700))
            SceneBuilder.put(&$0, NveTables.Typography.textAlign, NveTextAlign.center.rawValue)
            // Not in `set`: must NOT override the inherited 24pt.
            SceneBuilder.put(&$0, NveTables.Typography.fontSize, Float(9))
        }
        s.text(title, "Hello")
        s.sparse(NveTables.Scroll.key, words: NveTables.Scroll.words, defaults: NveTables.Scroll.defaults, scroll)
        let value = s.str("hunter2")
        let placeholder = s.str("Password")
        s.sparse(NveTables.TextInput.key, words: NveTables.TextInput.words, defaults: NveTables.TextInput.defaults, input) {
            SceneBuilder.put(&$0, NveTables.TextInput.value, value)
            SceneBuilder.put(&$0, NveTables.TextInput.placeholder, placeholder)
            SceneBuilder.put(&$0, NveTables.TextInput.secure, UInt32(1))
        }
        let alt = s.str("Logo")
        s.sparse(NveTables.Image.key, words: NveTables.Image.words, defaults: NveTables.Image.defaults, image) {
            SceneBuilder.put(&$0, NveTables.Image.src, UInt32(9))
            SceneBuilder.put(&$0, NveTables.Image.alt, alt)
            SceneBuilder.put(&$0, NveTables.Image.fit, NveImageFit.contain.rawValue)
        }
        let label = s.str("Brand logo")
        s.sparse(NveTables.Accessibility.key, words: NveTables.Accessibility.words, defaults: NveTables.Accessibility.defaults, image) {
            SceneBuilder.put(&$0, NveTables.Accessibility.role, NveAriaRole.image.rawValue)
            SceneBuilder.put(&$0, NveTables.Accessibility.label, label)
        }
        s.sparse(NveTables.Interaction.key, words: NveTables.Interaction.words, defaults: NveTables.Interaction.defaults, card)
        m.scene = s
        return (root, card, title, scroll, input, image)
    }

    private func makeSession() throws -> NveRuntimeSession {
        try NveRuntimeSession(api: MockRuntime.api, size: CGSize(width: 200, height: 300))
    }

    // MARK: Session

    func testSessionValidatesTheVtableAndOwnsTheHandle() throws {
        XCTAssertThrowsError(try NveRuntimeSession(api: nil)) { XCTAssertEqual($0 as? NveRuntimeError, .runtimeUnavailable) }
        for corrupt in [
            { (a: inout NveRuntimeApi) in a.abi_version += 1 },
            { (a: inout NveRuntimeApi) in a.schema_hash ^= 1 },
            { (a: inout NveRuntimeApi) in a.struct_size -= 8 },
            { (a: inout NveRuntimeApi) in a.resource_payload = nil },
            { (a: inout NveRuntimeApi) in a.set_text_measurer = nil },
            { (a: inout NveRuntimeApi) in a.last_error = nil },
        ] {
            var copy = MockRuntime.api.pointee
            corrupt(&copy)
            try withUnsafePointer(to: copy) { p in
                XCTAssertFalse(NveRuntimeSession.isCompatible(p))
                XCTAssertThrowsError(try NveRuntimeSession(api: p)) { XCTAssertEqual($0 as? NveRuntimeError, .abiMismatch) }
            }
        }
        XCTAssertEqual(m.creates, 0)
        do {
            let session = try makeSession()
            XCTAssertTrue(try session.tick(dt: 0.016))
            m.tickChanged = false
            XCTAssertFalse(try session.tick())
        }
        XCTAssertEqual([m.creates, m.destroys], [1, 1])
    }

    func testStatusErrorsCarryLastErrorAndSceneBorrowsAreBalanced() throws {
        let session = try makeSession()
        m.failNextWith = Int32(NveRuntimeError.busy)
        m.lastError = "scene busy"
        XCTAssertThrowsError(try session.tick()) {
            XCTAssertEqual($0 as? NveRuntimeError, .status(code: Int32(NveRuntimeError.busy), message: "scene busy"))
        }
        _ = buildScene()
        let count = try session.withScene { $0.nodeCapacity }
        XCTAssertEqual(count, 8)
        m.schema ^= 1
        XCTAssertThrowsError(try session.withScene { _ in }) {
            guard case .scene(.schemaMismatch)? = $0 as? NveRuntimeError else { return XCTFail("\($0)") }
        }
        XCTAssertEqual(m.acquires, m.releases, "every acquire is released, also on validation failure")
    }

    func testComponentDevAndResourceSlotsRoundTrip() throws {
        let session = try makeSession()
        m.state = [1, 2, 3]
        XCTAssertEqual(try session.snapshotState(), Data([1, 2, 3]))
        try session.restoreState(Data([9, 8]))
        XCTAssertEqual(m.restored, [9, 8])
        XCTAssertTrue(try session.applyHotPatch(Data([0x4E, 0x56, 0x50, 0x31])))
        XCTAssertEqual(m.patches, [4])
        m.resources[5] = [0, 3, 0, 0, 0, 0x61, 0x2E, 0x62]
        let res = try session.withScene { try session.resource(5, in: $0) }
        XCTAssertEqual(res.bytes, [0, 3, 0, 0, 0, 0x61, 0x2E, 0x62])
        XCTAssertEqual(res.kind, Key64(0x1234))
        XCTAssertEqual(NveImageSource(resourceBytes: res.bytes), .url("a.b"))
        XCTAssertThrowsError(try session.withScene { try session.resource(6, in: $0) }) {
            XCTAssertTrue(($0 as? NveRuntimeError)?.isMissingCapability == true, "stale id: NOT_FOUND")
        }
        XCTAssertEqual(m.acquires, m.releases)
        try session.pairConnect(uri: "nve://pair?host=127.0.0.1&ws=8092", token: "t")
        XCTAssertEqual(m.pairedUri, "nve://pair?host=127.0.0.1&ws=8092")
        XCTAssertTrue(session.isPaired)
        try session.pairDisconnect()
        XCTAssertFalse(session.isPaired)
    }

    func testWakeAndTextMeasurerTrampolinesReachSwift() throws {
        let session = try makeSession()
        let woke = expectation(description: "wake")
        try session.setWakeHandler { woke.fulfill() }
        let (fn, ctx) = m.wake
        fn?(ctx)
        wait(for: [woke], timeout: 1)
        try session.setWakeHandler(nil)
        XCTAssertNil(m.wake.0)

        let measurer = RecordingMeasurer()
        try session.setTextMeasurer(measurer)
        let size = try XCTUnwrap(m.measureText("héllo 🚀", size: 17, weight: 600, maxWidth: .infinity))
        XCTAssertEqual([size.width, size.height], [12.5, 30])
        XCTAssertEqual(measurer.calls.first?.0, "héllo 🚀")
        XCTAssertEqual(measurer.calls.first?.1, 17)
        XCTAssertEqual(measurer.calls.first?.2, 600)
        XCTAssertEqual(measurer.calls.first?.3, .infinity)
        // A refused swap keeps the installed box alive (the runtime still calls it) and frees the new one.
        let rejected = RecordingMeasurer()
        m.failNextWith = Int32(NVE_RUNTIME_INVALID_ARG.rawValue)
        XCTAssertThrowsError(try session.setTextMeasurer(rejected))
        XCTAssertNotNil(try XCTUnwrap(m.measureText("again", size: 12, weight: 400, maxWidth: 50)))
        XCTAssertEqual(measurer.calls.count, 2)
        XCTAssertTrue(rejected.calls.isEmpty)
        m.failNextWith = Int32(NVE_RUNTIME_INVALID_ARG.rawValue)
        XCTAssertThrowsError(try session.setWakeHandler { XCTFail("never installed") })
        XCTAssertNil(m.wake.0)
        try session.setTextMeasurer(nil)
        XCTAssertNil(m.measurer.0)
        let real = NveCoreTextMeasurer().measure(text: "Hello world", fontSize: 16, fontWeight: 400, maxWidth: 30)
        let wide = NveCoreTextMeasurer().measure(text: "Hello world", fontSize: 16, fontWeight: 400, maxWidth: .infinity)
        XCTAssertGreaterThan(real.height, wide.height, "constrained width wraps onto more lines")
        XCTAssertEqual(NveCoreTextMeasurer().measure(text: "", fontSize: 16, fontWeight: 400, maxWidth: 10), .zero)
    }

    // MARK: Scene tables

    func testTablesResolveStylesInheritanceOrderAndContent() throws {
        let ids = buildScene()
        let session = try makeSession()
        try session.withScene { scene in
            let t = NveRuntimeTables(scene: scene)
            XCTAssertEqual(t.background(ids.card.index), NveColor(Rgba(packed: SceneBuilder.rgba(255, 0, 0))))
            XCTAssertNil(t.background(ids.title.index), "no `nve.visual` attached: nothing painted")
            XCTAssertEqual(t.opacity(ids.title.index), 1, "unattached dense row reads as defaults, not zeros")
            XCTAssertEqual(t.borderRadius(ids.card.index), 8)
            XCTAssertEqual(t.opacity(ids.root.index), 1, "dense default opacity is 1")
            XCTAssertEqual(t.zIndex(ids.card.index), 1)
            XCTAssertEqual(t.text(ids.title.index), "Hello")
            XCTAssertNil(t.text(ids.card.index))
            let style = t.textStyle(ids.title.index)
            XCTAssertEqual(style.fontSize, 24, "inherited from root; unset local field ignored")
            XCTAssertEqual(style.color, NveColor(Rgba(packed: SceneBuilder.rgba(0, 0, 255))))
            XCTAssertEqual(style.fontWeight, 700)
            XCTAssertEqual(style.align, .center)
            XCTAssertEqual(t.textStyle(ids.card.index).fontWeight, 400, "default weight")
            XCTAssertTrue(t.isScroll(ids.scroll.index) && t.clipsChildren(ids.scroll.index))
            XCTAssertFalse(t.clipsChildren(ids.card.index))
            XCTAssertTrue(t.isInteractive(ids.card.index))
            let input = try XCTUnwrap(t.textInput(ids.input.index))
            XCTAssertEqual(input.value, "hunter2")
            XCTAssertTrue(input.secure)
            let image = try XCTUnwrap(t.imageSource(ids.image.index))
            XCTAssertEqual(image.resource, 9)
            XCTAssertEqual(image.fit, .contain)
            XCTAssertNil(image.tint)
            // Paint order: z_index 1 card moves after the z 0 siblings, which keep source order.
            XCTAssertEqual(t.paintOrderedChildren(ids.root.index), [ids.title, ids.scroll, ids.input, ids.image, ids.card].map(\.index))
            // Stale generations never resolve rows.
            XCTAssertNil(t.row(t.text, 99))
        }
    }

    // MARK: Store + controller

    func testStoreSyncMirrorsRuntimeScrollAndBuildsAccessibility() throws {
        let ids = buildScene()
        let session = try makeSession()
        let store = NveSceneStore(session: session)
        m.scroll[ids.scroll.raw] = (0, 40)
        m.removed = [NodeId(index: 77, generation: 1).raw]
        store.sync()
        XCTAssertEqual(store.frameId, 1)
        XCTAssertEqual(store.scrollOffset(for: ids.scroll), CGPoint(x: 0, y: 40))
        XCTAssertEqual(store.lastRemovedCount, 1)
        XCTAssertEqual(store.scroll(ids.scroll, by: CGVector(dx: 0, dy: 10)), CGPoint(x: 0, y: 50))
        XCTAssertEqual(store.scrollTarget(at: CGPoint(x: 20, y: 150)), ids.scroll)
        XCTAssertNil(store.scrollTarget(at: CGPoint(x: 20, y: 20)))
        m.hitResult = ids.card.raw
        XCTAssertEqual(store.hitTest(at: CGPoint(x: 20, y: 20)), ids.card)
        m.hitResult = 0
        XCTAssertNil(store.hitTest(at: CGPoint(x: 1, y: 1)))

        let elements = store.accessibilityElements()
        let byId = Dictionary(uniqueKeysWithValues: elements.map { ($0.id, $0) })
        XCTAssertEqual(byId[ids.card]?.role, .button)
        XCTAssertEqual(byId[ids.title]?.role, .header, "24pt inherited text is announced as a heading")
        XCTAssertEqual(byId[ids.title]?.label, "Hello")
        XCTAssertEqual(byId[ids.input]?.value, "•••••••")
        XCTAssertEqual(byId[ids.input]?.label, "Password")
        XCTAssertTrue(byId[ids.input]?.isSecure == true)
        XCTAssertEqual(byId[ids.image]?.label, "Brand logo")
        XCTAssertEqual(byId[ids.image]?.role, .image)

        m.resources[9] = [1, 3, 0, 0, 0, 0xFF, 0xD8, 0xFF]
        XCTAssertEqual(store.imageSource(9), .encoded([0xFF, 0xD8, 0xFF]))
        XCTAssertNil(store.image(resource: 9), "invalid JPEG bytes decode to nil, never crash")
    }

    func testControllerDrivesPropsEventsLayoutReloadAndPairing() throws {
        _ = buildScene()
        let controller = try NveSurfaceController(
            surfaceId: 7, getInterface: noApp, runtimeApi: MockRuntime.api, textMeasurer: RecordingMeasurer())
        XCTAssertEqual(controller.resolutionSource, .explicit)
        XCTAssertEqual(m.attaches, 1)
        XCTAssertNotNil(m.measurer.0, "the native text measurer is installed")
        XCTAssertGreaterThan(m.ticks, 0)

        XCTAssertTrue(controller.setProps(["title": "A", "count": 1]))
        XCTAssertFalse(controller.setProps(["count": 1, "title": "A"]), "canonical props dedupe")
        XCTAssertEqual(m.propCounts, [2])

        var received: [NveEvent] = []
        controller.onEvent = { received.append($0) }
        m.emitted = [NveDirectEvent()]
        m.emitted[0].surface_id = 7
        m.emitted[0].node_id = 3
        XCTAssertTrue(controller.dispatchEvent(NveEvent(surfaceId: 7, nodeId: 2, timestampUs: 1, kind: .press, payload: .point(x: 1, y: 2))))
        XCTAssertEqual(m.dispatched.last?.first?.nodeId, 2)
        XCTAssertEqual(received.map(\.nodeId), [3], "outbound events are drained after each pump")

        // Unconstrained axes take the laid-out root rect (fallback when it is empty).
        let rootRect = m.scene.rects[NodeId.root.index]
        let natural = rootRect.width > 0 && rootRect.height > 0
            ? CGSize(width: CGFloat(rootRect.width), height: CGFloat(rootRect.height)) : controller.fallbackIntrinsicSize
        XCTAssertEqual(controller.measure(proposal: CGSize(width: 320, height: CGFloat.infinity)), CGSize(width: 320, height: natural.height))
        XCTAssertEqual(controller.measure(proposal: CGSize(width: CGFloat.infinity, height: 50)), CGSize(width: natural.width, height: 50))
        XCTAssertEqual(controller.measure(proposal: CGSize(width: 10, height: 20)), CGSize(width: 10, height: 20))
        controller.resize(CGSize(width: 320, height: 480), scale: 2)
        XCTAssertEqual(m.viewports.last?.0, 320)
        XCTAssertEqual(m.viewports.last?.2, 2)

        try controller.pair(uri: "nve://pair?host=h&ws=1")
        XCTAssertTrue(controller.isPaired)
        try controller.unpair()
        XCTAssertFalse(controller.isPaired)
        try controller.detach()
        XCTAssertEqual(m.detaches, 2, "attach detaches first, then the explicit detach")
        XCTAssertFalse(controller.isMounted)
    }

    @MainActor
    func testRendererPaintsBackgroundsFromTheRuntimeScene() throws {
        _ = buildScene()
        let controller = try NveSurfaceController(getInterface: noApp, runtimeApi: MockRuntime.api)
        controller.store.sync()
        let view = Canvas { context, _ in NveSceneRenderer.draw(store: controller.store, into: &context) }
            .frame(width: 200, height: 300)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        let image = try XCTUnwrap(renderer.cgImage)
        func pixel(_ x: Int, _ y: Int) -> (UInt8, UInt8, UInt8) {
            var px = [UInt8](repeating: 0, count: 4)
            let ctx = CGContext(data: &px, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
            ctx.draw(image, in: CGRect(x: -x, y: y - image.height + 1, width: image.width, height: image.height))
            return (px[0], px[1], px[2])
        }
        let card = pixel(35, 35)
        XCTAssertGreaterThan(card.0, 200)
        XCTAssertLessThan(card.1, 60)
        let root = pixel(150, 40)
        XCTAssertGreaterThan(root.1, 200, "white root background")
    }
}
