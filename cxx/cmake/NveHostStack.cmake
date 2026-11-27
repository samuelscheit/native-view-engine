# The native C++ host stack every desktop / ObjC++ shell links:
#   nve::host_cxx    header-only scene readers, `HostSession`, `RetainedTree`, `Nve2Writer`
#   nve::runtime     `libnve_runtime` (`nve_runtime_get_interface`)
#   nve::canvas_cxx  `nve.canvas` table binding + `NVE_DRAW1` codec
# plus, for host test suites, `nve::host_test_support` (`nve/host/testing/scripted_app.hpp`) and
# `nve_add_app_module_fixtures(prefix)` / `nve_use_app_module_fixtures(target prefix)`.
#
#   include("${NVE_ROOT}/cxx/cmake/NveHostStack.cmake")
#   target_link_libraries(my_host PRIVATE nve::host_cxx nve::runtime nve::canvas_cxx)

include_guard(GLOBAL)

get_filename_component(_nve_stack_root "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)

include("${CMAKE_CURRENT_LIST_DIR}/NveRuntime.cmake")
include("${CMAKE_CURRENT_LIST_DIR}/NveHostFixtures.cmake")

if(NOT TARGET nve_host_cxx)
  add_subdirectory("${_nve_stack_root}/packages/core/host-cxx" "${CMAKE_BINARY_DIR}/nve_host_cxx")
endif()
if(NOT TARGET nve_host_test_support)
  add_library(nve_host_test_support INTERFACE)
  add_library(nve::host_test_support ALIAS nve_host_test_support)
  target_include_directories(nve_host_test_support INTERFACE "${_nve_stack_root}/packages/core/host-cxx/tests/support")
  target_link_libraries(nve_host_test_support INTERFACE nve_host_cxx)
endif()
if(NOT TARGET nve_canvas_cxx)
  add_subdirectory("${_nve_stack_root}/packages/modules/canvas/cxx" "${CMAKE_BINARY_DIR}/nve_canvas_cxx")
endif()
