cmake_minimum_required(VERSION 3.24)
project(nve_qt_native_views LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_NV_ENABLE_BLUR "Enable BlurSurface native view" ON)
option(NVE_NV_ENABLE_WEBVIEW "Enable WebView native view" ON)
option(NVE_NV_ENABLE_VIDEO "Enable VideoPlayer native view" ON)

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

find_package(Qt6 REQUIRED COMPONENTS Widgets)

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/nve-fabric-bridge"
    "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge"
  )
endif()

add_library(nve_qt_native_views STATIC
  NveQtNativeViews.hpp
  NveQtNativeViews.cpp
)

target_include_directories(nve_qt_native_views
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

if(NVE_NV_ENABLE_BLUR)
  set(_NVE_NV_BLUR_DEF 1)
else()
  set(_NVE_NV_BLUR_DEF 0)
endif()

if(NVE_NV_ENABLE_WEBVIEW)
  set(_NVE_NV_WEBVIEW_DEF 1)
else()
  set(_NVE_NV_WEBVIEW_DEF 0)
endif()

if(NVE_NV_ENABLE_VIDEO)
  set(_NVE_NV_VIDEO_DEF 1)
else()
  set(_NVE_NV_VIDEO_DEF 0)
endif()

target_compile_definitions(nve_qt_native_views
  PUBLIC
    NVE_NV_ENABLE_BLUR=${_NVE_NV_BLUR_DEF}
    NVE_NV_ENABLE_WEBVIEW=${_NVE_NV_WEBVIEW_DEF}
    NVE_NV_ENABLE_VIDEO=${_NVE_NV_VIDEO_DEF}
)

if(MSVC)
  target_compile_options(nve_qt_native_views PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_qt_native_views PRIVATE -Wall -Wextra -Werror)
endif()

target_link_libraries(nve_qt_native_views
  PUBLIC
    nve_fabric_bridge
    Qt6::Widgets
)

if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
  enable_testing()
  add_executable(nve_qt_native_views_test tests/qt_native_views_test.cpp)
  target_include_directories(nve_qt_native_views_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/nve-fabric-bridge/src"
  )
  if(MSVC)
    target_compile_options(nve_qt_native_views_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
  else()
    target_compile_options(nve_qt_native_views_test PRIVATE -Wall -Wextra -Werror)
  endif()
  target_link_libraries(nve_qt_native_views_test PRIVATE nve_qt_native_views)
  add_test(NAME qt_native_views_test COMMAND nve_qt_native_views_test)
  set_tests_properties(qt_native_views_test PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")
endif()
