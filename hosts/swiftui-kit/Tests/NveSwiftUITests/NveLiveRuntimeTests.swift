import CNveAbi
import Darwin
import NveHost
@testable import NveSwiftUI
import XCTest

/// The real `libnve_runtime` + `libnve_host_fixture_app` across the C ABI (no mock): the runtime
/// loads, hot-reloads, and rejects app modules itself (`load_app_module`), and props round-trip
/// through the app as an emitted `Custom` event. Libraries come from `NVE_RUNTIME_LIB` /
/// `NVE_HOST_FIXTURE_APP_LIB` (`scripts/check-suites.sh` builds and exports them).
final class NveLiveRuntimeTests: XCTestCase {
    private static func library(_ env: String) throws -> String {
        guard let path = ProcessInfo.processInfo.environment[env], !path.isEmpty else {
            throw XCTSkip("\(env) unset: `cargo build -p nve-runtime -p nve-host-fixtures` and export it")
        }
        return path
    }

    /// The runtime's C API, `dlopen`ed once per process (the runtime is the host's library, unlike apps).
    private static func runtimeApi() throws -> UnsafePointer<NveRuntimeApi> {
        let path = try library("NVE_RUNTIME_LIB")
        guard let lib = dlopen(path, RTLD_NOW | RTLD_LOCAL), let sym = dlsym(lib, "nve_runtime_get_interface") else {
            throw XCTSkip("cannot load \(path)")
        }
        typealias GetInterface = @convention(c) (UInt32) -> UnsafePointer<NveRuntimeApi>?
        return try XCTUnwrap(unsafeBitCast(sym, to: GetInterface.self)(NVE_RUNTIME_ABI_VERSION))
    }

    func testTheRuntimeLoadsReloadsAndRejectsAppModulesAndPropsRoundTrip() throws {
        let api = try Self.runtimeApi()
        let app = try Self.library("NVE_HOST_FIXTURE_APP_LIB")
        XCTAssertTrue(NveRuntimeSession.isCompatible(api), "generated header and runtime agree")

        let controller = try NveSurfaceController(surfaceId: 7, dylibPath: app, runtimeApi: api, textMeasurer: nil)
        XCTAssertEqual(controller.resolutionSource, .dynamicLibrary)
        XCTAssertTrue(controller.isMounted)
        XCTAssertGreaterThan(controller.store.frameId, 0, "the loaded app published a scene")

        var received: [NveSwiftUI.NveEvent] = []
        controller.onEvent = { received.append($0) }
        XCTAssertTrue(controller.setProps(["count": 3, "label": "hi"]))
        XCTAssertEqual(received.map(\.kind), [.custom], "the app echoes new props as one custom event")
        XCTAssertEqual(received.first?.nodeId, 1)
        XCTAssertFalse(controller.setProps(["label": "hi", "count": 3]), "canonical props dedupe")

        // Hot reload of the same file: the runtime stages a fresh copy and carries the state.
        try controller.reload(dylibPath: app)
        XCTAssertEqual(controller.resolutionSource, .dynamicLibrary)
        received.removeAll()
        XCTAssertTrue(controller.setProps(["count": 4]), "props are resent to the reloaded app")
        XCTAssertEqual(received.map(\.kind), [.custom], "the reloaded module runs")

        // A missing module is refused by the runtime and the running app keeps going.
        XCTAssertThrowsError(try controller.reload(dylibPath: "/nonexistent/libnve_missing_app.dylib"))
        XCTAssertTrue(controller.isMounted)
        received.removeAll()
        XCTAssertTrue(controller.setProps(["count": 5]))
        XCTAssertEqual(received.count, 1, "the previous module still answers")

        // The store mirrors the live scene's latest frame.
        XCTAssertEqual(try controller.session.withScene { $0.frameId }, controller.store.frameId)
        try controller.detach()
        XCTAssertFalse(controller.isMounted)
    }
}
