cmake_minimum_required(VERSION 3.24)
project(nve_qt_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_ENABLE_NATIVE_VIEWS "Enable packages/modules/native-views" OFF)

# Unified prefixes first, then the split Homebrew kegs (qtbase + qtdeclarative).
foreach(_nve_qt_hint IN ITEMS
    "/opt/homebrew/opt/qt" "/opt/homebrew/opt/qtbase" "/opt/homebrew/opt/qtdeclarative"
    "/usr/local/opt/qt" "/usr/local/opt/qtbase" "/usr/local/opt/qtdeclarative")
  if(EXISTS "${_nve_qt_hint}")
    list(APPEND CMAKE_PREFIX_PATH "${_nve_qt_hint}")
  endif()
endforeach()
find_package(Qt6 REQUIRED COMPONENTS Widgets OPTIONAL_COMPONENTS Quick Qml)

# `nve::host_cxx` + `nve::runtime` (libnve_runtime) + `nve::canvas_cxx` + app-module fixtures.
include("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/cmake/NveHostStack.cmake")

if(MSVC)
  set(_nve_qt_warnings /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  set(_nve_qt_warnings -Wall -Wextra -Werror)
endif()

add_library(nve_qt_shell_lib STATIC
  NveQtDrawList.cpp
  NveQtHost.cpp
  NveQtImageCache.cpp
  NveQtPaint.cpp
  NveQtSemantics.cpp
  NveQtText.cpp
  NveQtWidget.hpp
  NveQtWidget.cpp
  NveQtCustomWidgets.cpp
  NveQtWidgetReconciler.cpp
  NveQtWindowManager.hpp
  NveQtWindowManager.cpp
)
target_include_directories(nve_qt_shell_lib PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(nve_qt_shell_lib PRIVATE ${_nve_qt_warnings})
target_link_libraries(nve_qt_shell_lib PUBLIC nve::host_cxx nve::runtime nve::canvas_cxx Qt6::Widgets)

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

add_executable(nve_qt_shell main.cpp)
target_compile_options(nve_qt_shell PRIVATE ${_nve_qt_warnings})
target_link_libraries(nve_qt_shell PRIVATE nve_qt_shell_lib)

enable_testing()

nve_add_app_module_fixtures(nve_qt_fixture)

add_executable(nve_qt_shell_test
  tests/qt_shell_test.cpp
  tests/qt_shell_input_scroll_test.cpp
  tests/qt_shell_a11y_media_test.cpp
  tests/qt_shell_canvas_pair_test.cpp
  tests/qt_window_manager_test.cpp
)
target_compile_options(nve_qt_shell_test PRIVATE ${_nve_qt_warnings})
nve_use_app_module_fixtures(nve_qt_shell_test nve_qt_fixture)
target_link_libraries(nve_qt_shell_test PRIVATE nve_qt_shell_lib nve::host_test_support)
add_test(NAME qt_shell_test COMMAND nve_qt_shell_test)
set_tests_properties(qt_shell_test PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")

nve_host_sanitize(nve_qt_shell_lib nve_qt_shell nve_qt_shell_test)
