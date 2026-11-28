import CNveAbi
import CoreGraphics
import CoreText
import Foundation
import NveHost
import SwiftUI
import XCTest
@testable import NveSwiftUI

#if os(macOS)
import AppKit
#endif

private func noApp(_ id: UInt32) -> UnsafeRawPointer? { nil }

private func packRgba(_ r: UInt8, _ g: UInt8, _ b: UInt8, _ a: UInt8 = 255) -> UInt32 {
    UInt32(r) | (UInt32(g) << 8) | (UInt32(b) << 16) | (UInt32(a) << 24)
}

private func encodeWords(_ words: [UInt32]) -> [UInt8] {
    var out: [UInt8] = []
    out.reserveCapacity(words.count * 4)
    for w in words {
        withUnsafeBytes(of: w.littleEndian) { out.append(contentsOf: $0) }
    }
    return out
}

private func rgbaPixel(_ image: CGImage, x: Int, y: Int) -> (UInt8, UInt8, UInt8, UInt8) {
    var px = [UInt8](repeating: 0, count: 4)
    let ctx = CGContext(
        data: &px, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue
    )!
    ctx.draw(
        image,
        in: CGRect(x: -CGFloat(x), y: -CGFloat(image.height - 1 - y), width: CGFloat(image.width), height: CGFloat(image.height))
    )
    return (px[0], px[1], px[2], px[3])
}

private func countDark(_ image: CGImage, in rect: CGRect, threshold: UInt8 = 150) -> Int {
    let x0 = max(0, Int(rect.minX)), x1 = min(image.width, Int(rect.maxX))
    let y0 = max(0, Int(rect.minY)), y1 = min(image.height, Int(rect.maxY))
    var count = 0
    for y in y0..<y1 {
        for x in x0..<x1 {
            let p = rgbaPixel(image, x: x, y: y)
            if p.3 > 20 && p.0 < threshold && p.1 < threshold && p.2 < threshold { count += 1 }
        }
    }
    return count
}

/// Deep Apple parity tests closing every Lens 4 gap from the Track 1B.2 Slice 3 review:
/// off-thread deinit callback safety, atomic detach, single-hop coalescing, Model A baselines &
/// edge proposals, live `NveSurfaceRepresentable` dedup/onDisappear, AppKit `NSTextView`/
/// `NSTextField` IME + hit-test clipping + teardown + content-box inset, typography fidelity, and
/// all 8 Canvas DrawList opcodes + transparent clear + bounded cache eviction.
final class NveAppleReviewParityTests: XCTestCase {
    private var m: MockRuntime { MockRuntime.shared }

    override func setUp() {
        super.setUp()
        MockRuntime.shared.reset()
    }

    // MARK: 1. Off-thread Deinit Safety, Exact Coalescing & Atomic Detach (P0 #1, P2 #12)

    func testOffThreadDeinitDoesNotUseAfterFreeWhenCallbacksFireLater() throws {
        var savedWake: (NveRuntimeWakeFn?, UnsafeMutableRawPointer?) = (nil, nil)
        var savedMeasurer: (NveRuntimeMeasureTextFn?, UnsafeMutableRawPointer?) = (nil, nil)
        var savedIntrinsic: (NveRuntimeIntrinsicSizeInvalidatedFn?, UnsafeMutableRawPointer?) = (nil, nil)

        weak var weakController: NveSurfaceController?
        do {
            let controller = try NveSurfaceController(surfaceId: 41, getInterface: noApp, runtimeApi: MockRuntime.api)
            weakController = controller
            savedWake = m.wake
            savedMeasurer = m.measurer
            savedIntrinsic = m.intrinsicInvalidated
            XCTAssertNotNil(savedWake.0)
            XCTAssertNotNil(savedMeasurer.0)
            XCTAssertNotNil(savedIntrinsic.0)

            // Simulate runtime rejecting callback unregister from another thread (`NVE_RUNTIME_BUSY`).
            m.failCallbackClearWith = Int32(NVE_RUNTIME_BUSY.rawValue)
        }

        // Controller & session are gone, yet boxes were intentionally retained (not freed) on error.
        XCTAssertNil(weakController)
        savedIntrinsic.0?(savedIntrinsic.1)
        savedWake.0?(savedWake.1)
        var outW: Float = -1, outH: Float = -1
        var text = "Safe"
        var style = NveRuntimeTextStyle.sized(14)
        text.withUTF8 { buf in
            savedMeasurer.0?(savedMeasurer.1, NveStrSlice(ptr: buf.baseAddress, len: buf.count), &style, 100, &outW, &outH)
        }
        XCTAssertGreaterThan(outW, 0)
        XCTAssertGreaterThan(outH, 0)

        // Drain main queue to verify delayed wake/intrinsic dispatches hit weak nil cleanly.
        let exp = expectation(description: "drained")
        DispatchQueue.main.async { exp.fulfill() }
        wait(for: [exp], timeout: 1)
    }

    func testIntrinsicSizeInvalidationCoalescesBurstIntoExactSingleHopAndAtomicDetach() throws {
        let controller = try NveSurfaceController(surfaceId: 42, getInterface: noApp, runtimeApi: MockRuntime.api)
        XCTAssertEqual(controller.intrinsicSizeRevision, 0)

        let (fn, ctx) = m.intrinsicInvalidated
        for _ in 0..<5 { fn?(ctx) }

        let exp = expectation(description: "single-hop coalesced revision")
        DispatchQueue.main.async { exp.fulfill() }
        wait(for: [exp], timeout: 1)
        XCTAssertEqual(controller.intrinsicSizeRevision, 1, "burst of 5 synchronous invalidations coalesces into exactly 1 revision bump")

        // Failed detach (BUSY) MUST leave slot 29 registered atomically.
        m.failNextWith = Int32(NVE_RUNTIME_BUSY.rawValue)
        XCTAssertThrowsError(try controller.detach())
        XCTAssertNotNil(m.intrinsicInvalidated.0, "slot 29 stays registered when attachApp(nil) fails")

        // Successful detach clears slot 29.
        try controller.detach()
        XCTAssertNil(m.intrinsicInvalidated.0)
    }

    // MARK: 2. Model A Baselines, Edge Proposals, 0x0 Collapsed Surface & Live Representable (P2 #12)

    @MainActor
    func testModelABaselinesEdgeProposalsCollapsedSurfaceAndFrameIdCacheInvalidation() throws {
        // Before first published frame (frameId == 0), a 0x0 measure falls back to fallbackIntrinsicSize.
        m.tickChanged = false
        m.frameId = 0
        m.measureHandler = { _, _ in NveSurfaceMeasure(width: 0, height: 0, first_baseline: .nan, last_baseline: .nan) }
        let controller = try NveSurfaceController(surfaceId: 43, getInterface: noApp, runtimeApi: MockRuntime.api)
        XCTAssertEqual(controller.store.frameId, 0)
        XCTAssertEqual(controller.measureSurface(proposal: .unspecified).size, controller.fallbackIntrinsicSize)

        // After first published frame (frameId > 0), an intentionally collapsed 0x0 surface returns 0x0.
        m.frameId = 1
        m.tickChanged = true
        controller.pump()
        XCTAssertEqual(controller.store.frameId, 1)
        XCTAssertEqual(controller.measureSurface(proposal: .unspecified).size, .zero)

        // Edge proposals: .zero, .infinity, negative, and NaN never pass invalid constraints.
        m.measureCalls.removeAll()
        m.measureHandler = { c, _ in
            NveSurfaceMeasure(width: min(100, c.max_w), height: min(40, c.max_h), first_baseline: 12, last_baseline: 32)
        }
        let zeroM = controller.measureSurface(proposal: .zero)
        XCTAssertEqual(zeroM.size, .zero)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_w, 0)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_h, 0)

        let infM = controller.measureSurface(proposal: .infinity)
        XCTAssertEqual(infM.size, CGSize(width: 100, height: 40))
        XCTAssertEqual(m.measureCalls.last?.constraints.max_w, .infinity)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_h, .infinity)

        let negM = controller.measureSurface(proposal: ProposedViewSize(width: -20, height: -10))
        XCTAssertEqual(negM.size, .zero)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_w, 0)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_h, 0)

        let nanM = controller.measureSurface(proposal: ProposedViewSize(width: .nan, height: .nan))
        XCTAssertEqual(nanM.size, CGSize(width: 100, height: 40))
        XCTAssertEqual(m.measureCalls.last?.constraints.max_w, .infinity)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_h, .infinity)

        // explicitAlignment baselineCoordinate respects bounds.minY, nil on non-baseline guides, and frameId cache invalidation.
        m.measureCalls.removeAll()
        let bounds = CGRect(x: 10, y: 25, width: 100, height: 40)
        var cache = NveIntrinsicLayout.Cache()
        let layout1 = NveIntrinsicLayout(controller: controller, intrinsicRevision: 0, frameId: 1)
        XCTAssertEqual(layout1.baselineCoordinate(for: .firstTextBaseline, in: bounds, proposal: .unspecified, cache: &cache), 37)
        XCTAssertEqual(layout1.baselineCoordinate(for: .lastTextBaseline, in: bounds, proposal: .unspecified, cache: &cache), 57)
        XCTAssertNil(layout1.baselineCoordinate(for: .center, in: bounds, proposal: .unspecified, cache: &cache))
        XCTAssertEqual(m.measureCalls.count, 1, "first/last baseline share one cached FFI measure")

        let layout2 = NveIntrinsicLayout(controller: controller, intrinsicRevision: 0, frameId: 2)
        _ = layout2.measure(proposal: .unspecified, cache: &cache)
        XCTAssertEqual(m.measureCalls.count, 2, "frameId change invalidates layout cache")

        // Real SwiftUI HStack(alignment: .firstTextBaseline) exercises explicitAlignment inside ImageRenderer.
        var s = SceneBuilder()
        let root = s.node(rect: CGRect(x: 0, y: 0, width: 40, height: 40))
        s.visual(root) { SceneBuilder.put(&$0, NveTables.Visual.background, SceneBuilder.rgba(220, 20, 20)) }
        m.scene = s
        m.frameId = 3
        controller.pump()

        func renderTopRedY(baseline: Float) throws -> Int {
            m.measureHandler = { _, _ in
                NveSurfaceMeasure(width: 40, height: 40, first_baseline: baseline, last_baseline: baseline)
            }
            let view = HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text("Hi").font(.system(size: 16))
                NveIntrinsicLayout(controller: controller, intrinsicRevision: UInt64(baseline), frameId: 3) {
                    Color(.sRGB, red: 1, green: 0, blue: 0, opacity: 1)
                }
            }
            .frame(width: 120, height: 100, alignment: .topLeading)
            .background(Color.white)
            let r = ImageRenderer(content: view)
            r.scale = 1
            let cg = try XCTUnwrap(r.cgImage)
            for y in 0..<cg.height {
                for x in 0..<cg.width {
                    let p = rgbaPixel(cg, x: x, y: y)
                    if p.0 > 200 && p.1 < 50 && p.2 < 50 { return y }
                }
            }
            return -1
        }
        let lowBaseY = try renderTopRedY(baseline: 6)
        let highBaseY = try renderTopRedY(baseline: 36)
        XCTAssertGreaterThanOrEqual(lowBaseY, 0)
        XCTAssertGreaterThanOrEqual(highBaseY, 0)
        XCTAssertGreaterThan(lowBaseY, highBaseY, "larger firstTextBaseline shifts the NVE box higher to align baselines")
    }

    #if os(macOS)
    @MainActor
    func testLiveSurfaceRepresentableDeduplicatesPropsAndClearsOnEventOnDisappear() throws {
        final class HostModel: ObservableObject {
            @Published var props: [String: Any]
            init(_ props: [String: Any]) { self.props = props }
        }
        struct HostWrapper: View {
            @ObservedObject var model: HostModel
            let controller: NveSurfaceController
            let onEvent: (NveSwiftUI.NveEvent) -> Void
            var body: some View {
                NveSurfaceRepresentable(componentName: "Card", props: model.props, controller: controller, onEvent: onEvent)
            }
        }

        let controller = try NveSurfaceController(surfaceId: 44, getInterface: noApp, runtimeApi: MockRuntime.api)
        var received: [NveSwiftUI.NveEvent] = []
        let model = HostModel(["a": 1, "b": "hi"])
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 160, height: 80), styleMask: [.borderless], backing: .buffered, defer: false)
        let hosting = NSHostingView(rootView: HostWrapper(model: model, controller: controller, onEvent: { received.append($0) }))
        hosting.frame = NSRect(x: 0, y: 0, width: 160, height: 80)
        func tickLayout() {
            hosting.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        }
        window.contentView = hosting; tickLayout()
        XCTAssertEqual(m.propCounts.count, 1, "initial mount pushes props once")

        model.props = ["b": "hi", "a": 1]; tickLayout()
        XCTAssertEqual(m.propCounts.count, 1, "equal canonical props skip redundant set_props")

        model.props = ["a": 2, "b": "hi"]; tickLayout()
        XCTAssertEqual(m.propCounts.count, 2, "changed props push once")

        window.contentView = nil
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        XCTAssertNil(controller.onEvent, "onDisappear clears controller.onEvent to avoid stale callback leaks")
        _ = received
    }
    #endif

    // MARK: 3. Native TextInput IME, Selection Preservation, Echo Race, Clipping & Teardown (P0 #2, P1 #7)

    func testControlledTextEchoRacePendingAppFlushAndHostWriteGuard() {
        let state = NveControlledTextState()
        XCTAssertTrue(state.shouldApplyAppValue("A", currentShownText: "", hasMarkedText: false))

        // A -> B user edit, then unrelated frame re-publishes stale appValue "A" -> short-circuit, no clobber.
        XCTAssertEqual(state.handleUserEdit(newText: "B", hasMarkedText: false, markedText: nil), [.textChange("B")])
        XCTAssertFalse(state.shouldApplyAppValue("A", currentShownText: "B", hasMarkedText: false))
        XCTAssertEqual(state.handleUserEdit(newText: "A", hasMarkedText: false, markedText: nil), [.textChange("A")])
        XCTAssertFalse(state.shouldApplyAppValue("B", currentShownText: "A", hasMarkedText: false))
        XCTAssertFalse(state.shouldApplyAppValue("A", currentShownText: "A", hasMarkedText: false))

        // Deferred pendingAppValue flushes when IME composition cancels back to unchanged committed text.
        XCTAssertEqual(state.handleUserEdit(newText: "Ani", hasMarkedText: true, markedText: "ni"), [.compositionStart("ni")])
        XCTAssertFalse(state.shouldApplyAppValue("SERVER_RESET", currentShownText: "Ani", hasMarkedText: true))
        var dispatched: [[NveTextInputAction]] = []; var written: [String] = []
        state.processNativeEdit(
            currentText: "A", hasMarkedText: false, markedText: nil, maxLength: 0, isSynchronous: false,
            dispatch: { dispatched.append($0) }, readAppValue: { nil }, writeText: { written.append($0) }
        )
        XCTAssertEqual(dispatched, [[.compositionEnd("")]])
        XCTAssertEqual(written, ["SERVER_RESET"], "deferred app value flushes on IME cancel")

        // isWritingFromHost guard ignores clamping, inFlight, and dispatch.
        dispatched.removeAll()
        written.removeAll()
        state.withHostWrite {
            state.processNativeEdit(
                currentText: "WAY_TOO_LONG",
                hasMarkedText: false,
                markedText: nil,
                maxLength: 3,
                isSynchronous: true,
                dispatch: { dispatched.append($0) },
                readAppValue: { "SERVER_RESET" },
                writeText: { written.append($0) }
            )
        }
        XCTAssertTrue(dispatched.isEmpty)
        XCTAssertTrue(written.isEmpty)
    }

    #if os(macOS)
    @MainActor
    func testMacPlatformTextInputImeInsertHitTestClipSelectionTeardownAndContentBox() throws {
        var s = SceneBuilder()
        let root = s.node(rect: CGRect(x: 0, y: 0, width: 240, height: 100))
        let input = s.node(parent: root, rect: CGRect(x: 10, y: 10, width: 180, height: 50))
        s.setChildren(root, [input])
        s.visual(input) {
            SceneBuilder.put(&$0, NveTables.Visual.borderWidth, Float(2))
        }
        s.edit(NveTables.Layout.key, words: NveTables.Layout.words, sparse: false, defaults: NveTables.Layout.defaults, input) {
            SceneBuilder.put(&$0, NveTables.Layout.paddingLeft, Float(8).bitPattern & ~3)
            SceneBuilder.put(&$0, NveTables.Layout.paddingTop, Float(6).bitPattern & ~3)
            SceneBuilder.put(&$0, NveTables.Layout.paddingRight, Float(10).bitPattern & ~3)
            SceneBuilder.put(&$0, NveTables.Layout.paddingBottom, Float(4).bitPattern & ~3)
        }
        let initialVal = s.str("ab")
        s.sparse(NveTables.TextInput.key, words: NveTables.TextInput.words, defaults: NveTables.TextInput.defaults, input) {
            SceneBuilder.put(&$0, NveTables.TextInput.value, initialVal)
            SceneBuilder.put(&$0, NveTables.TextInput.multiline, UInt32(1))
            SceneBuilder.put(&$0, NveTables.TextInput.maxLength, UInt32(4))
        }
        m.scene = s
        let controller = try NveSurfaceController(surfaceId: 45, getInterface: noApp, runtimeApi: MockRuntime.api)
        controller.store.sync()

        // Content-box rect subtracts border (2) + layout padding (L:8, T:6, R:10, B:4).
        let inputs = controller.store.textInputs()
        XCTAssertEqual(inputs.count, 1)
        XCTAssertEqual(inputs[0].frame, CGRect(x: 20, y: 18, width: 158, height: 36))

        // Real NvePlatformNSTextView hosted in an NSWindow as first responder: setMarkedText -> insertText
        // emits a single [.compositionEnd("你"), .textChange("ab你")] batch with zero re-entrant corruption.
        let desc = inputs[0]
        let coord = NveMacTextInputRepresentable.Coordinator(descriptor: desc, controller: controller)
        let tv = NvePlatformNSTextView(frame: NSRect(x: 0, y: 0, width: 158, height: 36))
        let win = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 80),
            styleMask: [.borderless], backing: .buffered, defer: false
        )
        win.contentView = tv
        tv.string = "ab"
        tv.setSelectedRange(NSRange(location: 2, length: 0))
        tv.delegate = coord
        tv.onMarkedTextChanged = { [weak coord, weak tv] in
            guard let coord, let tv else { return }
            coord.handleTextViewEdit(tv)
        }
        coord.textView = tv
        _ = win.makeFirstResponder(tv)
        _ = coord.state.shouldApplyAppValue("ab", currentShownText: "ab", hasMarkedText: false)

        func publishInput(_ value: String) {
            var next = SceneBuilder()
            let r = next.node(rect: CGRect(x: 0, y: 0, width: 240, height: 100))
            let n = next.node(parent: r, rect: CGRect(x: 10, y: 10, width: 180, height: 50))
            next.setChildren(r, [n])
            let v = next.str(value)
            next.sparse(NveTables.TextInput.key, words: NveTables.TextInput.words, defaults: NveTables.TextInput.defaults, n) {
                SceneBuilder.put(&$0, NveTables.TextInput.value, v)
                SceneBuilder.put(&$0, NveTables.TextInput.multiline, UInt32(1))
                SceneBuilder.put(&$0, NveTables.TextInput.maxLength, UInt32(4))
            }
            m.scene = next; m.frameId += 1
        }
        m.onDispatchEvents = { batch in
            for ev in batch where ev.kind == .textChange {
                if case let .text(val) = ev.payload { publishInput(val) }
            }
        }

        m.dispatched.removeAll()
        tv.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(m.dispatched.flatMap { $0 }.filter { $0.kind != .focus }.map(\.kind), [.compositionStart])

        m.dispatched.removeAll()
        tv.insertText("你", replacementRange: NSRange(location: NSNotFound, length: 0))
        let commitBatches = m.dispatched.filter { $0.contains { $0.kind != .focus && $0.kind != .blur } }
        XCTAssertEqual(commitBatches.count, 1, "insertText suppresses intermediate unmarkText/textDidChange so commit fires once")
        XCTAssertEqual(commitBatches[0].map(\.kind), [.compositionEnd, .textChange])
        if case let .text(committed) = commitBatches[0][0].payload { XCTAssertEqual(committed, "你") } else { XCTFail("expected committed text") }
        _ = win

        // Caret preservation on maxLength clamping & synchronous app snap-back.
        tv.string = "12345"; tv.setSelectedRange(NSRange(location: 2, length: 0))
        coord.handleTextViewEdit(tv)
        XCTAssertEqual(tv.string, "1234")
        XCTAssertEqual(tv.selectedRange(), NSRange(location: 2, length: 0), "mid-string caret preserved across maxLength clamp")

        m.onDispatchEvents = { _ in publishInput("SNAP") }
        tv.string = "abcd"; tv.setSelectedRange(NSRange(location: 3, length: 0))
        coord.handleTextViewEdit(tv)
        XCTAssertEqual(tv.string, "SNAP"); XCTAssertEqual(tv.selectedRange(), NSRange(location: 3, length: 0))
        m.onDispatchEvents = nil

        // Dismantle / teardown clears delegates and closes active focus + IME.
        _ = coord.state.handleFocusChange(true)
        _ = coord.state.handleUserEdit(newText: "SNAPz", hasMarkedText: true, markedText: "z")
        m.dispatched.removeAll()
        let host = NveMacClippedInputHostView(contentView: tv)
        NveMacTextInputRepresentable.dismantleNSView(host, coordinator: coord)
        XCTAssertNil(tv.delegate)
        XCTAssertNil(tv.onMarkedTextChanged)
        XCTAssertEqual(m.dispatched.last?.map(\.kind), [.compositionEnd, .blur])

        // NSTextField stuck-composition resolved on Return + readOnly submit guard.
        let singleField = NSTextField(frame: NSRect(x: 0, y: 0, width: 100, height: 24))
        singleField.stringValue = "hello"
        let defaultStyle = NveTextStyle(
            color: nil, fontSize: 14, fontWeight: 400, italic: false, align: .start, maxLines: 1, decoration: 0
        )
        let singleDesc = NveTextInputDescriptor(
            id: input, frame: CGRect(x: 0, y: 0, width: 100, height: 24), visibleClip: nil,
            props: NveTextInputProps(value: "hello", placeholder: "", secure: false, multiline: false, readOnly: false, disabled: false, inputKind: .text, maxLength: 0),
            style: defaultStyle, opacity: 1, accessibilityLabel: "Input", accessibilityHint: nil
        )
        let singleCoord = NveMacTextInputRepresentable.Coordinator(descriptor: singleDesc, controller: controller)
        singleCoord.textField = singleField
        _ = singleCoord.state.shouldApplyAppValue("hello", currentShownText: "hello", hasMarkedText: false)
        _ = singleCoord.state.handleUserEdit(newText: "helloz", hasMarkedText: true, markedText: "z")
        m.dispatched.removeAll()
        let dummyEditor = NSTextView(frame: .zero)
        dummyEditor.string = "hello"
        XCTAssertTrue(singleCoord.control(singleField, textView: dummyEditor, doCommandBy: #selector(NSResponder.insertNewline(_:))))
        XCTAssertFalse(singleCoord.state.isComposing, "Return clears stale composition without deadlocking")
        XCTAssertEqual(m.dispatched.last?.map(\.kind), [.compositionEnd, .submit])

        // readOnly blocks Return submit completely.
        singleCoord.descriptor = NveTextInputDescriptor(
            id: input, frame: CGRect(x: 0, y: 0, width: 100, height: 24), visibleClip: nil,
            props: NveTextInputProps(value: "hello", placeholder: "", secure: false, multiline: false, readOnly: true, disabled: false, inputKind: .text, maxLength: 0),
            style: defaultStyle, opacity: 1, accessibilityLabel: "Input", accessibilityHint: nil
        )
        m.dispatched.removeAll()
        XCTAssertFalse(singleCoord.control(singleField, textView: dummyEditor, doCommandBy: #selector(NSResponder.insertNewline(_:))))
        XCTAssertTrue(m.dispatched.isEmpty)

        // NveMacClippedInputHostView hitTest rejects clicks outside clipped scroll bounds.
        let parent = NSView(frame: NSRect(x: 0, y: 0, width: 200, height: 100))
        let inner = NSView(frame: .zero)
        let clippedHost = NveMacClippedInputHostView(contentView: inner)
        clippedHost.frame = NSRect(x: 0, y: 0, width: 50, height: 40)
        clippedHost.contentRect = CGRect(x: 0, y: 0, width: 120, height: 40)
        parent.addSubview(clippedHost)
        XCTAssertNotNil(clippedHost.hitTest(NSPoint(x: 25, y: 20)), "click inside clipped viewport hits input")
        XCTAssertNil(clippedHost.hitTest(NSPoint(x: 75, y: 20)), "click outside clipped viewport is rejected")
    }
    #endif

    // MARK: 4. Typography Deep Parity (P1 #5, P1 #6, P2 #14)

    @MainActor
    func testTypographyNamedBoldPercentLineHeightEllipsisOverlineCrlfAndInvalidLocalFallback() throws {
        let measurer = NveCoreTextMeasurer()

        // Named family bold weight actually resolves Menlo-Bold (not Menlo-Regular!).
        let reg = try XCTUnwrap(measurer.font(size: 16, weight: 400, italic: false, family: "Menlo"))
        let bold = try XCTUnwrap(measurer.font(size: 16, weight: 700, italic: false, family: "Menlo"))
        XCTAssertTrue(CTFontGetSymbolicTraits(bold).contains(.traitBold), "Menlo weight 700 resolves bold trait")
        XCTAssertNotEqual(CTFontCopyPostScriptName(bold) as String, CTFontCopyPostScriptName(reg) as String)

        // NVE_LINE_HEIGHT_PERCENT (250% of 16pt == 40pt points).
        var pctStyle = NveRuntimeTextStyle.sized(16)
        pctStyle.line_height_unit = UInt8(NVE_LINE_HEIGHT_PERCENT)
        pctStyle.line_height = 250.0
        var ptStyle = NveRuntimeTextStyle.sized(16)
        ptStyle.line_height_unit = UInt8(NVE_LINE_HEIGHT_POINTS)
        ptStyle.line_height = 40.0
        let pctH = measurer.measure(text: "One\nTwo\nThree", style: pctStyle, maxWidth: .infinity).height
        let ptH = measurer.measure(text: "One\nTwo\nThree", style: ptStyle, maxWidth: .infinity).height
        XCTAssertEqual(pctH, ptH, accuracy: 0.5)

        // CRLF with max_lines = 1 produces single-line height (no spurious blank line from \r).
        var oneLine = NveRuntimeTextStyle.sized(14)
        oneLine.max_lines = 1
        let singleH = measurer.measure(text: "Line 1", style: oneLine, maxWidth: .infinity).height
        let crlfH = measurer.measure(text: "Line 1\r\nLine 2", style: oneLine, maxWidth: .infinity).height
        XCTAssertEqual(crlfH, singleH, accuracy: 0.5)

        // Helper to paint one text node into a white CGImage.
        func paintText(_ text: String, width: CGFloat, height: CGFloat, spec: NveTypographySpec) throws -> CGImage {
            let black = NveColor(Rgba(packed: packRgba(0, 0, 0)))!
            let view = Canvas { ctx, size in
                ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white))
                ctx.withCGContext { cg in
                    NveTypographyEngine.shared.draw(
                        text: text,
                        spec: spec,
                        color: black,
                        in: CGRect(x: 0, y: 0, width: width, height: height),
                        into: cg
                    )
                }
            }.frame(width: width, height: height)
            let r = ImageRenderer(content: view)
            r.scale = 1
            return try XCTUnwrap(r.cgImage)
        }

        // Wrapped overflow maxLines = 1 truncates with ellipsis (differs from un-truncated clip).
        var truncSpec = NveTypographySpec(fontFamily: "Menlo", fontSize: 14, maxLines: 1)
        let truncImg = try paintText("The quick brown fox jumps over the lazy dog", width: 80, height: 24, spec: truncSpec)
        truncSpec.maxLines = 0
        let clipImg = try paintText("The quick brown fox jumps over the lazy dog", width: 80, height: 24, spec: truncSpec)
        var diffPixels = 0
        for y in 0..<24 {
            for x in 55..<80 {
                if rgbaPixel(truncImg, x: x, y: y) != rgbaPixel(clipImg, x: x, y: y) { diffPixels += 1 }
            }
        }
        XCTAssertGreaterThan(diffPixels, 5, "maxLines=1 wrapped overflow draws ellipsis distinct from raw clip")

        // Hard-newline overflow maxLines = 1 paints ellipsis after "First" (differs from plain "First").
        let nlSpec = NveTypographySpec(fontFamily: "Menlo", fontSize: 14, maxLines: 1)
        let nlImg = try paintText("First\nSecond", width: 140, height: 24, spec: nlSpec)
        let plainImg = try paintText("First", width: 140, height: 24, spec: nlSpec)
        var nlDiff = 0
        for y in 0..<24 {
            for x in 35..<80 {
                if rgbaPixel(nlImg, x: x, y: y) != rgbaPixel(plainImg, x: x, y: y) { nlDiff += 1 }
            }
        }
        XCTAssertGreaterThan(nlDiff, 3, "hard-newline overflow paints ellipsis after first line")

        // Overline paints visible ink in the top band (y in 0..<4) without clipping off-canvas.
        let plainTop = try paintText("aaaa", width: 80, height: 24, spec: NveTypographySpec(fontFamily: "Menlo", fontSize: 16))
        let overTop = try paintText(
            "aaaa", width: 80, height: 24,
            spec: NveTypographySpec(fontFamily: "Menlo", fontSize: 16, decoration: NveTextDecoration.overline)
        )
        XCTAssertEqual(countDark(plainTop, in: CGRect(x: 0, y: 0, width: 60, height: 4)), 0)
        XCTAssertGreaterThan(countDark(overTop, in: CGRect(x: 0, y: 0, width: 60, height: 4)), 10, "overline renders cleanly inside top band")

        // Invalid local fontSize (0) must inherit valid parent fontSize (24) instead of default 16.
        var s = SceneBuilder()
        let parent = s.node(rect: CGRect(x: 0, y: 0, width: 100, height: 50))
        let child = s.node(parent: parent, rect: CGRect(x: 0, y: 0, width: 100, height: 50))
        s.setChildren(parent, [child])
        s.typography(parent) {
            SceneBuilder.setExplicit(&$0, NveTables.Typography.fontSize)
            SceneBuilder.put(&$0, NveTables.Typography.fontSize, Float(24))
        }
        s.typography(child) {
            SceneBuilder.setExplicit(&$0, NveTables.Typography.fontSize)
            SceneBuilder.put(&$0, NveTables.Typography.fontSize, Float(0))
        }
        m.scene = s
        let controller = try NveSurfaceController(surfaceId: 46, getInterface: noApp, runtimeApi: MockRuntime.api)
        controller.store.sync()
        let style = try XCTUnwrap(controller.store.withTables { $0.textStyle(child.index) })
        XCTAssertEqual(style.fontSize, 24, accuracy: 0.01)
    }

    // MARK: 5. Canvas All-8 Opcodes, 1-Point Polyline, Transparent Clear & Bounded Cache Eviction (P1 #9, P2 #13)

    @MainActor
    func testCanvasAll8OpcodesOnePointPolylineTransparentClearAndBoundedCacheEviction() throws {
        // 1. Transparent Clear(0) uses .copy blend mode to erase prior red background to alpha 0.
        let clearBytes = Data(encodeWords([1, NveDrawListOpcode.clear.rawValue, packRgba(0, 0, 0, 0)]))
        let clearView = Canvas { ctx, size in
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.red))
            _ = try? NveDrawListRenderer.draw(clearBytes, into: &ctx, in: CGRect(x: 5, y: 5, width: 20, height: 20))
        }.frame(width: 30, height: 30)
        let rClear = ImageRenderer(content: clearView)
        rClear.scale = 1
        let clearCg = try XCTUnwrap(rClear.cgImage)
        XCTAssertEqual(rgbaPixel(clearCg, x: 15, y: 15).3, 0, "Clear(0) erases destination alpha via .copy")
        XCTAssertGreaterThan(rgbaPixel(clearCg, x: 2, y: 2).0, 200, "outside canvas stays red")

        // 2. All 8 opcodes + 1-point polyline dot + non-finite bounds guard.
        let white = packRgba(255, 255, 255)
        let green = packRgba(20, 220, 40)
        let blue = packRgba(20, 60, 240)
        let red = packRgba(240, 30, 30)
        let yellow = packRgba(240, 220, 20)
        let cyan = packRgba(20, 220, 220)
        let magenta = packRgba(220, 20, 220)
        let allOps = Data(encodeWords([
            9,
            NveDrawListOpcode.clear.rawValue, white,
            NveDrawListOpcode.fillRect.rawValue, Float(2).bitPattern, Float(2).bitPattern, Float(16).bitPattern, Float(16).bitPattern, red,
            NveDrawListOpcode.fillRoundedRect.rawValue, Float(22).bitPattern, Float(2).bitPattern, Float(16).bitPattern, Float(16).bitPattern, Float(4).bitPattern, green,
            NveDrawListOpcode.fillCircle.rawValue, Float(50).bitPattern, Float(10).bitPattern, Float(7).bitPattern, blue,
            NveDrawListOpcode.strokeLine.rawValue, Float(4).bitPattern, Float(28).bitPattern, Float(30).bitPattern, Float(28).bitPattern, Float(4).bitPattern, yellow,
            NveDrawListOpcode.strokePolyline.rawValue, Float(4).bitPattern, cyan, 2,
            Float(4).bitPattern, Float(42).bitPattern, Float(30).bitPattern, Float(42).bitPattern,
            NveDrawListOpcode.strokePolyline.rawValue, Float(8).bitPattern, magenta, 1,
            Float(50).bitPattern, Float(42).bitPattern,
            NveDrawListOpcode.clipRect.rawValue, Float(64).bitPattern, Float(2).bitPattern, Float(10).bitPattern, Float(10).bitPattern,
            NveDrawListOpcode.restoreClip.rawValue,
        ]))
        let opsView = Canvas { ctx, _ in
            _ = try? NveDrawListRenderer.draw(allOps, into: &ctx, in: CGRect(x: 0, y: 0, width: CGFloat.nan, height: 40))
            _ = try? NveDrawListRenderer.draw(allOps, into: &ctx, in: CGRect(x: 0, y: 0, width: 80, height: 60))
        }.frame(width: 80, height: 60)
        let rOps = ImageRenderer(content: opsView)
        rOps.scale = 1
        let opsCg = try XCTUnwrap(rOps.cgImage)
        XCTAssertGreaterThan(rgbaPixel(opsCg, x: 10, y: 10).0, 200, "FillRect painted red")
        XCTAssertGreaterThan(rgbaPixel(opsCg, x: 30, y: 10).1, 180, "FillRoundedRect painted green")
        XCTAssertGreaterThan(rgbaPixel(opsCg, x: 50, y: 10).2, 200, "FillCircle painted blue")
        XCTAssertGreaterThan(rgbaPixel(opsCg, x: 16, y: 28).0, 200, "StrokeLine painted yellow")
        XCTAssertGreaterThan(rgbaPixel(opsCg, x: 16, y: 42).1, 180, "2-pt StrokePolyline painted cyan")
        XCTAssertGreaterThan(rgbaPixel(opsCg, x: 50, y: 42).0, 180, "1-pt StrokePolyline painted magenta dot")
        XCTAssertGreaterThan(rgbaPixel(opsCg, x: 50, y: 42).2, 180, "1-pt StrokePolyline painted magenta dot")

        // 3. Bounded cache eviction in NveSceneStore (> 256 resource IDs).
        var s = SceneBuilder()
        _ = s.node(rect: CGRect(x: 0, y: 0, width: 40, height: 40))
        m.scene = s
        let oneClear = encodeWords([1, NveDrawListOpcode.clear.rawValue, white])
        for id: UInt32 in 1...300 {
            m.resources[id] = oneClear
            m.resourceKinds[id] = NveDrawList.kind.value
        }
        let controller = try NveSurfaceController(surfaceId: 47, getInterface: noApp, runtimeApi: MockRuntime.api)
        controller.store.sync()
        for id: UInt32 in 1...300 {
            XCTAssertNotNil(controller.store.drawList(resource: id))
            XCTAssertLessThanOrEqual(controller.store.cachedDrawListCount, NveSceneStore.maxCachedResources)
        }
        XCTAssertNotNil(controller.store.drawList(resource: 1), "evicted IDs re-fetch cleanly on demand")
    }
}
