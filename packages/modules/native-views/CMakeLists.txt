cmake_minimum_required(VERSION 3.24)
project(nve_native_views LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Independent per-view compile toggles. Disabling any toggle compiles zero bytes
# of that view's implementation, backend interfaces, or registry bindings.
option(NVE_NV_ENABLE_BLUR "Enable BlurSurface native view" ON)
option(NVE_NV_ENABLE_WEBVIEW "Enable WebView native view" ON)
option(NVE_NV_ENABLE_VIDEO "Enable VideoPlayer native view" ON)

option(NVE_NV_BUILD_WINDOWS "Build Windows native views submodule" OFF)
option(NVE_NV_BUILD_QT "Build Qt 6 native views submodule" OFF)
option(NVE_NV_BUILD_GTK "Build GTK 4 native views submodule" OFF)

if(NVE_NV_BUILD_WINDOWS)
  add_subdirectory(windows)
endif()

if(NVE_NV_BUILD_QT)
  add_subdirectory(qt)
endif()

if(NVE_NV_BUILD_GTK)
  add_subdirectory(gtk)
endif()
