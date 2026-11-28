cmake_minimum_required(VERSION 3.24)
project(nve_native_views LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Header-only `NVNP` / `NVNC` codecs + `NativeViewRegistry` (`nve_native_view.hpp`).
if(NOT TARGET nve_host_cxx)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../../core/host-cxx" "${CMAKE_CURRENT_BINARY_DIR}/nve_host_cxx")
endif()

add_library(nve_native_views INTERFACE)
target_include_directories(nve_native_views INTERFACE ${CMAKE_CURRENT_SOURCE_DIR}/include)
target_link_libraries(nve_native_views INTERFACE nve_host_cxx)

if(PROJECT_IS_TOP_LEVEL)
  enable_testing()
  add_executable(nve_native_view_test tests/native_view_test.cpp)
  if(MSVC)
    target_compile_options(nve_native_view_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX)
  else()
    target_compile_options(nve_native_view_test PRIVATE -Wall -Wextra -Werror -Wconversion -Wshadow)
  endif()
  target_link_libraries(nve_native_view_test PRIVATE nve_native_views)
  add_test(NAME native_view_test COMMAND nve_native_view_test)
endif()
