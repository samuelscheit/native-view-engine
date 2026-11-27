cmake_minimum_required(VERSION 3.24)
project(nve_gtk_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_ENABLE_NATIVE_VIEWS "Enable packages/modules/native-views" OFF)

find_package(PkgConfig REQUIRED)
pkg_check_modules(GTK4 REQUIRED IMPORTED_TARGET gtk4)

# `nve::host_cxx` + `nve::runtime` (libnve_runtime) + `nve::canvas_cxx` + app-module fixtures.
include("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/cmake/NveHostStack.cmake")

set(_nve_gtk_warnings -Wall -Wextra -Werror)

add_library(nve_gtk_shell_lib STATIC
  NveGtkHost.cpp
  NveGtkMedia.cpp
  NveGtkStyleA11y.cpp
  NveGtkReconciler.cpp
  NveGtkWindowManager.cpp
)
target_include_directories(nve_gtk_shell_lib PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(nve_gtk_shell_lib PRIVATE ${_nve_gtk_warnings})
target_link_libraries(nve_gtk_shell_lib PUBLIC nve::host_cxx nve::runtime nve::canvas_cxx PkgConfig::GTK4)

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
target_compile_options(nve_gtk_shell PRIVATE ${_nve_gtk_warnings})
target_link_libraries(nve_gtk_shell PRIVATE nve_gtk_shell_lib)

enable_testing()

nve_add_app_module_fixtures(nve_gtk_fixture)
add_executable(nve_gtk_shell_test
  tests/gtk_shell_test.cpp
  tests/gtk_shell_a11y_media_test.cpp
  tests/gtk_shell_canvas_pair_test.cpp
  tests/gtk_window_manager_test.cpp
)
target_compile_options(nve_gtk_shell_test PRIVATE ${_nve_gtk_warnings})
nve_use_app_module_fixtures(nve_gtk_shell_test nve_gtk_fixture)
target_link_libraries(nve_gtk_shell_test PRIVATE nve_gtk_shell_lib nve::host_test_support)
add_test(NAME gtk_shell_test COMMAND nve_gtk_shell_test)
nve_host_sanitize(nve_gtk_shell_lib nve_gtk_shell nve_gtk_shell_test)
