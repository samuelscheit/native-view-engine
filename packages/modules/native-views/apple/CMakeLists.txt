cmake_minimum_required(VERSION 3.24)
project(nve_apple_native_views LANGUAGES C CXX OBJCXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_NV_ENABLE_BLUR "Enable Apple BlurSurface native view (NSVisualEffectView / UIVisualEffectView)" ON)
option(NVE_NV_ENABLE_WEBVIEW "Enable Apple WebView native view (WKWebView)" ON)
option(NVE_NV_ENABLE_VIDEO "Enable Apple VideoPlayer native view (AVPlayerLayer)" ON)

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/nve-fabric-bridge"
    "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge"
  )
endif()

add_library(nve_apple_native_views STATIC
  NveAppleNativeViews.h
  NveAppleNativeViews.mm
)

target_include_directories(nve_apple_native_views
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

target_compile_definitions(nve_apple_native_views
  PUBLIC
    NVE_NV_ENABLE_BLUR=${_NVE_NV_BLUR_DEF}
    NVE_NV_ENABLE_WEBVIEW=${_NVE_NV_WEBVIEW_DEF}
    NVE_NV_ENABLE_VIDEO=${_NVE_NV_VIDEO_DEF}
)

target_compile_options(nve_apple_native_views PRIVATE -fobjc-arc -Wall -Wextra -Werror)

target_link_libraries(nve_apple_native_views
  PUBLIC
    nve_fabric_bridge
    "-framework Foundation"
    "-framework QuartzCore"
)

if(APPLE AND NOT IOS)
  target_link_libraries(nve_apple_native_views PUBLIC "-framework Cocoa")
endif()

if(NVE_NV_ENABLE_WEBVIEW)
  target_link_libraries(nve_apple_native_views PUBLIC "-framework WebKit")
endif()

if(NVE_NV_ENABLE_VIDEO)
  target_link_libraries(nve_apple_native_views
    PUBLIC
      "-framework AVFoundation"
      "-framework CoreMedia"
  )
endif()

if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
  enable_testing()
  add_executable(nve_apple_native_views_test tests/apple_native_views_test.mm)
  target_include_directories(nve_apple_native_views_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/nve-fabric-bridge/src"
  )
  target_compile_options(nve_apple_native_views_test PRIVATE -fobjc-arc -Wall -Wextra -Werror)
  target_link_libraries(nve_apple_native_views_test PRIVATE nve_apple_native_views)
  add_test(NAME apple_native_views_test COMMAND nve_apple_native_views_test)
endif()
