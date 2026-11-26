import CNveAbi
import NveSwiftUI
import XCTest

private enum AbiTestFixtures {
    static let surfaceApi: UnsafePointer<NveSurfaceApi> = {
        let ptr = UnsafeMutablePointer<NveSurfaceApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveSurfaceApi(
                create: { sid in UnsafeMutableRawPointer(bitPattern: Int(sid) + 0x1000) },
                destroy: { _ in },
                render: { _, _ in NveBufferSlice(ptr: nil, len: 0) },
                dispatch_events: { _, batch in batch.len > 0 ? 1 : 0 }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let layoutApi: UnsafePointer<NveLayoutApi> = {
        let ptr = UnsafeMutablePointer<NveLayoutApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveLayoutApi(
                measure: { _, constraints in
                    NveIntrinsicSize(width: constraints.min_w + 10, height: constraints.min_h + 20)
                },
                hit_test: { _, _, _, _ in UInt8(NVE_GESTURE_CLAIM.rawValue) }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let componentApi: UnsafePointer<NveComponentApi> = {
        let ptr = UnsafeMutablePointer<NveComponentApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveComponentApi(
                set_props: { _, _, count in count > 0 ? 1 : 0 },
                drain_emitted_events: { _ in NveEventBatch(ptr: nil, len: 0, _pad: 0) }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let devApi: UnsafePointer<NveDevApi> = {
        let ptr = UnsafeMutablePointer<NveDevApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveDevApi(
                apply_hot_patch: { _, _ in 1 },
                snapshot_state: { _ in NveBufferSlice(ptr: nil, len: 0) },
                restore_state: { _, _ in }
            )
        )
        return UnsafePointer(ptr)
    }()
}

private func fullGetInterface(_ interfaceId: UInt32) -> UnsafeRawPointer? {
    switch interfaceId {
    case UInt32(NVE_IFACE_SURFACE):
        return UnsafeRawPointer(AbiTestFixtures.surfaceApi)
    case UInt32(NVE_IFACE_LAYOUT):
        return UnsafeRawPointer(AbiTestFixtures.layoutApi)
    case UInt32(NVE_IFACE_COMPONENT):
        return UnsafeRawPointer(AbiTestFixtures.componentApi)
    case UInt32(NVE_IFACE_DEV):
        return UnsafeRawPointer(AbiTestFixtures.devApi)
    default:
        return nil
    }
}

private func surfaceOnlyGetInterface(_ interfaceId: UInt32) -> UnsafeRawPointer? {
    if interfaceId == UInt32(NVE_IFACE_SURFACE) {
        return UnsafeRawPointer(AbiTestFixtures.surfaceApi)
    }
    return nil
}

private func emptyGetInterface(_ interfaceId: UInt32) -> UnsafeRawPointer? {
    nil
}

final class CNveAbiTests: XCTestCase {
    func testAbiConstantsAndMagic() {
        XCTAssertEqual(UInt32(NVE_ABI_MAGIC), 0x3145_564E)
        XCTAssertEqual(UInt32(NVE_ABI_VERSION), 1)
        XCTAssertEqual(UInt32(NVE_ROOT_NODE_ID), 1)

        XCTAssertEqual(UInt32(NVE_IFACE_SURFACE), 1)
        XCTAssertEqual(UInt32(NVE_IFACE_LAYOUT), 2)
        XCTAssertEqual(UInt32(NVE_IFACE_COMPONENT), 3)
        XCTAssertEqual(UInt32(NVE_IFACE_DEV), 4)
    }

    func testGestureDispositionMatchesSwiftOverlay() {
        XCTAssertEqual(UInt8(NVE_GESTURE_CLAIM.rawValue), NveSwiftUI.NveGestureDisposition.claim.rawValue)
        XCTAssertEqual(
            UInt8(NVE_GESTURE_PASS_THROUGH.rawValue),
            NveSwiftUI.NveGestureDisposition.passThrough.rawValue
        )
        XCTAssertEqual(
            UInt8(NVE_GESTURE_DEFER_TO_PARENT.rawValue),
            NveSwiftUI.NveGestureDisposition.deferToParent.rawValue
        )
    }

    func testResolveModuleWithNilResolverReturnsEmptyModule() {
        let module = nve_resolve_module(nil)
        XCTAssertNil(module.surface)
        XCTAssertNil(module.layout)
        XCTAssertNil(module.component)
        XCTAssertNil(module.dev)
    }

    func testResolveModuleWithAllInterfaces() {
        let module = nve_resolve_module(fullGetInterface)
        XCTAssertEqual(module.surface, AbiTestFixtures.surfaceApi)
        XCTAssertEqual(module.layout, AbiTestFixtures.layoutApi)
        XCTAssertEqual(module.component, AbiTestFixtures.componentApi)
        XCTAssertEqual(module.dev, AbiTestFixtures.devApi)

        let handle = module.surface?.pointee.create?(42)
        XCTAssertEqual(handle, UnsafeMutableRawPointer(bitPattern: 42 + 0x1000))

        let constraints = NveSizeConstraints(min_w: 10, min_h: 20, max_w: 300, max_h: 400)
        let size = module.layout?.pointee.measure?(handle, constraints)
        XCTAssertEqual(size?.width, 20)
        XCTAssertEqual(size?.height, 40)

        let disposition = module.layout?.pointee.hit_test?(handle, 5, 5, 1)
        XCTAssertEqual(disposition, UInt8(NVE_GESTURE_CLAIM.rawValue))
    }

    func testResolveModuleWithMissingOptionalInterfaces() {
        let module = nve_resolve_module(surfaceOnlyGetInterface)
        XCTAssertEqual(module.surface, AbiTestFixtures.surfaceApi)
        XCTAssertNil(module.layout)
        XCTAssertNil(module.component)
        XCTAssertNil(module.dev)
    }

    func testResolveModuleWhenResolverReturnsNilForAllIds() {
        let module = nve_resolve_module(emptyGetInterface)
        XCTAssertNil(module.surface)
        XCTAssertNil(module.layout)
        XCTAssertNil(module.component)
        XCTAssertNil(module.dev)
    }

    func testAbiStructFieldLayout() {
        let cfg = NveSurfaceConfig(surface_id: 9, width: 390, height: 844, scale_factor: 3)
        XCTAssertEqual(cfg.surface_id, 9)
        XCTAssertEqual(cfg.width, 390)
        XCTAssertEqual(cfg.height, 844)
        XCTAssertEqual(cfg.scale_factor, 3)

        let stats = NveFrameStats(
            op_count: 12,
            mutation_count: 8,
            decode_us: 10,
            layout_us: 20,
            diff_us: 30,
            mount_us: 40
        )
        XCTAssertEqual(stats.op_count, 12)
        XCTAssertEqual(stats.mutation_count, 8)
        XCTAssertEqual(stats.mount_us, 40)

        #if arch(x86_64) || arch(arm64)
        XCTAssertEqual(MemoryLayout<NveStrSlice>.size, 16)
        XCTAssertEqual(MemoryLayout<NveValue>.size, 24)
        XCTAssertEqual(MemoryLayout<NveMapEntry>.size, 40)
        XCTAssertEqual(MemoryLayout<NveDirectEvent>.size, 96)
        XCTAssertEqual(MemoryLayout<NveEventBatch>.size, 16)
        #endif
    }
}
