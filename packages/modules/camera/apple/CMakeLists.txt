cmake_minimum_required(VERSION 3.24)
project(nve_camera_apple LANGUAGES C CXX OBJCXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

set(NVE_REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../..")
set(NVE_APPLE_COMMON_DIR "${NVE_REPO_ROOT}/hosts/apple-common")
if(NOT TARGET nve_native_views)
  add_subdirectory("${NVE_REPO_ROOT}/packages/modules/native-views/cxx" "${CMAKE_CURRENT_BINARY_DIR}/nve_native_views")
endif()

add_library(nve_camera_apple STATIC
  NveAppleCamera.mm
)

target_include_directories(nve_camera_apple
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${NVE_APPLE_COMMON_DIR}
)
target_link_libraries(nve_camera_apple PUBLIC nve_native_views)

target_compile_options(nve_camera_apple PRIVATE -fobjc-arc -Wall -Wextra -Werror)
target_link_libraries(nve_camera_apple PUBLIC "-framework Foundation")

if(APPLE AND NOT IOS)
  target_link_libraries(nve_camera_apple PUBLIC "-framework Cocoa")
endif()

if(PROJECT_IS_TOP_LEVEL)
  enable_testing()
  add_executable(nve_apple_camera_test
    tests/apple_camera_test.mm
    "${NVE_APPLE_COMMON_DIR}/NveAppleValueBridge.mm"
  )
  target_compile_options(nve_apple_camera_test PRIVATE -fobjc-arc -Wall -Wextra -Werror)
  target_include_directories(nve_apple_camera_test PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${NVE_APPLE_COMMON_DIR}
  )
  target_link_libraries(nve_apple_camera_test PRIVATE nve_camera_apple)
  add_test(NAME apple_camera_test COMMAND nve_apple_camera_test)
endif()
