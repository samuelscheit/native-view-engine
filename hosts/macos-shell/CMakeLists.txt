cmake_minimum_required(VERSION 3.24)
project(nve_macos_shell LANGUAGES C CXX OBJCXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if(NOT TARGET nve_fabric_bridge)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge" "${CMAKE_CURRENT_BINARY_DIR}/nve_fabric_bridge")
endif()

add_library(nve_macos_shell_lib STATIC
  NveMacFabricHost.mm
  RCTNveMacComponentView.mm
)

target_include_directories(nve_macos_shell_lib
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_compile_options(nve_macos_shell_lib PRIVATE -fobjc-arc -Wall -Wextra)
target_link_libraries(nve_macos_shell_lib
  PUBLIC
    nve_fabric_bridge
    "-framework Cocoa"
    "-framework QuartzCore"
)

add_executable(nve_macos_shell main.mm)
target_compile_options(nve_macos_shell PRIVATE -fobjc-arc -Wall -Wextra)
target_link_libraries(nve_macos_shell PRIVATE nve_macos_shell_lib)

enable_testing()
add_executable(nve_macos_shell_test tests/macos_shell_test.mm)
target_compile_options(nve_macos_shell_test PRIVATE -fobjc-arc -Wall -Wextra)
target_include_directories(nve_macos_shell_test PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/nve-fabric-bridge/src")
target_link_libraries(nve_macos_shell_test PRIVATE nve_macos_shell_lib)
add_test(NAME macos_shell_test COMMAND nve_macos_shell_test)
