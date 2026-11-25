cmake_minimum_required(VERSION 3.24)
project(nve_gtk_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_ENABLE_NATIVE_VIEWS "Enable packages/modules/native-views" OFF)

find_package(PkgConfig REQUIRED)
pkg_check_modules(GTK4 REQUIRED IMPORTED_TARGET gtk4)

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge"
    "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge"
  )
endif()

add_library(nve_gtk_shell_lib STATIC
  NveGtkHost.cpp
  NveGtkMedia.cpp
  NveGtkStyleA11y.cpp
  NveGtkReconciler.cpp
  NveGtkWindowManager.cpp
)

target_include_directories(nve_gtk_shell_lib
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_compile_options(nve_gtk_shell_lib PRIVATE -Wall -Wextra -Werror)
target_link_libraries(nve_gtk_shell_lib
  PUBLIC
    nve_fabric_bridge
    PkgConfig::GTK4
)

if(NVE_ENABLE_NATIVE_VIEWS)
  if(NOT TARGET nve_gtk_native_views)
    add_subdirectory(
      "${CMAKE_CURRENT_SOURCE_DIR}/../../packages/modules/native-views/gtk"
      "${CMAKE_CURRENT_BINARY_DIR}/nve_gtk_native_views"
    )
  endif()
  target_link_libraries(nve_gtk_shell_lib PUBLIC nve_gtk_native_views)
  target_compile_definitions(nve_gtk_shell_lib PUBLIC NVE_ENABLE_NATIVE_VIEWS=1)
endif()

add_executable(nve_gtk_shell main.cpp)
target_compile_options(nve_gtk_shell PRIVATE -Wall -Wextra -Werror)
target_link_libraries(nve_gtk_shell PRIVATE nve_gtk_shell_lib)

enable_testing()

add_library(nve_gtk_test_dylib_no_entry SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_gtk_test_dylib_no_entry PRIVATE
  NVE_BUILD_SHARED_LIB=1
  NVE_FIXTURE_NO_ENTRY=1
)
target_include_directories(nve_gtk_test_dylib_no_entry PRIVATE
  "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include"
)

add_library(nve_gtk_test_dylib_no_surface SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_gtk_test_dylib_no_surface PRIVATE
  NVE_BUILD_SHARED_LIB=1
  NVE_FIXTURE_NO_SURFACE=1
)
target_include_directories(nve_gtk_test_dylib_no_surface PRIVATE
  "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include"
)

add_library(nve_gtk_test_dylib_stateful SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_gtk_test_dylib_stateful PRIVATE
  NVE_BUILD_SHARED_LIB=1
  NVE_FIXTURE_STATEFUL=1
)
target_include_directories(nve_gtk_test_dylib_stateful PRIVATE
  "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include"
)

add_executable(nve_gtk_shell_test
  tests/gtk_shell_test.cpp
  tests/gtk_window_manager_test.cpp
)
target_compile_options(nve_gtk_shell_test PRIVATE -Wall -Wextra -Werror)
target_include_directories(nve_gtk_shell_test PRIVATE
  "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/src"
)
target_compile_definitions(nve_gtk_shell_test PRIVATE
  NVE_TEST_NO_ENTRY_DYLIB="$<TARGET_FILE:nve_gtk_test_dylib_no_entry>"
  NVE_TEST_NO_SURFACE_DYLIB="$<TARGET_FILE:nve_gtk_test_dylib_no_surface>"
  NVE_TEST_STATEFUL_DYLIB="$<TARGET_FILE:nve_gtk_test_dylib_stateful>"
)
target_link_libraries(nve_gtk_shell_test PRIVATE nve_gtk_shell_lib)
add_dependencies(nve_gtk_shell_test
  nve_gtk_test_dylib_no_entry
  nve_gtk_test_dylib_no_surface
  nve_gtk_test_dylib_stateful
)

add_test(NAME gtk_shell_test COMMAND nve_gtk_shell_test)
