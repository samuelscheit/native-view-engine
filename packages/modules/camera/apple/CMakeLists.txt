cmake_minimum_required(VERSION 3.24)
project(nve_camera_apple LANGUAGES C CXX OBJCXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

set(NVE_REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../..")
set(NVE_APPLE_COMMON_DIR "${NVE_REPO_ROOT}/hosts/apple-common")
set(NVE_FABRIC_INCLUDE_DIR "${NVE_REPO_ROOT}/cxx/nve-fabric-bridge/include")

add_library(nve_camera_apple STATIC
  NveAppleCamera.mm
)

target_include_directories(nve_camera_apple
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${NVE_APPLE_COMMON_DIR}
    ${NVE_FABRIC_INCLUDE_DIR}
)

target_compile_options(nve_camera_apple PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_camera_apple
  PUBLIC
    "-framework AVFoundation"
    "-framework CoreMedia"
    "-framework CoreVideo"
    "-framework QuartzCore"
)

if(APPLE AND NOT IOS)
  target_link_libraries(nve_camera_apple PUBLIC "-framework Cocoa")
endif()

if(PROJECT_IS_TOP_LEVEL)
  if(NOT TARGET nve_fabric_bridge)
    add_subdirectory("${NVE_REPO_ROOT}/cxx/nve-fabric-bridge" "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge")
  endif()

  enable_testing()
  add_executable(nve_apple_camera_test
    tests/apple_camera_test.mm
    "${NVE_APPLE_COMMON_DIR}/NveAppleValueBridge.mm"
  )
  target_compile_options(nve_apple_camera_test PRIVATE -fobjc-arc -Wall -Wextra -Werror)
  target_include_directories(nve_apple_camera_test PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${NVE_APPLE_COMMON_DIR}
    "${NVE_REPO_ROOT}/cxx/nve-fabric-bridge/src"
  )
  target_link_libraries(nve_apple_camera_test PRIVATE nve_camera_apple nve_fabric_bridge)
  add_test(NAME apple_camera_test COMMAND nve_apple_camera_test)
endif()
