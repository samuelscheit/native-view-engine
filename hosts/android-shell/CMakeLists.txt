cmake_minimum_required(VERSION 3.24)
project(nve_android_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
add_subdirectory(../../cxx/nve-fabric-bridge nve-fabric-bridge-build)

add_library(nve_android_shell SHARED
  NveJniBridge.cpp
)

target_link_libraries(nve_android_shell
  PRIVATE
    nve_fabric_bridge
    dl
)
