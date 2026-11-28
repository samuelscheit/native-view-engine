cmake_minimum_required(VERSION 3.24)
project(nve_macos_shell LANGUAGES C CXX OBJCXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# `nve::host_cxx` + `nve::runtime` + `nve::canvas_cxx` (libnve_runtime staticlib + generated ABI /
# table headers + flat `nve.canvas` draw-list codec).
include("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/cmake/NveHostStack.cmake")

set(NVE_APPLE_COMMON_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../apple-common")

add_library(nve_macos_shell_lib STATIC
  "${NVE_APPLE_COMMON_DIR}/NveAppleImageCache.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleValueBridge.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleSupport.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleCanvasView.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleSceneNodes.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleSceneReconciler.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleComponentHost.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleComponentHost+Session.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleComponentViewBase.mm"
  NveMacWindowManager.mm
  RCTNveMacComponentView.mm
)
target_include_directories(nve_macos_shell_lib PUBLIC ${CMAKE_CURRENT_SOURCE_DIR} ${NVE_APPLE_COMMON_DIR})
target_compile_options(nve_macos_shell_lib PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_macos_shell_lib PUBLIC
  nve::runtime
  nve::canvas_cxx
  "-framework Cocoa"
  "-framework QuartzCore"
  "-framework CoreText"
)
# Keep Objective-C categories (e.g. `NveAppleComponentHost (Session)`) of this static library.
target_link_options(nve_macos_shell_lib INTERFACE -ObjC)

add_executable(nve_macos_shell main.mm)
target_compile_options(nve_macos_shell PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_macos_shell PRIVATE nve_macos_shell_lib)

enable_testing()

foreach(_fixture NO_ENTRY NO_SURFACE STATEFUL CREATE_FAIL)
  string(TOLOWER "${_fixture}" _name)
  add_library(nve_test_dylib_${_name} SHARED tests/test_dylib_fixtures.cpp)
  target_compile_definitions(nve_test_dylib_${_name} PRIVATE NVE_FIXTURE_${_fixture}=1)
  target_include_directories(nve_test_dylib_${_name} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../packages/core/host-cxx/include" ${NVE_APPLE_COMMON_DIR})
  target_compile_options(nve_test_dylib_${_name} PRIVATE -Wall -Wextra -Werror)
  add_dependencies(nve_test_dylib_${_name} nve_runtime)  # generated headers
endforeach()

add_executable(nve_macos_shell_test tests/macos_shell_test.mm)
target_compile_options(nve_macos_shell_test PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_compile_definitions(nve_macos_shell_test PRIVATE
  NVE_TEST_NO_ENTRY_DYLIB="$<TARGET_FILE:nve_test_dylib_no_entry>"
  NVE_TEST_NO_SURFACE_DYLIB="$<TARGET_FILE:nve_test_dylib_no_surface>"
  NVE_TEST_STATEFUL_DYLIB="$<TARGET_FILE:nve_test_dylib_stateful>"
  NVE_TEST_CREATE_FAIL_DYLIB="$<TARGET_FILE:nve_test_dylib_create_fail>"
)
target_link_libraries(nve_macos_shell_test PRIVATE nve_macos_shell_lib)
add_dependencies(nve_macos_shell_test
  nve_test_dylib_no_entry nve_test_dylib_no_surface nve_test_dylib_stateful nve_test_dylib_create_fail)
add_test(NAME macos_shell_test COMMAND nve_macos_shell_test)

add_executable(nve_macos_parity_test tests/macos_parity_test.mm)
target_compile_options(nve_macos_parity_test PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_macos_parity_test PRIVATE nve_macos_shell_lib)
add_test(NAME macos_parity_test COMMAND nve_macos_parity_test)

add_executable(nve_macos_window_manager_test tests/window_manager_test.mm)
target_compile_options(nve_macos_window_manager_test PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_macos_window_manager_test PRIVATE nve_macos_shell_lib)
add_test(NAME macos_window_manager_test COMMAND nve_macos_window_manager_test)
