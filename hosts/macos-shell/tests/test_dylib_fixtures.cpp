// Fixture app dylibs for the macOS shell tests (one variant per compile definition).
#include <nve/host/nve_abi.generated.h>

#include <cstdint>

#if defined(NVE_FIXTURE_NO_ENTRY)

extern "C" int nve_fixture_dummy_symbol(void) { return 42; }

#elif defined(NVE_FIXTURE_NO_SURFACE)

extern "C" const void *nve_get_interface(uint32_t) { return nullptr; }

#elif defined(NVE_FIXTURE_STATEFUL)

#include "NveAppleDemoApp.hpp"

extern "C" const void *nve_get_interface(uint32_t id) { return nve::apple::DemoApp::get_interface(id); }

#elif defined(NVE_FIXTURE_CREATE_FAIL)

namespace {
NveAppHandle fail_create(uint32_t) { return nullptr; }
void fail_destroy(NveAppHandle) {}
NveBufferSlice fail_render(NveAppHandle, uint64_t) { return NveBufferSlice{nullptr, 0u}; }
uint32_t fail_dispatch(NveAppHandle, NveEventBatch) { return 0u; }
}  // namespace

extern "C" const void *nve_get_interface(uint32_t id) {
  static const NveSurfaceApi kSurface{&fail_create, &fail_destroy, &fail_render, &fail_dispatch};
  return id == NVE_IFACE_SURFACE ? &kSurface : nullptr;
}

#else
#error "define one NVE_FIXTURE_* variant"
#endif