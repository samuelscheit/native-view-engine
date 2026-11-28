import CNveAbi
import NveHost
import Foundation
import NveSwiftUI
import SwiftUI
import XCTest

private final class StaticMockCounter: @unchecked Sendable {
    static let shared = StaticMockCounter()
    private let lock = NSLock()
    var createCount = 0
    var destroyCount = 0
    var lastSurfaceId: UInt32 = 0

    func reset() {
        lock.lock()
        createCount = 0
        destroyCount = 0
        lastSurfaceId = 0
        lock.unlock()
    }

    func onCreate(_ surfaceId: UInt32) -> NveAppHandle? {
        lock.lock()
        createCount += 1
        lastSurfaceId = surfaceId
        let ptr = UnsafeMutableRawPointer(bitPattern: 0x5000 + createCount * 16)!
        lock.unlock()
        return ptr
    }

    func onDestroy(_ handle: NveAppHandle?) {
        guard handle != nil else { return }
        lock.lock()
        destroyCount += 1
        lock.unlock()
    }
}

private enum StaticMockVtable {
    static let surfaceApi: UnsafePointer<NveSurfaceApi> = {
        let ptr = UnsafeMutablePointer<NveSurfaceApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveSurfaceApi(
                create: { sid in StaticMockCounter.shared.onCreate(sid) },
                destroy: { handle in StaticMockCounter.shared.onDestroy(handle) },
                render: { _, _ in NveBufferSlice(ptr: nil, len: 0) },
                dispatch_events: { _, _ in 0 }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let layoutApi: UnsafePointer<NveLayoutApi> = {
        let ptr = UnsafeMutablePointer<NveLayoutApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveLayoutApi(
                measure: { _, _ in NveIntrinsicSize(width: 320, height: 180) },
                hit_test: { _, _, _, _ in UInt8(NVE_GESTURE_CLAIM.rawValue) }
            )
        )
        return UnsafePointer(ptr)
    }()
}

private func staticMockGetInterface(_ id: UInt32) -> UnsafeRawPointer? {
    switch id {
    case UInt32(NVE_IFACE_SURFACE): return UnsafeRawPointer(StaticMockVtable.surfaceApi)
    case UInt32(NVE_IFACE_LAYOUT): return UnsafeRawPointer(StaticMockVtable.layoutApi)
    default: return nil
    }
}

final class NveMultiApplePlatformTests: XCTestCase {
    override func setUp() {
        super.setUp()
        StaticMockCounter.shared.reset()
    }

    func testStaticInterfaceResolverPrecedence() throws {
        let table = NveStaticInterfaceTable()
        table.register(symbol: "nve_static_app", getInterface: staticMockGetInterface)
        table.register(symbol: "  ", getInterface: staticMockGetInterface)
        XCTAssertEqual(table.registeredSymbols, ["nve_static_app"])

        // Static registrations resolve without dlopen; the entry point is handed to the runtime as-is.
        let fromTable = try XCTUnwrap(NveInterfaceResolver.resolve(symbol: "nve_static_app", staticTable: table))
        XCTAssertEqual(fromTable.source, .staticTable)
        XCTAssertNotNil(fromTable.getInterface(UInt32(NVE_IFACE_SURFACE)))
        XCTAssertNil(fromTable.getInterface(UInt32(NVE_IFACE_DEV)))

        // Explicit entry points win over the table.
        let explicit = try XCTUnwrap(
            NveInterfaceResolver.resolve(symbol: "nve_static_app", getInterface: staticMockGetInterface, staticTable: table))
        XCTAssertEqual(explicit.source, .explicit)
        XCTAssertNil(NveInterfaceResolver.resolve(symbol: "nve_symbol_that_does_not_exist", staticTable: table))

        XCTAssertTrue(table.unregister(symbol: "nve_static_app"))
        XCTAssertFalse(table.unregister(symbol: "nve_static_app"))
        XCTAssertTrue(table.registeredSymbols.isEmpty)
    }

    func testAppleDeviceProfilesAcrossAllFiveApplePlatforms() {
        XCTAssertEqual(NveApplePlatform.allCases.count, 5)
        XCTAssertEqual(NveAppleDeviceProfile.current.platform, NveApplePlatform.current)

        let mac = NveAppleDeviceProfile.forPlatform(.macOS)
        XCTAssertTrue(mac.supportsPointer)
        XCTAssertTrue(mac.supportsWebView)
        XCTAssertFalse(mac.isTenFootUI)
        XCTAssertFalse(mac.isCompactWatch)
        XCTAssertFalse(mac.isSpatial)

        let ios = NveAppleDeviceProfile.forPlatform(.iOS)
        XCTAssertTrue(ios.supportsTouch)
        XCTAssertTrue(ios.supportsWebView)
        XCTAssertEqual(ios.recommendedMinimumHitTargetPt, 44.0)

        let tv = NveAppleDeviceProfile.forPlatform(.tvOS)
        XCTAssertTrue(tv.isTenFootUI)
        XCTAssertTrue(tv.supportsSiriRemote)
        XCTAssertFalse(tv.supportsWebView)
        XCTAssertFalse(tv.supportsTouch)
        XCTAssertEqual(tv.defaultOverscanRatio, 0.05, accuracy: 1e-6)
        XCTAssertEqual(tv.recommendedMinimumHitTargetPt, 66.0)

        let watch = NveAppleDeviceProfile.forPlatform(.watchOS)
        XCTAssertTrue(watch.isCompactWatch)
        XCTAssertTrue(watch.supportsDigitalCrown)
        XCTAssertTrue(watch.supportsTouch)
        XCTAssertFalse(watch.supportsWebView)
        XCTAssertEqual(watch.recommendedMinimumHitTargetPt, 38.0)

        let vision = NveAppleDeviceProfile.forPlatform(.visionOS)
        XCTAssertTrue(vision.isSpatial)
        XCTAssertTrue(vision.supportsPointer)
        XCTAssertTrue(vision.supportsTouch)
        XCTAssertTrue(vision.supportsWebView)
        XCTAssertEqual(vision.recommendedMinimumHitTargetPt, 60.0)
    }

    func testTvOsSiriRemoteKeyDownMappingAndOverscanInsets() throws {
        let cases: [(NveTvRemoteCommand, String)] = [
            (.up, "ArrowUp"),
            (.down, "ArrowDown"),
            (.left, "ArrowLeft"),
            (.right, "ArrowRight"),
            (.select, "Select"),
            (.playPause, "MediaPlayPause"),
            (.exit, "Back"),
        ]

        for (command, expectedKey) in cases {
            XCTAssertEqual(command.keyName, expectedKey)
            let event = command.makeKeyDownEvent(surfaceId: 7, nodeId: 19, timestampUs: 123_456)
            XCTAssertEqual(event.surfaceId, 7)
            XCTAssertEqual(event.nodeId, 19)
            XCTAssertEqual(event.kind, .keyDown)
            XCTAssertNil(event.name)
            XCTAssertEqual(event.payload, .key(key: expectedKey))

            // Direct `NveDirectEvent` batch roundtrip (what `dispatch_events` receives).
            try NveDirectEventWriter.withBatch([event]) { batch in
                XCTAssertEqual(NveDirectEventWriter.decodeBatch(batch), [event])
            }
        }

        // Typed KeyDown with modifiers + code roundtrips cleanly through EventRing and NveDirectEvent.
        let modified = NveEvent(
            surfaceId: 7,
            nodeId: 19,
            timestampUs: 999,
            kind: .keyDown,
            payload: .key(key: "ArrowRight", code: "ArrowRight", modifiers: 0x0D, keyCode: 39, isComposing: true)
        )
        try NveDirectEventWriter.withBatch([modified]) { batch in
            XCTAssertEqual(NveDirectEventWriter.decodeBatch(batch), [modified])
        }

        // 5% title-safe overscan on 1080p tvOS logical viewport (1920x1080) => 96pt H / 54pt V.
        let insets1080 = NveTvOverscanInsets.forViewport(size: CGSize(width: 1920, height: 1080))
        XCTAssertEqual(insets1080.leading, 96, accuracy: 1e-4)
        XCTAssertEqual(insets1080.trailing, 96, accuracy: 1e-4)
        XCTAssertEqual(insets1080.top, 54, accuracy: 1e-4)
        XCTAssertEqual(insets1080.bottom, 54, accuracy: 1e-4)

        let invalid = NveTvOverscanInsets.forViewport(size: CGSize(width: CGFloat.nan, height: -20), ratio: .infinity)
        XCTAssertEqual(invalid, NveTvOverscanInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
    }

    func testWatchOsDigitalCrownDeltaConversion() {
        XCTAssertEqual(
            NveWatchCrownScrollMapper.scrollDeltaPoints(from: 0.0, to: 1.5),
            36.0,
            accuracy: 1e-4
        )
        XCTAssertEqual(
            NveWatchCrownScrollMapper.scrollDeltaPoints(from: 4.0, to: 2.0, pointsPerDetent: 16.0),
            -32.0,
            accuracy: 1e-4
        )
        XCTAssertEqual(
            NveWatchCrownScrollMapper.scrollDeltaPoints(from: 1.0, to: 1.0),
            0.0,
            accuracy: 1e-4
        )
        XCTAssertEqual(
            NveWatchCrownScrollMapper.scrollDeltaPoints(from: .nan, to: 1.0),
            0.0,
            accuracy: 1e-4
        )
        XCTAssertEqual(
            NveWatchCrownScrollMapper.scrollDeltaPoints(from: 0.0, to: .infinity),
            0.0,
            accuracy: 1e-4
        )
        XCTAssertEqual(
            NveWatchCrownScrollMapper.scrollDeltaPoints(from: 0.0, to: 2.0, pointsPerDetent: -10.0),
            0.0,
            accuracy: 1e-4
        )
        XCTAssertGreaterThan(NveWatchCrownScrollMapper.watchDeviceScreenScale, 0)
    }

    func testWebViewUnavailablePlaceholderBuildsCleanly() {
        let state = NveSwiftUIWebViewState()
        state.apply(NveWebViewProps(url: "https://example.com", allowsBackForward: true))
        let placeholder = NveWebViewUnavailablePlaceholder(state: state)
        _ = placeholder.body
        XCTAssertEqual(state.lastRequestedUrl, "https://example.com")
    }
}
