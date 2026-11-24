cmake_minimum_required(VERSION 3.24)
project(nve_fabric_bridge LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_WITH_RN_FABRIC "Link against React Native Fabric C++ headers/libraries" OFF)
set(RN_REACT_COMMON_DIR "" CACHE PATH "Path to react-native/packages/react-native/ReactCommon")

add_library(nve_fabric_bridge STATIC
  src/fabric_adapter.cpp
)

target_include_directories(nve_fabric_bridge
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
  PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
)

if(NVE_WITH_RN_FABRIC)
  if(NOT RN_REACT_COMMON_DIR)
    message(FATAL_ERROR "RN_REACT_COMMON_DIR must be set when NVE_WITH_RN_FABRIC=ON")
  endif()
  target_compile_definitions(nve_fabric_bridge PRIVATE NVE_WITH_RN_FABRIC=1)
  target_include_directories(nve_fabric_bridge PRIVATE ${RN_REACT_COMMON_DIR})
endif()

enable_testing()
add_executable(nve_fabric_bridge_test tests/bridge_test.cpp)
target_link_libraries(nve_fabric_bridge_test PRIVATE nve_fabric_bridge)
add_test(NAME bridge_abi_test COMMAND nve_fabric_bridge_test)
