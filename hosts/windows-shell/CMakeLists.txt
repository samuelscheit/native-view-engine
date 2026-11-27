cmake_minimum_required(VERSION 3.24)
project(nve_windows_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_WITH_RNW_FABRIC "Build the react-native-windows 0.83+ Composition Fabric component (nve_rnw_component)" OFF)
option(NVE_ENABLE_NATIVE_VIEWS "Enable packages/modules/native-views" OFF)
set(RNW_GENERATED_DIR "" CACHE PATH "react-native-windows C++/WinRT generated headers (winrt/Microsoft.ReactNative.Composition.h)")
set(NVE_RNW_EXTRA_INCLUDE_DIRS "" CACHE STRING "Extra include directories for react-native-windows / Windows App SDK")
set(NVE_RNW_LIBS "" CACHE STRING "Libraries to link when NVE_WITH_RNW_FABRIC=ON")

# `nve::host_cxx` + `nve::runtime` (libnve_runtime) + `nve::canvas_cxx` + app-module fixtures.
include("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/cmake/NveHostStack.cmake")

if(MSVC)
  set(_nve_win_warnings /W4 /Zc:__cplusplus /utf-8 /permissive-)
else()
  set(_nve_win_warnings -Wall -Wextra -Werror)
endif()

# Portable core (retained model + runtime controller + window manager): every OS builds and tests it.
add_library(nve_windows_shell_lib STATIC
  NveWinVisualTree.cpp
  NveWinHost.cpp
  NveWinWindowManager.cpp
)
if(WIN32)
  # Win32 window, Direct2D / DirectWrite / WIC painting, native EDIT overlays.
  target_sources(nve_windows_shell_lib PRIVATE
    NveWinHostWindow.cpp
    NveWinTextInputs.cpp
    NveWinD2DRenderer.cpp
    NveWinD2DMedia.cpp
  )
  target_compile_definitions(nve_windows_shell_lib PUBLIC NOMINMAX WIN32_LEAN_AND_MEAN UNICODE _UNICODE)
  target_link_libraries(nve_windows_shell_lib PUBLIC d2d1 dwrite windowscodecs ole32 user32 gdi32 comctl32 shell32
                                                     shlwapi crypt32)
endif()
target_include_directories(nve_windows_shell_lib PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(nve_windows_shell_lib PRIVATE ${_nve_win_warnings})
target_link_libraries(nve_windows_shell_lib PUBLIC nve::host_cxx nve::runtime nve::canvas_cxx)

if(NVE_ENABLE_NATIVE_VIEWS)
  if(NOT TARGET nve_win_native_views)
    add_subdirectory(
      "${CMAKE_CURRENT_SOURCE_DIR}/../../packages/modules/native-views/windows"
      "${CMAKE_CURRENT_BINARY_DIR}/nve_win_native_views"
    )
  endif()
  target_link_libraries(nve_windows_shell_lib PUBLIC nve_win_native_views)
  target_compile_definitions(nve_windows_shell_lib PUBLIC NVE_ENABLE_NATIVE_VIEWS=1)
endif()

# react-native-windows Fabric component: an optional layer ON TOP of the host (the host never
# depends on it).
if(NVE_WITH_RNW_FABRIC)
  if(NOT WIN32)
    message(FATAL_ERROR "NVE_WITH_RNW_FABRIC=ON is only supported on Windows")
  endif()
  add_library(nve_rnw_component STATIC NveRnwComponentView.cpp)
  target_compile_options(nve_rnw_component PRIVATE ${_nve_win_warnings})
  target_link_libraries(nve_rnw_component PUBLIC nve_windows_shell_lib windowsapp ${NVE_RNW_LIBS})
  if(RNW_GENERATED_DIR)
    target_include_directories(nve_rnw_component PUBLIC "${RNW_GENERATED_DIR}")
  endif()
  if(NVE_RNW_EXTRA_INCLUDE_DIRS)
    target_include_directories(nve_rnw_component PUBLIC ${NVE_RNW_EXTRA_INCLUDE_DIRS})
  endif()
endif()

if(WIN32)
  add_executable(nve_windows_shell main.cpp)
  target_compile_options(nve_windows_shell PRIVATE ${_nve_win_warnings})
  target_link_libraries(nve_windows_shell PRIVATE nve_windows_shell_lib)
endif()

enable_testing()

nve_add_app_module_fixtures(nve_win_fixture)
add_executable(nve_windows_shell_test
  tests/windows_shell_test.cpp
  tests/windows_shell_media_test.cpp
  tests/windows_shell_canvas_scroll_test.cpp
)
if(WIN32)
  target_sources(nve_windows_shell_test PRIVATE tests/windows_shell_d2d_test.cpp)
endif()
target_compile_options(nve_windows_shell_test PRIVATE ${_nve_win_warnings})
target_compile_definitions(nve_windows_shell_test PRIVATE
  NVE_TEST_COUNTER_MODULE="$<TARGET_FILE:nve_win_fixture_counter>"
  NVE_TEST_NO_SURFACE_MODULE="$<TARGET_FILE:nve_win_fixture_no_surface>"
  NVE_TEST_NO_ENTRY_MODULE="$<TARGET_FILE:nve_win_fixture_no_entry>"
)
target_link_libraries(nve_windows_shell_test PRIVATE nve_windows_shell_lib nve::host_test_support)
add_dependencies(nve_windows_shell_test nve_win_fixture_counter nve_win_fixture_no_surface nve_win_fixture_no_entry)
add_test(NAME windows_shell_test COMMAND nve_windows_shell_test)
nve_host_sanitize(nve_windows_shell_lib nve_windows_shell_test)
if(TARGET nve_windows_shell)
  nve_host_sanitize(nve_windows_shell)
endif()
if(TARGET nve_rnw_component)
  nve_host_sanitize(nve_rnw_component)
endif()
