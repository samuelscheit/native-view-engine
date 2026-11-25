cmake_minimum_required(VERSION 3.24)
project(nve_gtk_native_views LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_NV_ENABLE_BLUR "Enable BlurSurface native view" ON)
option(NVE_NV_ENABLE_WEBVIEW "Enable WebView native view" ON)
option(NVE_NV_ENABLE_VIDEO "Enable VideoPlayer native view" ON)

find_package(PkgConfig REQUIRED)
pkg_check_modules(GTK4 REQUIRED IMPORTED_TARGET gtk4)

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/nve-fabric-bridge"
    "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge"
  )
endif()

add_library(nve_gtk_native_views STATIC
  NveGtkNativeViews.hpp
  NveGtkNativeViews.cpp
)

target_include_directories(nve_gtk_native_views
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

target_compile_definitions(nve_gtk_native_views
  PUBLIC
    NVE_NV_ENABLE_BLUR=${_NVE_NV_BLUR_DEF}
    NVE_NV_ENABLE_WEBVIEW=${_NVE_NV_WEBVIEW_DEF}
    NVE_NV_ENABLE_VIDEO=${_NVE_NV_VIDEO_DEF}
)

target_compile_options(nve_gtk_native_views PRIVATE -Wall -Wextra -Werror)
target_link_libraries(nve_gtk_native_views
  PUBLIC
    nve_fabric_bridge
    PkgConfig::GTK4
)

if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
  enable_testing()
  add_executable(nve_gtk_native_views_test tests/gtk_native_views_test.cpp)
  target_compile_options(nve_gtk_native_views_test PRIVATE -Wall -Wextra -Werror)
  target_include_directories(nve_gtk_native_views_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/nve-fabric-bridge/src"
  )
  target_link_libraries(nve_gtk_native_views_test PRIVATE nve_gtk_native_views)
  add_test(NAME gtk_native_views_test COMMAND nve_gtk_native_views_test)
endif()
