# The native C++ host stack every desktop / ObjC++ shell links:
#   nve::host_cxx    header-only scene readers, `HostSession`, `RetainedTree`, `Nve2Writer`
#   nve::runtime     `libnve_runtime` (`nve_runtime_get_interface`)
#   nve::canvas_cxx  `nve.canvas` table binding + `NVE_DRAW1` codec
# plus, for host test suites, `nve::host_test_support` (`nve/host/testing/scripted_app.hpp`),
# `nve_add_app_module_fixtures(prefix)`, and `nve_host_sanitize(<targets>)` (ASan + UBSan when
# `NVE_HOST_SANITIZE` is on: by default for Debug / untyped builds with a supporting toolchain).
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
if(CMAKE_BUILD_TYPE STREQUAL "" OR CMAKE_BUILD_TYPE STREQUAL "Debug")
  set(_nve_sanitize_default ON)
else()
  set(_nve_sanitize_default OFF)
endif()
option(NVE_HOST_SANITIZE "Build host shells and their tests with ASan + UBSan" ${_nve_sanitize_default})

function(nve_host_sanitize)
  if(NOT NVE_HOST_SANITIZE OR MSVC)
    return()
  endif()
  if(NOT DEFINED NVE_HOST_HAS_SANITIZERS)
    include(CheckCXXSourceCompiles)
    set(CMAKE_REQUIRED_FLAGS "-fsanitize=address,undefined")
    set(CMAKE_REQUIRED_LINK_OPTIONS "-fsanitize=address,undefined")
    check_cxx_source_compiles("int main() { return 0; }" NVE_HOST_HAS_SANITIZERS)
  endif()
  if(NOT NVE_HOST_HAS_SANITIZERS)
    return()
  endif()
  foreach(_target IN LISTS ARGN)
    target_compile_options(${_target} PRIVATE
      -fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer)
    target_link_options(${_target} PRIVATE -fsanitize=address,undefined)
  endforeach()
endfunction()

if(NOT TARGET nve_canvas_cxx)
  add_subdirectory("${_nve_stack_root}/packages/modules/canvas/cxx" "${CMAKE_BINARY_DIR}/nve_canvas_cxx")
endif()
