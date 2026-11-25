#include "test_common.hpp"

int main() {
  nve_test::test_all_opcodes_and_custom_components();
  nve_test::test_corrupt_buffer_error_codes();
  nve_test::test_flexbox_layout();
  nve_test::test_standalone_flexbox_parity();
  nve_test::test_rn_component_bridge();
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  nve_test::test_mock_jsi_recorder_runtime();
#endif
  nve_test::test_bump_arena_and_bin_value_fuzz();
  nve_test::test_text_style_inheritance();
  nve_test::test_text_measurer_callback();
  nve_test::test_seeded_fuzz_corpus();
  nve_test::test_host_runtime_utilities_and_dylib_errors();
  nve_test::test_retained_view_reconciler();
  nve_test::test_native_host_session();
  nve_test::test_mmap_snapshot_and_session_instant_boot();
  nve_test::test_direct_scene_view_and_reconciler();

  std::cout << "nve_fabric_bridge_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI" : "Standalone")
            << " mode)\n";
  return 0;
}
