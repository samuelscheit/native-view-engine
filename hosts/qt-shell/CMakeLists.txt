cmake_minimum_required(VERSION 3.24)
project(nve_qt_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_ENABLE_NATIVE_VIEWS "Enable packages/modules/native-views" OFF)

# Prefer unified prefixes (/opt/homebrew, /opt/homebrew/opt/qt) before split kegs (qtbase,
# qtdeclarative) so Qt6Config.cmake discovers Widgets, Quick, and Qml together.
foreach(_nve_qt_hint IN ITEMS
    "/opt/homebrew/opt/qt"
    "/opt/homebrew"
    "/opt/homebrew/opt/qtdeclarative"
    "/opt/homebrew/opt/qtbase"
    "/usr/local/opt/qt"
    "/usr/local"
    "/usr/local/opt/qtdeclarative"
    "/usr/local/opt/qtbase")
  if(EXISTS "${_nve_qt_hint}")
    list(APPEND CMAKE_PREFIX_PATH "${_nve_qt_hint}")
  endif()
endforeach()

find_package(Qt6 REQUIRED COMPONENTS Widgets OPTIONAL_COMPONENTS Quick Qml)
if(NOT TARGET Qt6::Quick OR NOT TARGET Qt6::Qml)
  find_package(Qt6Qml QUIET PATHS
    "/opt/homebrew/opt/qtdeclarative/lib/cmake"
    "/opt/homebrew/lib/cmake"
    "/usr/local/opt/qtdeclarative/lib/cmake"
    "/usr/local/lib/cmake")
  find_package(Qt6Quick QUIET PATHS
    "/opt/homebrew/opt/qtdeclarative/lib/cmake"
    "/opt/homebrew/lib/cmake"
    "/usr/local/opt/qtdeclarative/lib/cmake"
    "/usr/local/lib/cmake")
endif()

include("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/cmake/NveCodegen.cmake")
nve_ensure_abi_codegen()

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge"
    "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge"
  )
endif()

add_library(nve_qt_shell_lib STATIC
  NveQtCommon.hpp
  NveQtWidget.hpp
  NveQtWidget.cpp
  NveQtCustomWidgets.cpp
  NveQtTextInputA11y.cpp
  NveQtWidgetReconciler.cpp
  NveQtWindowManager.hpp
  NveQtWindowManager.cpp
)

target_include_directories(nve_qt_shell_lib
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

if(MSVC)
  target_compile_options(nve_qt_shell_lib PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_qt_shell_lib PRIVATE -Wall -Wextra -Werror)
endif()

target_link_libraries(nve_qt_shell_lib
  PUBLIC
    nve_fabric_bridge
    Qt6::Widgets
)

if(NVE_ENABLE_NATIVE_VIEWS)
  if(NOT TARGET nve_qt_native_views)
    add_subdirectory(
      "${CMAKE_CURRENT_SOURCE_DIR}/../../packages/modules/native-views/qt"
      "${CMAKE_CURRENT_BINARY_DIR}/nve_qt_native_views"
    )
  endif()
  target_link_libraries(nve_qt_shell_lib PUBLIC nve_qt_native_views)
  target_compile_definitions(nve_qt_shell_lib PUBLIC NVE_ENABLE_NATIVE_VIEWS=1)
endif()

set(NVE_QT_HAS_QUICK OFF)
if(TARGET Qt6::Quick AND TARGET Qt6::Qml)
  set(NVE_QT_HAS_QUICK ON)
  message(STATUS "nve_qt_shell: Qt6::Quick found — enabling GPU Scene Graph NveQtQuickItem")
  target_sources(nve_qt_shell_lib
    PRIVATE
      NveQtTextCache.hpp
      NveQtTextCache.cpp
      NveQtSGBuilder.hpp
      NveQtSGBuilder.cpp
      NveQtQuickItem.hpp
      NveQtQuickItem.cpp
  )
  target_compile_definitions(nve_qt_shell_lib PUBLIC NVE_QT_HAS_QUICK=1)
  target_link_libraries(nve_qt_shell_lib
    PUBLIC
      Qt6::Quick
      Qt6::Qml
  )
else()
  message(STATUS "nve_qt_shell: Qt6::Quick not found — building QWidget host only")
endif()

add_executable(nve_qt_shell main.cpp)
if(MSVC)
  target_compile_options(nve_qt_shell PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_qt_shell PRIVATE -Wall -Wextra)
endif()
target_link_libraries(nve_qt_shell PRIVATE nve_qt_shell_lib)

enable_testing()

add_executable(nve_qt_shell_test
  tests/qt_shell_test.cpp
  tests/qt_window_manager_test.cpp
)
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

if(NVE_QT_HAS_QUICK)
  add_executable(nve_qt_quick_test tests/qt_quick_test.cpp)
  if(MSVC)
    target_compile_options(nve_qt_quick_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
  else()
    target_compile_options(nve_qt_quick_test PRIVATE -Wall -Wextra)
  endif()
  target_include_directories(nve_qt_quick_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/src"
  )
  target_compile_definitions(nve_qt_quick_test PRIVATE
    NVE_TEST_VALID_DYLIB="$<TARGET_FILE:nve_bridge_fixture_valid>"
    NVE_TEST_NO_ENTRY_DYLIB="$<TARGET_FILE:nve_bridge_fixture_no_entry>"
    NVE_TEST_NO_SURFACE_DYLIB="$<TARGET_FILE:nve_bridge_fixture_no_surface>"
  )
  target_link_libraries(nve_qt_quick_test PRIVATE nve_qt_shell_lib)
  add_dependencies(nve_qt_quick_test
    nve_bridge_fixture_valid
    nve_bridge_fixture_no_entry
    nve_bridge_fixture_no_surface
  )

  add_test(NAME qt_quick_test COMMAND nve_qt_quick_test)
  set_tests_properties(qt_quick_test PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen;QT_QUICK_BACKEND=software")
endif()
