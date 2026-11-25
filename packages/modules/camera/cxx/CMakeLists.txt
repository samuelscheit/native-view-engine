cmake_minimum_required(VERSION 3.24)
project(nve_camera_cxx LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if(NOT TARGET nve_nitro_bridge)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../nitro-bridge" "${CMAKE_CURRENT_BINARY_DIR}/nve_nitro_bridge")
endif()

add_library(nve_camera_cxx INTERFACE)
target_include_directories(nve_camera_cxx
  INTERFACE
    ${CMAKE_CURRENT_SOURCE_DIR}/include
    ${CMAKE_CURRENT_SOURCE_DIR}/../../../../cxx/nve-fabric-bridge/include
)
target_link_libraries(nve_camera_cxx INTERFACE nve_nitro_bridge)

if(PROJECT_IS_TOP_LEVEL)
  enable_testing()
  add_executable(nve_camera_test tests/camera_test.cpp)
  if(MSVC)
    target_compile_options(nve_camera_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
  else()
    target_compile_options(nve_camera_test PRIVATE -Wall -Wextra -Werror)
  endif()
  target_link_libraries(nve_camera_test PRIVATE nve_camera_cxx)
  add_test(NAME camera_test COMMAND nve_camera_test)
endif()
