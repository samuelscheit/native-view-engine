import CNveAbi
import CoreText
import NveHost
import SwiftUI
import XCTest
@testable import NveSwiftUI

private func noApp(_ id: UInt32) -> UnsafeRawPointer? { nil }

private final class RecordingMeasurer: NveTextMeasuring {
    var calls: [(String, CGFloat, UInt16, CGFloat, String)] = []
    func measure(text: String, style: NveRuntimeTextStyle, maxWidth: CGFloat) -> CGSize {
        let family = style.font_family.ptr.map {
            String(decoding: UnsafeBufferPointer(start: $0, count: style.font_family.len), as: UTF8.self)
        } ?? ""
        calls.append((text, CGFloat(style.font_size), style.font_weight, maxWidth, family))
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
            SceneBuilder.setExplicit(&$0, NveTables.Typography.color, NveTables.Typography.fontSize, NveTables.Typography.maxLines)
            SceneBuilder.put(&$0, NveTables.Typography.color, SceneBuilder.rgba(0, 0, 255))
            SceneBuilder.put(&$0, NveTables.Typography.fontSize, Float(24))
            SceneBuilder.put(&$0, NveTables.Typography.maxLines, UInt32(2))
        }
        s.typography(title) {
            SceneBuilder.setExplicit(&$0, NveTables.Typography.fontWeight, NveTables.Typography.textAlign)
            SceneBuilder.put(&$0, NveTables.Typography.fontWeight, UInt32(700))
            SceneBuilder.put(&$0, NveTables.Typography.textAlign, NveTextAlign.center.rawValue)
            // Not in `__explicit`: must NOT override the inherited 24pt.
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
        s.sparse(NveTables.Interaction.key, words: NveTables.Interaction.words, defaults: NveTables.Interaction.defaults, card) {
            SceneBuilder.setEvents(&$0, eventsWord: NveTables.Interaction.eventsWord, NveInteractionEventSet.press)
        }
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
        let flatUrl: [UInt8] = [0, 0, 0, 0, 3, 0, 0, 0, 0x61, 0x2E, 0x62, 0]
        m.resources[5] = flatUrl
        m.resourceKinds[5] = 0x1234
        let res = try session.withScene { try session.resource(5, in: $0) }
        XCTAssertEqual(res.bytes, flatUrl)
        XCTAssertEqual(res.kind, Key64(0x1234))
        XCTAssertEqual(NveImageSource(resourceBytes: res.bytes), .url("a.b"))
        XCTAssertNil(NveImageSource(resourceBytes: [0, 0, 0, 0, 3, 0, 0, 0, 0x61, 0x2E, 0x62, 1]), "non-zero pad byte")
        XCTAssertNil(NveImageSource(resourceBytes: [0, 3, 0, 0, 0, 0x61, 0x2E, 0x62]), "legacy 5-byte layout rejected")
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
        let size = try XCTUnwrap(m.measureText("héllo 🚀", size: 17, weight: 600, maxWidth: .infinity, family: "Menlo"))
        XCTAssertEqual([size.width, size.height], [12.5, 30])
        XCTAssertEqual(measurer.calls.first?.0, "héllo 🚀")
        XCTAssertEqual(measurer.calls.first?.1, 17)
        XCTAssertEqual(measurer.calls.first?.2, 600)
        XCTAssertEqual(measurer.calls.first?.3, .infinity)
        XCTAssertEqual(measurer.calls.first?.4, "Menlo", "the resolved style reaches the measurer")
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
        let core = NveCoreTextMeasurer()
        let real = core.measure(text: "Hello world", style: .sized(16), maxWidth: 30)
        let wide = core.measure(text: "Hello world", style: .sized(16), maxWidth: .infinity)
        XCTAssertGreaterThan(real.height, wide.height, "constrained width wraps onto more lines")
        XCTAssertEqual(core.measure(text: "", style: .sized(16), maxWidth: 10), .zero)
        // Italic is painted, so it is measured with the slanted face.
        let traits = { (italic: Bool) in CTFontGetSymbolicTraits(try XCTUnwrap(core.font(size: 16, weight: 400, italic: italic))) }
        XCTAssertTrue(try traits(true).contains(.traitItalic))
        XCTAssertFalse(try traits(false).contains(.traitItalic))
        XCTAssertGreaterThan(core.measure(text: "Hello world", style: .sized(16, italic: true), maxWidth: .infinity).width, 0)
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
            XCTAssertEqual(style.maxLines, 2, "optional zeroNiche maxLines inherited from root")
            XCTAssertEqual(style.fontWeight, 700)
            XCTAssertEqual(style.align, .center)
            XCTAssertEqual(t.textStyle(ids.card.index).fontWeight, 400, "default weight")
            XCTAssertTrue(t.isScroll(ids.scroll.index) && t.clipsChildren(ids.scroll.index))
            XCTAssertFalse(t.clipsChildren(ids.card.index))
            XCTAssertTrue(t.isInteractive(ids.card.index))
            XCTAssertTrue(t.hasInteractionEvent(ids.card.index, bit: NveInteractionEventSet.press))
            XCTAssertFalse(t.hasInteractionEvent(ids.card.index, bit: NveInteractionEventSet.longPress))
            XCTAssertEqual(t.interactionEvents(ids.card.index), NveInteractionEventSet.press)
            XCTAssertEqual(t.interactionEvents(ids.title.index), 0)
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
        XCTAssertEqual(store.scrollOffset(for: ids.scroll), CGPoint(x: 0, y: 40), "drawn offsets wait for the frame")
        m.frameId = 2
        store.sync()
        XCTAssertEqual(store.scrollOffset(for: ids.scroll), CGPoint(x: 0, y: 50), "the published frame moves them")
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

        m.resources[9] = [1, 0, 0, 0, 3, 0, 0, 0, 0xFF, 0xD8, 0xFF, 0]
        XCTAssertEqual(store.imageSource(9), .encoded([0xFF, 0xD8, 0xFF]))
        XCTAssertNil(store.image(resource: 9), "invalid JPEG bytes decode to nil, never crash")

        // A DrawList resource whose bytes collide with an EncodedBytes ImageSrc wire header
        // (`[1, 1, 0]`) must be rejected by kind check before decoding.
        m.resources[10] = [1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        m.resourceKinds[10] = Key64.of("nve.res.draw_list").value
        XCTAssertNil(store.imageSource(10), "wrong FlatResource kind must not decode as ImageSrc")
        XCTAssertNil(store.image(resource: 10))
    }

    func testImageSourcesFollowDirtyResourcesEpochsAndFrameGaps() throws {
        _ = buildScene()
        let store = NveSceneStore(session: try makeSession())
        let url = { (s: String) -> [UInt8] in
            let p = Array(s.utf8)
            let pad = (4 - (p.count & 3)) & 3
            return [0, 0, 0, 0, UInt8(p.count), 0, 0, 0] + p + Array(repeating: 0, count: pad)
        }
        m.resources = [4: url("a"), 5: url("b")]
        store.sync()
        XCTAssertEqual(store.imageSource(4), .url("a"))
        XCTAssertEqual(store.imageSource(5), .url("b"))
        // Synced again without a new frame: the cache stays (it used to read as a frame gap).
        m.resources = [4: url("stale"), 5: url("stale")]
        store.sync()
        XCTAssertEqual(store.imageSource(4), .url("a"))
        // Rewritten in place: only the listed id is re-read.
        m.resources = [4: url("a2"), 5: url("b2")]
        m.frameId = 2
        m.dirtyResources = [4]
        store.sync()
        XCTAssertEqual(store.imageSource(4), .url("a2"))
        XCTAssertEqual(store.imageSource(5), .url("b"), "ids absent from dirty_resources keep their bytes")
        // A new scene epoch reuses ids for other bytes: everything is re-read.
        m.resources = [4: url("a3"), 5: url("b3")]
        m.frameId = 3
        m.dirtyResources = []
        m.sceneEpoch = 1
        store.sync()
        XCTAssertEqual(store.imageSource(5), .url("b3"))
        // A skipped frame may have dirtied anything.
        m.resources = [4: url("a4"), 5: url("b4")]
        m.frameId = 5
        store.sync()
        XCTAssertEqual(store.imageSource(4), .url("a4"))
        XCTAssertEqual(store.imageSource(5), .url("b4"))
        // A 0x0 / undecodable encoded image is nil.
        XCTAssertNil(NveImageCache().image(encoded: [0x89, 0x50, 0x4E, 0x47], key: "png-header-only"))
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

        var received: [NveSwiftUI.NveEvent] = []
        controller.onEvent = { received.append($0) }
        m.emitted.pushPress(surface: 7, node: 3, timestamp: 10, x: 5, y: 6)
        XCTAssertTrue(controller.dispatchEvent(NveSwiftUI.NveEvent(surfaceId: 7, nodeId: 2, timestampUs: 1, kind: .press, payload: .point(x: 1, y: 2))))
        XCTAssertEqual(m.dispatched.last?.first?.nodeId, 2)
        XCTAssertEqual(received.map(\.nodeId), [3], "outbound events are drained after each pump")
        XCTAssertEqual(received.first?.kind, .press)
        controller.onEvent = nil
        m.emitted.pushFocus(surface: 7, node: 3, timestamp: 11)
        controller.pump()
        XCTAssertTrue(m.emitted.isEmpty, "events are drained even without a listener")

        // A rejected app keeps the running one (the runtime swaps atomically).
        m.failNextWith = -1
        XCTAssertThrowsError(try controller.attach(NveResolvedInterface(getInterface: noApp, source: .staticTable)))
        XCTAssertEqual(controller.resolutionSource, .explicit)
        XCTAssertEqual(m.detaches, 0, "attach swaps, it never detaches first")

        // `.dylib`s go through the runtime's `load_app_module`; a reload carries the app state.
        try controller.load(dylibPath: "/apps/libnve_app.dylib")
        XCTAssertEqual(controller.resolutionSource, .dynamicLibrary)
        m.failNextWith = -1
        XCTAssertThrowsError(try controller.reload(dylibPath: "/apps/broken.dylib"))
        XCTAssertEqual(controller.resolutionSource, .dynamicLibrary, "a failed reload keeps the running module")
        try controller.reload(dylibPath: "/apps/libnve_app.dylib")
        XCTAssertEqual(m.loads.map(\.0), ["/apps/libnve_app.dylib", "/apps/broken.dylib", "/apps/libnve_app.dylib"])
        XCTAssertEqual(m.loads.map(\.1), [0, 1, 1], "only reloads carry state")
        XCTAssertEqual(m.detaches, 0)

        // Model A `measure_surface`: constrained axes pass max constraints, unspecified passes IDEAL.
        let rootRect = m.scene.rects[NodeId.root.index]
        XCTAssertEqual(controller.measure(proposal: CGSize(width: 320, height: CGFloat.infinity)),
                       CGSize(width: CGFloat(rootRect.width), height: CGFloat(rootRect.height)))
        XCTAssertEqual(m.measureCalls.last?.flags, UInt32(NVE_MEASURE_FLAG_DRY_RUN))
        XCTAssertEqual(controller.measure(proposal: CGSize(width: CGFloat.infinity, height: 50)),
                       CGSize(width: CGFloat(rootRect.width), height: 50))
        XCTAssertEqual(controller.measure(proposal: CGSize(width: 10, height: 20)), CGSize(width: 10, height: 20))
        XCTAssertEqual(controller.measure(proposal: CGSize(width: CGFloat.infinity, height: CGFloat.infinity)),
                       CGSize(width: CGFloat(rootRect.width), height: CGFloat(rootRect.height)))
        XCTAssertEqual(m.measureCalls.last?.flags, UInt32(NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL))
        controller.resize(CGSize(width: 320, height: 480), scale: 2)
        XCTAssertEqual(m.viewports.last?.0, 320)
        XCTAssertEqual(m.viewports.last?.2, 2)

        m.failNextWith = -1
        XCTAssertThrowsError(try controller.pair(uri: "nve://pair?host=h&ws=1"))
        XCTAssertTrue(controller.isMounted, "a failed pair keeps the local app")
        try controller.pair(uri: "nve://pair?host=h&ws=1")
        XCTAssertTrue(controller.isPaired)
        XCTAssertFalse(controller.isMounted, "a successful pair replaced the local app")
        XCTAssertTrue(controller.setProps(["title": "A", "count": 1]), "props are resent to the paired app")
        try controller.unpair()
        XCTAssertFalse(controller.isPaired)
        try controller.detach()
        XCTAssertEqual(m.detaches, 1)
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

    func testOptionalImageTintAndAccessibilityRoleDecodeViaOptEncoding() throws {
        var s = SceneBuilder()
        let root = s.node(rect: CGRect(x: 0, y: 0, width: 100, height: 100))
        let tinted = s.node(parent: root, rect: CGRect(x: 0, y: 0, width: 50, height: 50))
        let unlabeled = s.node(parent: root, rect: CGRect(x: 50, y: 0, width: 50, height: 50))
        s.setChildren(root, [tinted, unlabeled])
        s.sparse(NveTables.Image.key, words: NveTables.Image.words, defaults: NveTables.Image.defaults, tinted) {
            SceneBuilder.put(&$0, NveTables.Image.src, UInt32(3))
            SceneBuilder.put(&$0, NveTables.Image.tint, SceneBuilder.rgba(10, 20, 30))
        }
        let hint = s.str("Details")
        s.sparse(NveTables.Accessibility.key, words: NveTables.Accessibility.words, defaults: NveTables.Accessibility.defaults, unlabeled) {
            SceneBuilder.put(&$0, NveTables.Accessibility.hint, hint)
        }
        m.scene = s
        let session = try makeSession()
        try session.withScene { scene in
            let t = NveRuntimeTables(scene: scene)
            let img = try XCTUnwrap(t.imageSource(tinted.index))
            XCTAssertEqual(img.tint, NveColor(Rgba(packed: SceneBuilder.rgba(10, 20, 30))))
            let a11y = try XCTUnwrap(t.accessibility(unlabeled.index))
            XCTAssertNil(a11y.role, "absent plusOne Accessibility.role decodes to nil")
            XCTAssertEqual(a11y.hint, "Details")
        }
    }

    func testV6MeasureFrame0AndHostServicesWrappers() throws {
        let session = try makeSession()

        // 1. `measureSurface` forwards constraints & flags and decodes baselines.
        let c = NveSizeConstraints(min_w: 20, min_h: 10, max_w: 320, max_h: 240)
        let measured = try session.measureSurface(constraints: c, flags: NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL)
        XCTAssertEqual([measured.width, measured.height, measured.first_baseline, measured.last_baseline], [120, 40, 28, 36])
        XCTAssertEqual(m.measuredConstraints.count, 1)
        XCTAssertEqual(m.measuredConstraints[0].1, NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL)

        // 2. `setIntrinsicSizeInvalidatedHandler`: fires, preserves old box on failed swap, clears on nil.
        let fired = expectation(description: "intrinsic invalidated")
        try session.setIntrinsicSizeInvalidatedHandler { fired.fulfill() }
        m.intrinsicSize.0?(m.intrinsicSize.1)
        wait(for: [fired], timeout: 1)
        m.failNextWith = Int32(NVE_RUNTIME_INVALID_ARG.rawValue)
        XCTAssertThrowsError(try session.setIntrinsicSizeInvalidatedHandler { XCTFail("must not replace") })
        XCTAssertNotNil(m.intrinsicSize.0, "failed swap keeps prior handler installed")
        try session.setIntrinsicSizeInvalidatedHandler(nil)
        XCTAssertNil(m.intrinsicSize.0)

        // 3. Frame-0 `saveFrame0File` / `bootFrame0File` / `bootFrame0Bytes`: NOT_FOUND -> false, OK -> true, other -> throws.
        XCTAssertTrue(try session.saveFrame0File(path: "/tmp/f0.nve2", appBuildId: 11, envHash: 22))
        m.failNextWith = Int32(NVE_RUNTIME_NOT_FOUND.rawValue)
        XCTAssertFalse(try session.saveFrame0File(path: "/tmp/empty.nve2", appBuildId: 11, envHash: 22))
        m.failNextWith = Int32(NVE_RUNTIME_IO.rawValue)
        XCTAssertThrowsError(try session.saveFrame0File(path: "/tmp/io.nve2"))

        XCTAssertFalse(try session.bootFrame0File(path: "/tmp/miss.nve2", appBuildId: 11, envHash: 22))
        XCTAssertFalse(try session.bootFrame0Bytes(Data([1, 2, 3]), appBuildId: 11, envHash: 22))
        m.frame0DefaultStatus = Int32(NVE_RUNTIME_OK.rawValue)
        XCTAssertTrue(try session.bootFrame0File(path: "/tmp/hit.nve2", appBuildId: 11, envHash: 22))
        XCTAssertTrue(try session.bootFrame0Bytes(Data([4, 5, 6, 7]), appBuildId: 11, envHash: 22))
        m.failNextWith = Int32(NVE_RUNTIME_INVALID_ARG.rawValue)
        XCTAssertThrowsError(try session.bootFrame0File(path: "/tmp/attached.nve2"))
        m.failNextWith = Int32(NVE_RUNTIME_INVALID_ARG.rawValue)
        XCTAssertThrowsError(try session.bootFrame0Bytes(Data([4, 5])))
        XCTAssertEqual(m.frame0Saves.count, 3)
        XCTAssertEqual(m.frame0FileBoots.count, 3)
        XCTAssertEqual(m.frame0BytesBoots.count, 3)

        // 4. Host services callbacks & cross-thread ingress wrappers.
        try session.setHostCallbacks(NveHostCallbacks())
        try session.setHostCallbacks(nil)
        try session.completeHostRequest(requestId: 77, status: 0, payload: Data([9, 8]))
        try session.pushHostEvent(serviceId: 0x2001, eventId: 4, payload: Data([1, 2]))
        try session.publishHostChannel(channelId: 5, flags: 1, payload: Data([3]))
        XCTAssertEqual(m.hostCallbackSets, [true, false])
        XCTAssertEqual(m.completedRequests.first?.0, 77)
        XCTAssertEqual(m.completedRequests.first?.2, [9, 8])
        XCTAssertEqual(m.pushedEvents.first?.0, 0x2001)
        XCTAssertEqual(m.pushedEvents.first?.1, 4)
        XCTAssertEqual(m.pushedEvents.first?.2, [1, 2])
        XCTAssertEqual(m.publishedChannels.first?.0, 5)
        XCTAssertEqual(m.publishedChannels.first?.1, 1)
        XCTAssertEqual(m.publishedChannels.first?.2, [3])
    }
}
