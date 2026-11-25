cmake_minimum_required(VERSION 3.24)
project(nve_windows_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_WITH_RNW_FABRIC "Enable react-native-windows 0.83+/0.88+ Composition Fabric integration" OFF)
option(NVE_ENABLE_NATIVE_VIEWS "Enable packages/modules/native-views" OFF)
set(RNW_GENERATED_DIR "" CACHE PATH "Path to react-native-windows C++/WinRT generated headers (containing winrt/Microsoft.ReactNative.Composition.h)")
set(NVE_RNW_EXTRA_INCLUDE_DIRS "" CACHE STRING "Optional extra include directories for react-native-windows / Windows App SDK")
set(NVE_RNW_LIBS "" CACHE STRING "Optional libraries to link when NVE_WITH_RNW_FABRIC=ON")

include("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/cmake/NveCodegen.cmake")
nve_ensure_abi_codegen()

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge" "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge")
endif()

set(NVE_WIN_SHELL_SOURCES
  NveRnwComponentView.cpp
  NveWinWindowManager.cpp
)
if(WIN32)
  list(APPEND NVE_WIN_SHELL_SOURCES
    NveWinHostWindow.cpp
    NveWinD2DRenderer.cpp
    NveWinD2DMedia.cpp
  )
endif()

add_library(nve_windows_shell_lib STATIC ${NVE_WIN_SHELL_SOURCES})

target_include_directories(nve_windows_shell_lib
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

if(MSVC)
  target_compile_options(nve_windows_shell_lib PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_windows_shell_lib PRIVATE -Wall -Wextra -Werror)
endif()

target_link_libraries(nve_windows_shell_lib
  PUBLIC
    nve_fabric_bridge
)

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

if(WIN32)
  target_link_libraries(nve_windows_shell_lib
    PUBLIC
      d2d1
      dwrite
      windowscodecs
      ole32
      user32
      gdi32
      ws2_32
  )
endif()

if(NVE_WITH_RNW_FABRIC)
  if(NOT WIN32)
    message(FATAL_ERROR "NVE_WITH_RNW_FABRIC=ON is only supported on Windows")
  endif()
  target_compile_definitions(nve_windows_shell_lib PUBLIC NVE_WITH_RNW_FABRIC=1)
  if(RNW_GENERATED_DIR)
    target_include_directories(nve_windows_shell_lib PUBLIC "${RNW_GENERATED_DIR}")
  endif()
  if(NVE_RNW_EXTRA_INCLUDE_DIRS)
    target_include_directories(nve_windows_shell_lib PUBLIC ${NVE_RNW_EXTRA_INCLUDE_DIRS})
  endif()
  target_link_libraries(nve_windows_shell_lib PUBLIC windowsapp ${NVE_RNW_LIBS})
endif()

add_executable(nve_windows_shell main.cpp)
if(MSVC)
  target_compile_options(nve_windows_shell PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_windows_shell PRIVATE -Wall -Wextra -Werror)
endif()
target_link_libraries(nve_windows_shell PRIVATE nve_windows_shell_lib)

enable_testing()

add_library(nve_win_fixture_no_entry SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_win_fixture_no_entry PRIVATE NVE_BUILD_SHARED_LIB=1 NVE_FIXTURE_NO_ENTRY=1)
target_include_directories(nve_win_fixture_no_entry PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include")

add_library(nve_win_fixture_no_surface SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_win_fixture_no_surface PRIVATE NVE_BUILD_SHARED_LIB=1 NVE_FIXTURE_NO_SURFACE=1)
target_include_directories(nve_win_fixture_no_surface PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include")

add_library(nve_win_fixture_stateful SHARED tests/test_dylib_fixtures.c)
target_compile_definitions(nve_win_fixture_stateful PRIVATE NVE_BUILD_SHARED_LIB=1 NVE_FIXTURE_STATEFUL=1)
target_include_directories(nve_win_fixture_stateful PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/include")

add_executable(nve_windows_shell_test tests/windows_shell_test.cpp)
target_include_directories(nve_windows_shell_test PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/src")
if(MSVC)
  target_compile_options(nve_windows_shell_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_windows_shell_test PRIVATE -Wall -Wextra -Werror)
endif()
target_compile_definitions(nve_windows_shell_test PRIVATE
  NVE_WIN_TEST_NO_ENTRY_DYLIB="$<TARGET_FILE:nve_win_fixture_no_entry>"
  NVE_WIN_TEST_NO_SURFACE_DYLIB="$<TARGET_FILE:nve_win_fixture_no_surface>"
  NVE_WIN_TEST_STATEFUL_DYLIB="$<TARGET_FILE:nve_win_fixture_stateful>"
)
target_link_libraries(nve_windows_shell_test PRIVATE nve_windows_shell_lib)
add_dependencies(nve_windows_shell_test
  nve_win_fixture_no_entry
  nve_win_fixture_no_surface
  nve_win_fixture_stateful
)
add_test(NAME windows_shell_test COMMAND nve_windows_shell_test)
