// App `cdylib` fixtures for `HostSession` / runtime `load_app_module` tests (built by
// `NveHostFixtures.cmake`):
//   NVE_FIXTURE_COUNTER          exports the built-in counter demo (`nve::host::demo::CounterApp`)
//   NVE_FIXTURE_COUNTER_DOUBLE   the "rebuilt" counter: same state format, but every event batch is
//                                applied twice (a press counts 2), so a hot reload that copies it
//                                over the counter's path proves the NEW code runs
//   NVE_FIXTURE_PARTIAL_SURFACE  exports `NVE_IFACE_SURFACE` with a NULL `dispatch_events` slot
//   NVE_FIXTURE_NO_SURFACE       exports `nve_get_interface` without `NVE_IFACE_SURFACE`
//   NVE_FIXTURE_NO_ENTRY         exports no `nve_get_interface` at all
#include "nve/host/demo_counter_app.hpp"

using nve::host::demo::CounterApp;

#if defined(NVE_FIXTURE_COUNTER)
extern "C" NVE_API const void* nve_get_interface(uint32_t interface_id) {
  return CounterApp::get_interface(interface_id);
}
#elif defined(NVE_FIXTURE_COUNTER_DOUBLE) || defined(NVE_FIXTURE_PARTIAL_SURFACE)
namespace {
const NveSurfaceApi& counter_surface() {
  return *static_cast<const NveSurfaceApi*>(CounterApp::get_interface(NVE_IFACE_SURFACE));
}
#if defined(NVE_FIXTURE_COUNTER_DOUBLE)
uint32_t dispatch_twice(NveAppHandle h, NveEventBatch batch) {
  const uint32_t first = counter_surface().dispatch_events(h, batch);
  return counter_surface().dispatch_events(h, batch) | first;
}
constexpr decltype(NveSurfaceApi::dispatch_events) kDispatch = &dispatch_twice;
#else
constexpr decltype(NveSurfaceApi::dispatch_events) kDispatch = nullptr;  // the runtime must refuse it
#endif
}  // namespace

extern "C" NVE_API const void* nve_get_interface(uint32_t interface_id) {
  if (interface_id != NVE_IFACE_SURFACE) return CounterApp::get_interface(interface_id);
  static const NveSurfaceApi kSurface = [] {
    NveSurfaceApi s = counter_surface();
    s.dispatch_events = kDispatch;
    return s;
  }();
  return &kSurface;
}
#elif defined(NVE_FIXTURE_NO_SURFACE)
extern "C" NVE_API const void* nve_get_interface(uint32_t) { return nullptr; }
#elif defined(NVE_FIXTURE_NO_ENTRY)
extern "C" NVE_API int nve_fixture_without_entry(void) { return 42; }
#else
#error "define one NVE_FIXTURE_* variant"
#endif
