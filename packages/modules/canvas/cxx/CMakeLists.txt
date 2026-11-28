cmake_minimum_required(VERSION 3.20)
project(nve_canvas_cxx LANGUAGES C CXX)

# Header-only C++20 `nve.canvas` leaf for native C++ hosts: generated table mirror,
# flat `u32`-word draw-list decoder / encoder, and the `CanvasTable` scene binding.
include("${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/cmake/NveCodegen.cmake")
nve_ensure_abi_codegen()

if(NOT TARGET nve_host_cxx)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../../core/host-cxx" "${CMAKE_CURRENT_BINARY_DIR}/nve_host_cxx")
endif()

add_library(nve_canvas_cxx INTERFACE)
add_library(nve::canvas_cxx ALIAS nve_canvas_cxx)
target_include_directories(nve_canvas_cxx INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_link_libraries(nve_canvas_cxx INTERFACE nve_host_cxx)

option(NVE_CANVAS_CXX_BUILD_TESTS "Build the nve_canvas_cxx test suite" ${PROJECT_IS_TOP_LEVEL})
if(NVE_CANVAS_CXX_BUILD_TESTS)
  enable_testing()
  add_executable(nve_canvas_draw_list_test tests/test_draw_list.cpp)
  set_target_properties(nve_canvas_draw_list_test PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF)
  target_link_libraries(nve_canvas_draw_list_test PRIVATE nve_canvas_cxx)
  target_compile_options(nve_canvas_draw_list_test PRIVATE -Wall -Wextra -Werror -pedantic -Wconversion -Wshadow)
  add_test(NAME nve_canvas_draw_list COMMAND nve_canvas_draw_list_test)
endif()
