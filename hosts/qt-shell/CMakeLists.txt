cmake_minimum_required(VERSION 3.24)
project(nve_qt_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

foreach(_nve_qt_hint IN ITEMS
    "/opt/homebrew/opt/qt"
    "/opt/homebrew/opt/qtbase"
    "/opt/homebrew"
    "/usr/local/opt/qt"
    "/usr/local/opt/qtbase")
  if(EXISTS "${_nve_qt_hint}")
    list(APPEND CMAKE_PREFIX_PATH "${_nve_qt_hint}")
  endif()
endforeach()

find_package(Qt6 REQUIRED COMPONENTS Widgets)

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge"
    "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge"
  )
endif()

add_library(nve_qt_shell_lib STATIC
  NveQtWidget.hpp
  NveQtWidget.cpp
)

target_include_directories(nve_qt_shell_lib
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

if(MSVC)
  target_compile_options(nve_qt_shell_lib PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_qt_shell_lib PRIVATE -Wall -Wextra)
endif()

target_link_libraries(nve_qt_shell_lib
  PUBLIC
    nve_fabric_bridge
    Qt6::Widgets
)

add_executable(nve_qt_shell main.cpp)
if(MSVC)
  target_compile_options(nve_qt_shell PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_qt_shell PRIVATE -Wall -Wextra)
endif()
target_link_libraries(nve_qt_shell PRIVATE nve_qt_shell_lib)

enable_testing()

add_executable(nve_qt_shell_test tests/qt_shell_test.cpp)
if(MSVC)
  target_compile_options(nve_qt_shell_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_qt_shell_test PRIVATE -Wall -Wextra)
endif()
target_include_directories(nve_qt_shell_test PRIVATE
  "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/src"
)
target_compile_definitions(nve_qt_shell_test PRIVATE
  NVE_TEST_VALID_DYLIB="$<TARGET_FILE:nve_bridge_fixture_valid>"
  NVE_TEST_NO_ENTRY_DYLIB="$<TARGET_FILE:nve_bridge_fixture_no_entry>"
  NVE_TEST_NO_SURFACE_DYLIB="$<TARGET_FILE:nve_bridge_fixture_no_surface>"
)
target_link_libraries(nve_qt_shell_test PRIVATE nve_qt_shell_lib)
add_dependencies(nve_qt_shell_test
  nve_bridge_fixture_valid
  nve_bridge_fixture_no_entry
  nve_bridge_fixture_no_surface
)

add_test(NAME qt_shell_test COMMAND nve_qt_shell_test)
set_tests_properties(qt_shell_test PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")
