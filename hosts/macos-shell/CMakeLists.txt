cmake_minimum_required(VERSION 3.24)
project(nve_macos_shell LANGUAGES C CXX OBJCXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge" "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge")
endif()

set(NVE_APPLE_COMMON_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../apple-common")

add_library(nve_macos_shell_lib STATIC
  "${NVE_APPLE_COMMON_DIR}/NveAppleCanvasView.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleImageCache.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleSupport.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleFabricHost.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleSceneReconciler.mm"
  "${NVE_APPLE_COMMON_DIR}/NveAppleComponentHost.mm"
  NveMacFabricHost.mm
  RCTNveMacComponentView.mm
)

target_include_directories(nve_macos_shell_lib
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${NVE_APPLE_COMMON_DIR}
)

target_compile_options(nve_macos_shell_lib PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_macos_shell_lib
  PUBLIC
    nve_fabric_bridge
    "-framework Cocoa"
    "-framework QuartzCore"
)

add_executable(nve_macos_shell main.mm)
target_compile_options(nve_macos_shell PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_macos_shell PRIVATE nve_macos_shell_lib)

enable_testing()

add_library(nve_test_dylib_no_entry SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_test_dylib_no_entry PRIVATE NVE_FIXTURE_NO_ENTRY=1)
target_include_directories(nve_test_dylib_no_entry PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include")

add_library(nve_test_dylib_no_surface SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_test_dylib_no_surface PRIVATE NVE_FIXTURE_NO_SURFACE=1)
target_include_directories(nve_test_dylib_no_surface PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include")

add_library(nve_test_dylib_stateful SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_test_dylib_stateful PRIVATE NVE_FIXTURE_STATEFUL=1)
target_include_directories(nve_test_dylib_stateful PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include")

add_library(nve_test_dylib_create_fail SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_test_dylib_create_fail PRIVATE NVE_FIXTURE_CREATE_FAIL=1)
target_include_directories(nve_test_dylib_create_fail PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include")

add_executable(nve_macos_shell_test tests/macos_shell_test.mm)
target_compile_options(nve_macos_shell_test PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_include_directories(nve_macos_shell_test PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/src")
target_compile_definitions(nve_macos_shell_test PRIVATE
  NVE_TEST_NO_ENTRY_DYLIB="$<TARGET_FILE:nve_test_dylib_no_entry>"
  NVE_TEST_NO_SURFACE_DYLIB="$<TARGET_FILE:nve_test_dylib_no_surface>"
  NVE_TEST_STATEFUL_DYLIB="$<TARGET_FILE:nve_test_dylib_stateful>"
  NVE_TEST_CREATE_FAIL_DYLIB="$<TARGET_FILE:nve_test_dylib_create_fail>"
)
target_link_libraries(nve_macos_shell_test PRIVATE nve_macos_shell_lib)
add_dependencies(nve_macos_shell_test
  nve_test_dylib_no_entry
  nve_test_dylib_no_surface
  nve_test_dylib_stateful
  nve_test_dylib_create_fail
)
add_test(NAME macos_shell_test COMMAND nve_macos_shell_test)
