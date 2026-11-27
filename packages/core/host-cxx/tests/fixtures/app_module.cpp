// App `cdylib` fixtures for `AppModule` / `HostSession` tests (built by `NveHostFixtures.cmake`):
//   NVE_FIXTURE_COUNTER     exports the built-in counter demo (`nve::host::demo::CounterApp`)
//   NVE_FIXTURE_NO_SURFACE  exports `nve_get_interface` without `NVE_IFACE_SURFACE`
//   NVE_FIXTURE_NO_ENTRY    exports no `nve_get_interface` at all
#include "nve/host/demo_counter_app.hpp"

#if defined(NVE_FIXTURE_COUNTER)
extern "C" NVE_API const void* nve_get_interface(uint32_t interface_id) {
  return nve::host::demo::CounterApp::get_interface(interface_id);
}
#elif defined(NVE_FIXTURE_NO_SURFACE)
extern "C" NVE_API const void* nve_get_interface(uint32_t) { return nullptr; }
#elif defined(NVE_FIXTURE_NO_ENTRY)
extern "C" NVE_API int nve_fixture_without_entry(void) { return 42; }
#else
#error "define one NVE_FIXTURE_* variant"
#endif
