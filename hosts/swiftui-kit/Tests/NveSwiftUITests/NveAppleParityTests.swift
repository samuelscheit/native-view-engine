import CNveAbi
import CoreGraphics
import CoreText
import Foundation
import NveHost
import SwiftUI
import XCTest
@testable import NveSwiftUI

private func noApp(_ id: UInt32) -> UnsafeRawPointer? { nil }

private struct SampleParityProps: Equatable, NveDirectEncodable {
    var title: String
    var count: Int64
    var directPropCount: Int { 2 }
    func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
        builder.write("title", title)
        builder.write("count", count)
    }
}

private func packRgba(_ r: UInt8, _ g: UInt8, _ b: UInt8, _ a: UInt8 = 255) -> UInt32 {
    UInt32(r) | (UInt32(g) << 8) | (UInt32(b) << 16) | (UInt32(a) << 24)
}

private func encodeDrawListWords(_ words: [UInt32]) -> [UInt8] {
    var out: [UInt8] = []
    out.reserveCapacity(words.count * 4)
    for w in words {
        withUnsafeBytes(of: w.littleEndian) { out.append(contentsOf: $0) }
    }
    return out
}

private func pixelAt(_ image: CGImage, x: Int, y: Int) -> (UInt8, UInt8, UInt8, UInt8) {
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

final class NveAppleParityTests: XCTestCase {
    private var m: MockRuntime { MockRuntime.shared }

    override func setUp() {
        super.setUp()
        MockRuntime.shared.reset()
    }

    // MARK: 1. Model A `measure_surface` + Baselines

    func testMeasureSurfaceFlagsConstraintsAndBaselines() throws {
        let controller = try NveSurfaceController(surfaceId: 3, getInterface: noApp, runtimeApi: MockRuntime.api)
        m.measureHandler = { c, flags in
            let ideal = (flags & UInt32(NVE_MEASURE_FLAG_IDEAL)) != 0
            return NveSurfaceMeasure(
                width: ideal ? 180 : min(180, c.max_w),
                height: ideal ? 48 : min(96, c.max_h),
                first_baseline: 16.5,
                last_baseline: 38.25
            )
        }

        // Unspecified proposal sets DRY_RUN | IDEAL and returns finite baselines.
        let ideal = controller.measureSurface(proposal: .unspecified)
        XCTAssertEqual(ideal.size, CGSize(width: 180, height: 48))
        XCTAssertEqual(ideal.firstBaseline, 16.5)
        XCTAssertEqual(ideal.lastBaseline, 38.25)
        XCTAssertEqual(m.measureCalls.last?.flags, UInt32(NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL))
        XCTAssertEqual(m.measureCalls.last?.constraints.max_w, .infinity)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_h, .infinity)

        // Constrained proposal sets DRY_RUN only (no IDEAL).
        let constrained = controller.measureSurface(proposal: ProposedViewSize(width: 100, height: 40))
        XCTAssertEqual(constrained.size, CGSize(width: 100, height: 40))
        XCTAssertEqual(m.measureCalls.last?.flags, UInt32(NVE_MEASURE_FLAG_DRY_RUN))
        XCTAssertEqual(m.measureCalls.last?.constraints.max_w, 100)
        XCTAssertEqual(m.measureCalls.last?.constraints.max_h, 40)

        // NaN baselines map cleanly to nil.
        m.measureHandler = { _, _ in
            NveSurfaceMeasure(width: 90, height: 30, first_baseline: .nan, last_baseline: .nan)
        }
        let noBaselines = controller.measureSurface(proposal: .unspecified)
        XCTAssertNil(noBaselines.firstBaseline)
        XCTAssertNil(noBaselines.lastBaseline)

        // Layout cache & explicitAlignment calculations.
        m.measureHandler = { _, _ in
            NveSurfaceMeasure(width: 120, height: 50, first_baseline: 14, last_baseline: 42)
        }
        m.measureCalls.removeAll()
        let layout = NveIntrinsicLayout(controller: controller, intrinsicRevision: controller.intrinsicSizeRevision)
        var cache = NveIntrinsicLayout.Cache()
        let measured = layout.measure(proposal: ProposedViewSize(width: 120, height: nil), cache: &cache)
        XCTAssertEqual(measured.size, CGSize(width: 120, height: 50))
        XCTAssertEqual(m.measureCalls.count, 1)

        // Repeated measure with same proposal & revision reuses cache without another FFI call.
        let cached = layout.measure(proposal: ProposedViewSize(width: 120, height: nil), cache: &cache)
        XCTAssertEqual(cached, measured)
        XCTAssertEqual(m.measureCalls.count, 1)

        // Bumping intrinsicRevision invalidates layout cache.
        let bumpedLayout = NveIntrinsicLayout(controller: controller, intrinsicRevision: controller.intrinsicSizeRevision + 1)
        _ = bumpedLayout.measure(proposal: ProposedViewSize(width: 120, height: nil), cache: &cache)
        XCTAssertEqual(m.measureCalls.count, 2)

        // Runtime error falls back cleanly without crashing.
        m.failNextWith = Int32(NVE_RUNTIME_BUSY.rawValue)
        let fallback = controller.measureSurface(proposal: .unspecified)
        XCTAssertEqual(fallback.size, controller.fallbackIntrinsicSize)
        XCTAssertNil(fallback.firstBaseline)
        XCTAssertNil(fallback.lastBaseline)
    }

    // MARK: 2. Intrinsic Size Invalidation Callback + Deinit / Detach Safety

    func testIntrinsicSizeInvalidationCallbackFiresAndUnregistersOnDetachAndDeinit() throws {
        weak var weakController: NveSurfaceController?
        do {
            let controller = try NveSurfaceController(surfaceId: 9, getInterface: noApp, runtimeApi: MockRuntime.api)
            weakController = controller
            XCTAssertNotNil(m.intrinsicInvalidated.0, "slot 29 registered on init/attach")
            XCTAssertEqual(controller.intrinsicSizeRevision, 0)

            // Burst of invalidations coalesces into main-queue revision update.
            let (fn, ctx) = m.intrinsicInvalidated
            fn?(ctx)
            fn?(ctx)
            let exp = expectation(description: "intrinsic invalidation published")
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { exp.fulfill() }
            wait(for: [exp], timeout: 1)
            XCTAssertGreaterThanOrEqual(controller.intrinsicSizeRevision, 1)

            // Detach unregisters slot 29 cleanly; re-attach restores it.
            try controller.detach()
            XCTAssertNil(m.intrinsicInvalidated.0, "slot 29 cleared on detach")
            try controller.attach(NveResolvedInterface(getInterface: noApp, source: .explicit))
            XCTAssertNotNil(m.intrinsicInvalidated.0, "slot 29 restored on re-attach")
        }
        // Controller deallocated: slot 29 and wake callback are both unregistered (zero UAF).
        XCTAssertNil(weakController)
        XCTAssertNil(m.intrinsicInvalidated.0, "slot 29 cleared on deinit")
        XCTAssertNil(m.wake.0, "wake callback cleared on deinit")
        XCTAssertGreaterThanOrEqual(m.intrinsicInvalidatedClearCount, 2)
    }

    // MARK: 3. Deterministic PropsToken Content Equality

    func testPropsTokenDeduplicatesEqualDynamicAndDirectPropsWithoutUuidChurn() {
        let a = NveSurfaceRepresentable(componentName: "Card", props: ["title": "Hello", "count": 42])
        let b = NveSurfaceRepresentable(componentName: "Card", props: ["count": 42, "title": "Hello"])
        let c = NveSurfaceRepresentable(componentName: "Card", props: ["title": "Changed", "count": 42])
        XCTAssertEqual(a.propsToken, b.propsToken, "canonicalized props with same content produce equal tokens")
        XCTAssertNotEqual(a.propsToken, c.propsToken, "changed prop value produces unequal token")

        let d1 = NveSurfaceRepresentable(componentName: "Card", directProps: SampleParityProps(title: "A", count: 1))
        let d2 = NveSurfaceRepresentable(componentName: "Card", directProps: SampleParityProps(title: "A", count: 1))
        let d3 = NveSurfaceRepresentable(componentName: "Card", directProps: SampleParityProps(title: "A", count: 2))
        XCTAssertEqual(d1.propsToken, d2.propsToken, "equal direct props produce equal tokens")
        XCTAssertNotEqual(d1.propsToken, d3.propsToken, "changed direct props produce unequal tokens")
    }

    // MARK: 4. `nve.canvas` Scene Table + `NveDrawListRenderer` Integration

    @MainActor
    func testSceneRendererDrawsNveCanvasNodeWithClipOriginAndKindValidation() throws {
        var s = SceneBuilder()
        let root = s.node(rect: CGRect(x: 0, y: 0, width: 100, height: 100))
        let canvasNode = s.node(parent: root, rect: CGRect(x: 20, y: 20, width: 40, height: 40))
        let badCanvasNode = s.node(parent: root, rect: CGRect(x: 65, y: 20, width: 20, height: 20))
        s.setChildren(root, [canvasNode, badCanvasNode])
        s.visual(root) { SceneBuilder.put(&$0, NveTables.Visual.background, SceneBuilder.rgba(0, 0, 0)) }
        s.sparse(NveCanvasTable.key, words: 2, defaults: [0, 0], canvasNode) {
            SceneBuilder.put(&$0, NveCanvasTable.drawList, UInt32(7))
            SceneBuilder.put(&$0, NveCanvasTable.generation, UInt32(1))
        }
        s.sparse(NveCanvasTable.key, words: 2, defaults: [0, 0], badCanvasNode) {
            SceneBuilder.put(&$0, NveCanvasTable.drawList, UInt32(8))
            SceneBuilder.put(&$0, NveCanvasTable.generation, UInt32(1))
        }
        m.scene = s

        // DrawList: clear green, plus an oversized blue rect extending past canvas bounds (must be clipped!).
        let green = packRgba(16, 200, 64)
        let blue = packRgba(20, 80, 240)
        let drawBytes = encodeDrawListWords([
            2,
            NveDrawListOpcode.clear.rawValue, green,
            NveDrawListOpcode.fillRect.rawValue,
            Float(10).bitPattern, Float(10).bitPattern, Float(80).bitPattern, Float(80).bitPattern, blue,
        ])
        m.resources[7] = drawBytes
        m.resourceKinds[7] = NveDrawList.kind.value
        // Resource 8 has identical bytes but wrong FlatResource kind (`image_src`) -> MUST be rejected!
        m.resources[8] = drawBytes
        m.resourceKinds[8] = Key64.of("nve.res.image_src").value

        let controller = try NveSurfaceController(getInterface: noApp, runtimeApi: MockRuntime.api)
        controller.store.sync()

        XCTAssertEqual(controller.store.drawList(resource: 7), drawBytes)
        XCTAssertNil(controller.store.drawList(resource: 8), "wrong FlatResource kind must be rejected")

        let view = Canvas { ctx, _ in NveSceneRenderer.draw(store: controller.store, into: &ctx) }
            .frame(width: 100, height: 100)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        let cg = try XCTUnwrap(renderer.cgImage)

        // Inside canvas (20,20)-(60,60), at local (4,4) -> global (24,24): green clear
        let clearPx = pixelAt(cg, x: 24, y: 24)
        XCTAssertEqual(clearPx.1, 200, accuracy: 3)
        // Inside canvas at local (20,20) -> global (40,40): blue fillRect
        let rectPx = pixelAt(cg, x: 40, y: 40)
        XCTAssertEqual(rectPx.2, 240, accuracy: 3)
        // Outside canvas at global (62,40): clipped, stays black root background!
        let outsidePx = pixelAt(cg, x: 62, y: 40)
        XCTAssertEqual(outsidePx.2, 0, accuracy: 2)
        // Bad-kind canvas at (75,30): stays black root background!
        let badPx = pixelAt(cg, x: 75, y: 30)
        XCTAssertEqual(badPx.1, 0, accuracy: 2)

        // In-place resource rewrite listed in `dirtyResources` invalidates cached draw list.
        let redBytes = encodeDrawListWords([1, NveDrawListOpcode.clear.rawValue, packRgba(240, 20, 20)])
        m.resources[7] = redBytes
        m.frameId = 2
        m.dirtyResources = [7]
        controller.store.sync()
        XCTAssertEqual(controller.store.drawList(resource: 7), redBytes)
    }

    // MARK: 5. Typography Parity (`font_family`, `letter_spacing`, `line_height`, `max_lines`, wrapping, decorations)

    @MainActor
    func testTypographyMeasurementAndPaintingParity() throws {
        let measurer = NveCoreTextMeasurer()

        // CSS font-family fallback skips nonexistent family and resolves Menlo.
        let font = try XCTUnwrap(measurer.font(size: 16, weight: 400, italic: false, family: "NonexistentNveFont, Menlo, monospace"))
        let familyName = CTFontCopyFamilyName(font) as String
        XCTAssertEqual(familyName, "Menlo")

        // Positive letter_spacing widens measured text monotonically.
        let tight = NveRuntimeTextStyle.sized(16)
        var wide = NveRuntimeTextStyle.sized(16)
        wide.letter_spacing = 6.0
        let tightSize = measurer.measure(text: "Typography", style: tight, maxWidth: .infinity)
        let wideSize = measurer.measure(text: "Typography", style: wide, maxWidth: .infinity)
        XCTAssertGreaterThan(wideSize.width, tightSize.width + 30)

        // Explicit line_height increases multi-line height.
        let normalLH = NveRuntimeTextStyle.sized(14)
        var tallLH = NveRuntimeTextStyle.sized(14)
        tallLH.line_height_unit = UInt8(NVE_LINE_HEIGHT_POINTS)
        tallLH.line_height = 40.0
        let multiText = "Line one\nLine two\nLine three"
        let normalSize = measurer.measure(text: multiText, style: normalLH, maxWidth: .infinity)
        let tallSize = measurer.measure(text: multiText, style: tallLH, maxWidth: .infinity)
        XCTAssertGreaterThan(tallSize.height, normalSize.height + 40)

        // max_lines caps multi-line height cleanly.
        var capped = normalLH
        capped.max_lines = 1
        let cappedSize = measurer.measure(text: multiText, style: capped, maxWidth: .infinity)
        XCTAssertLessThan(cappedSize.height, normalSize.height)

        // white_space: nowrap never wraps even at narrow maxWidth; word_break: breakAll min-content < longest word.
        let phrase = "considerably long"
        let fullW = measurer.measure(text: phrase, style: normalLH, maxWidth: .infinity)
        var noWrap = normalLH
        noWrap.white_space = UInt8(NVE_WHITE_SPACE_NO_WRAP)
        let nwNarrow = measurer.measure(text: phrase, style: noWrap, maxWidth: 30)
        XCTAssertEqual(nwNarrow.width, fullW.width, accuracy: 1.0)
        XCTAssertEqual(nwNarrow.height, fullW.height, accuracy: 1.0)

        let wordMin = measurer.measure(text: phrase, style: normalLH, maxWidth: 0)
        var breakAll = normalLH
        breakAll.word_break = UInt8(NVE_WORD_BREAK_BREAK_ALL)
        let charMin = measurer.measure(text: phrase, style: breakAll, maxWidth: 0)
        XCTAssertGreaterThan(wordMin.width, charMin.width)
        XCTAssertLessThan(wordMin.width, fullW.width)

        // Scene typography inheritance & painting with underline + strikethrough + fontFamily renders non-empty ink.
        var s = SceneBuilder()
        let root = s.node(rect: CGRect(x: 0, y: 0, width: 160, height: 60))
        let label = s.node(parent: root, rect: CGRect(x: 4, y: 4, width: 150, height: 50))
        s.setChildren(root, [label])
        s.visual(root) { SceneBuilder.put(&$0, NveTables.Visual.background, SceneBuilder.rgba(255, 255, 255)) }
        let famSpan = s.str("Menlo, monospace")
        s.typography(root) {
            SceneBuilder.setExplicit(
                &$0,
                NveTables.Typography.fontFamily,
                NveTables.Typography.letterSpacing,
                NveTables.Typography.lineHeight,
                NveTables.Typography.textDecoration,
                NveTables.Typography.whiteSpace,
                NveTables.Typography.wordBreak,
                NveTables.Typography.overflowWrap
            )
            SceneBuilder.put(&$0, NveTables.Typography.fontFamily, famSpan)
            SceneBuilder.put(&$0, NveTables.Typography.letterSpacing, Float(2.0))
            SceneBuilder.put(&$0, NveTables.Typography.lineHeight, Float(28.0).bitPattern & ~3) // points
            SceneBuilder.put(&$0, NveTables.Typography.textDecoration, NveTextDecoration.underline | NveTextDecoration.lineThrough)
            SceneBuilder.put(&$0, NveTables.Typography.whiteSpace, NveWhiteSpace.preWrap.rawValue)
            SceneBuilder.put(&$0, NveTables.Typography.wordBreak, NveWordBreak.breakWord.rawValue)
            SceneBuilder.put(&$0, NveTables.Typography.overflowWrap, NveOverflowWrap.anywhere.rawValue)
        }
        s.text(label, "Decorated")
        m.scene = s

        let controller = try NveSurfaceController(getInterface: noApp, runtimeApi: MockRuntime.api)
        controller.store.sync()
        let resolved = try XCTUnwrap(controller.store.withTables { $0.textStyle(label.index) })
        XCTAssertEqual(resolved.fontFamily, "Menlo, monospace")
        XCTAssertEqual(resolved.letterSpacing, 2.0, accuracy: 0.01)
        XCTAssertEqual(resolved.lineHeight.unit, .points)
        XCTAssertEqual(resolved.lineHeight.value, 28.0, accuracy: 0.1)
        XCTAssertEqual(resolved.decoration, NveTextDecoration.underline | NveTextDecoration.lineThrough)
        XCTAssertEqual(resolved.whiteSpace, .preWrap)
        XCTAssertEqual(resolved.wordBreak, .breakWord)
        XCTAssertEqual(resolved.overflowWrap, .anywhere)

        let canvas = Canvas { ctx, _ in NveSceneRenderer.draw(store: controller.store, into: &ctx) }
            .frame(width: 160, height: 60)
        let renderer = ImageRenderer(content: canvas)
        renderer.scale = 1
        let cg = try XCTUnwrap(renderer.cgImage)
        var darkPixels = 0
        for y in 4..<40 {
            for x in 4..<140 {
                let p = pixelAt(cg, x: x, y: y)
                if p.0 < 120 && p.1 < 120 && p.2 < 120 { darkPixels += 1 }
            }
        }
        XCTAssertGreaterThan(darkPixels, 40, "CoreText painted visible decorated glyphs")
        _ = tight
    }

    // MARK: 6. Editable `TextInput` IME + Echo Suppression + Mounted Overlay Skipping

    @MainActor
    func testControlledTextImeGuardEchoSuppressionAndMountedOverlaySkip() throws {
        let state = NveControlledTextState()

        // Initial app publish writes into control.
        XCTAssertTrue(state.shouldApplyAppValue("hello", currentShownText: "", hasMarkedText: false))

        // Start CJK IME composition: emits CompositionStart, NEVER TextChange.
        let start = state.handleUserEdit(newText: "helloni", hasMarkedText: true, markedText: "ni")
        XCTAssertEqual(start, [.compositionStart("ni")])
        XCTAssertTrue(state.isComposing)

        // While marked text is active, app re-render MUST NOT clobber active preedit.
        XCTAssertFalse(state.shouldApplyAppValue("server-reset", currentShownText: "helloni", hasMarkedText: true))

        // Preedit update emits CompositionUpdate only.
        let update = state.handleUserEdit(newText: "hellonihao", hasMarkedText: true, markedText: "nihao")
        XCTAssertEqual(update, [.compositionUpdate("nihao")])

        // Submit (Return) during active composition is suppressed so IME can commit candidate.
        XCTAssertNil(state.handleSubmit(currentText: "hellonihao", hasMarkedText: true))

        // Candidate commit clears marked text -> emits CompositionEnd("你好") + TextChange("hello你好").
        let commit = state.handleUserEdit(newText: "hello你好", hasMarkedText: false, markedText: nil)
        XCTAssertEqual(commit, [.compositionEnd("你好"), .textChange("hello你好")])
        XCTAssertFalse(state.isComposing)

        // Async echo of in-flight edit is absorbed without overwriting caret/typing.
        let typingMore = state.handleUserEdit(newText: "hello你好!", hasMarkedText: false, markedText: nil)
        XCTAssertEqual(typingMore, [.textChange("hello你好!")])
        XCTAssertFalse(state.shouldApplyAppValue("hello你好", currentShownText: "hello你好!", hasMarkedText: false),
                       "stale echo of earlier in-flight edit must not clobber newer typing")
        XCTAssertFalse(state.shouldApplyAppValue("hello你好!", currentShownText: "hello你好!", hasMarkedText: false),
                       "latest in-flight echo absorbed")

        // Genuine app transform (not in-flight) overwrites control.
        XCTAssertTrue(state.shouldApplyAppValue("HELLO", currentShownText: "hello你好!", hasMarkedText: false))

        // Host programmatic writes never emit user edit events.
        let ignored = state.withHostWrite {
            state.handleUserEdit(newText: "HELLO", hasMarkedText: false, markedText: nil)
        }
        XCTAssertTrue(ignored.isEmpty)

        // Synchronous local app answer snaps back rejected/transformed edit.
        _ = state.handleUserEdit(newText: "HELLO9", hasMarkedText: false, markedText: nil)
        XCTAssertEqual(state.reconcileSynchronousAnswer(appValue: "HELLO", hasMarkedText: false), "HELLO")

        // Blur mid-composition emits balanced CompositionEnd("") + Blur.
        XCTAssertEqual(state.handleFocusChange(true), [.focus])
        XCTAssertEqual(state.handleUserEdit(newText: "HELLOz", hasMarkedText: true, markedText: "z"), [.compositionStart("z")])
        XCTAssertEqual(state.handleFocusChange(false), [.compositionEnd(""), .blur])
        XCTAssertFalse(state.isComposing)

        // Submit outside composition emits .submit.
        XCTAssertEqual(state.handleSubmit(currentText: "HELLO", hasMarkedText: false), .submit("HELLO"))

        // Verify `NveSceneStore.textInputs()` and `skipTextInputIds` passive paint skipping.
        var s = SceneBuilder()
        let root = s.node(rect: CGRect(x: 0, y: 0, width: 200, height: 80))
        let input = s.node(parent: root, rect: CGRect(x: 10, y: 10, width: 180, height: 40))
        s.setChildren(root, [input])
        s.visual(root) { SceneBuilder.put(&$0, NveTables.Visual.background, SceneBuilder.rgba(255, 255, 255)) }
        s.visual(input) { SceneBuilder.put(&$0, NveTables.Visual.background, SceneBuilder.rgba(255, 255, 255)) }
        let val = s.str("VISIBLE_TEXT")
        let ph = s.str("Enter...")
        s.sparse(NveTables.TextInput.key, words: NveTables.TextInput.words, defaults: NveTables.TextInput.defaults, input) {
            SceneBuilder.put(&$0, NveTables.TextInput.value, val)
            SceneBuilder.put(&$0, NveTables.TextInput.placeholder, ph)
            SceneBuilder.put(&$0, NveTables.TextInput.readOnly, UInt32(1))
            SceneBuilder.put(&$0, NveTables.TextInput.inputKind, NveInputKind.email.rawValue)
        }
        m.scene = s
        let controller = try NveSurfaceController(getInterface: noApp, runtimeApi: MockRuntime.api)
        controller.store.sync()

        let inputs = controller.store.textInputs()
        XCTAssertEqual(inputs.count, 1)
        XCTAssertEqual(inputs[0].id, input)
        XCTAssertEqual(inputs[0].props.value, "VISIBLE_TEXT")
        XCTAssertEqual(inputs[0].props.placeholder, "Enter...")
        XCTAssertTrue(inputs[0].props.readOnly)
        XCTAssertEqual(inputs[0].props.inputKind, .email)

        func countDarkPixels(skip: Set<NodeId>) throws -> Int {
            let canvas = Canvas { ctx, _ in
                NveSceneRenderer.draw(store: controller.store, into: &ctx, skipTextInputIds: skip)
            }.frame(width: 200, height: 80)
            let r = ImageRenderer(content: canvas)
            r.scale = 1
            let cg = try XCTUnwrap(r.cgImage)
            var count = 0
            for y in 12..<48 {
                for x in 12..<188 {
                    let p = pixelAt(cg, x: x, y: y)
                    if p.0 < 150 && p.1 < 150 && p.2 < 150 { count += 1 }
                }
            }
            return count
        }

        XCTAssertGreaterThan(try countDarkPixels(skip: []), 30, "passive drawTextInput paints when not mounted")
        XCTAssertEqual(try countDarkPixels(skip: [input]), 0, "mounted input skips passive canvas drawTextInput")
    }
}
