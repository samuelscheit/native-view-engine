cmake_minimum_required(VERSION 3.24)
project(nve_fabric_bridge LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

include("${CMAKE_CURRENT_SOURCE_DIR}/../cmake/NveCodegen.cmake")
nve_ensure_abi_codegen()

option(NVE_WITH_RN_FABRIC "Enable full React Native Fabric ShadowTree/UIManager/EventListener integration" OFF)
option(NVE_WITH_RN_YOGA "Link against React Native Yoga (auto-enabled when RN_REACT_COMMON_DIR is set)" ON)
option(NVE_WITH_RN_JSI "Link against React Native JSI for MockJsiRecorderRuntime (auto-enabled when RN_REACT_COMMON_DIR is set)" ON)
option(NVE_BUILD_BENCHMARKS "Build C++20 Fabric bridge benchmarks" ${PROJECT_IS_TOP_LEVEL})
set(RN_REACT_COMMON_DIR "" CACHE PATH "Path to react-native/packages/react-native/ReactCommon")
set(RN_COMMON_DIR "" CACHE PATH "Alias for RN_REACT_COMMON_DIR")
set(NVE_RN_EXTRA_INCLUDE_DIRS "" CACHE STRING "Optional extra include dirs (e.g. Folly/glog/boost) when NVE_WITH_RN_FABRIC=ON")
set(NVE_RN_FABRIC_LIBS "" CACHE STRING "Optional prebuilt RN Fabric libraries to link when NVE_WITH_RN_FABRIC=ON")

if(NOT RN_REACT_COMMON_DIR AND RN_COMMON_DIR)
  set(RN_REACT_COMMON_DIR "${RN_COMMON_DIR}")
elseif(NOT RN_REACT_COMMON_DIR AND DEFINED ENV{RN_REACT_COMMON_DIR} AND EXISTS "$ENV{RN_REACT_COMMON_DIR}")
  set(RN_REACT_COMMON_DIR "$ENV{RN_REACT_COMMON_DIR}")
elseif(NOT RN_REACT_COMMON_DIR AND DEFINED ENV{RN_COMMON_DIR} AND EXISTS "$ENV{RN_COMMON_DIR}")
  set(RN_REACT_COMMON_DIR "$ENV{RN_COMMON_DIR}")
endif()

add_library(nve_fabric_bridge STATIC
  src/shadow_tree.cpp
  src/yoga_layout.cpp
  src/flexbox_solver.cpp
  src/scene_publisher.cpp
  src/fabric_c_api.cpp
  src/nve_rn_component.cpp
  src/pair_wire_v2.cpp
  src/pair_client.cpp
)

target_include_directories(nve_fabric_bridge
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
  PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
)

if(MSVC)
  target_compile_options(nve_fabric_bridge PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_fabric_bridge PRIVATE -Wall -Wextra)
  set_source_files_properties(
    src/pair_wire_v2.cpp
    src/pair_client.cpp
    PROPERTIES COMPILE_OPTIONS "-Wall;-Wextra;-Werror"
  )
endif()

find_package(Threads REQUIRED)
target_link_libraries(nve_fabric_bridge PUBLIC Threads::Threads)
if(WIN32)
  target_link_libraries(nve_fabric_bridge PUBLIC ws2_32)
endif()
if(UNIX AND NOT APPLE)
  target_link_libraries(nve_fabric_bridge PUBLIC ${CMAKE_DL_LIBS})
endif()

set(NVE_HAS_RN_YOGA OFF)
set(NVE_HAS_RN_JSI OFF)

if(RN_REACT_COMMON_DIR)
  if(NVE_WITH_RN_YOGA AND EXISTS "${RN_REACT_COMMON_DIR}/yoga/yoga/CMakeLists.txt")
    add_subdirectory("${RN_REACT_COMMON_DIR}/yoga/yoga" "${CMAKE_CURRENT_BINARY_DIR}/rn_yoga" EXCLUDE_FROM_ALL)
    target_link_libraries(nve_fabric_bridge PUBLIC yogacore)
    target_compile_definitions(nve_fabric_bridge PUBLIC NVE_WITH_RN_YOGA=1)
    set(NVE_HAS_RN_YOGA ON)
    message(STATUS "nve_fabric_bridge: Linked React Native Yoga from ${RN_REACT_COMMON_DIR}/yoga")
  elseif(NVE_WITH_RN_YOGA)
    message(WARNING "RN_REACT_COMMON_DIR set, but Yoga CMakeLists.txt not found at ${RN_REACT_COMMON_DIR}/yoga/yoga")
  endif()

  if(NVE_WITH_RN_JSI AND EXISTS "${RN_REACT_COMMON_DIR}/jsi/jsi/CMakeLists.txt")
    add_subdirectory("${RN_REACT_COMMON_DIR}/jsi/jsi" "${CMAKE_CURRENT_BINARY_DIR}/rn_jsi" EXCLUDE_FROM_ALL)
    target_link_libraries(nve_fabric_bridge PUBLIC jsi)
    target_compile_definitions(nve_fabric_bridge PUBLIC NVE_WITH_RN_JSI=1)
    set(NVE_HAS_RN_JSI ON)
    message(STATUS "nve_fabric_bridge: Linked React Native JSI from ${RN_REACT_COMMON_DIR}/jsi")
  elseif(NVE_WITH_RN_JSI)
    message(WARNING "RN_REACT_COMMON_DIR set, but JSI CMakeLists.txt not found at ${RN_REACT_COMMON_DIR}/jsi/jsi")
  endif()
endif()

if(NVE_WITH_RN_FABRIC)
  if(NOT RN_REACT_COMMON_DIR)
    message(FATAL_ERROR "RN_REACT_COMMON_DIR must be set when NVE_WITH_RN_FABRIC=ON")
  endif()
  target_compile_definitions(nve_fabric_bridge PUBLIC NVE_WITH_RN_FABRIC=1)
  target_include_directories(nve_fabric_bridge PUBLIC
    ${RN_REACT_COMMON_DIR}
    ${RN_REACT_COMMON_DIR}/yoga
    ${RN_REACT_COMMON_DIR}/jsi
    ${NVE_RN_EXTRA_INCLUDE_DIRS}
  )
  if(NVE_RN_FABRIC_LIBS)
    target_link_libraries(nve_fabric_bridge PUBLIC ${NVE_RN_FABRIC_LIBS})
  endif()
endif()

enable_testing()

add_library(nve_bridge_fixture_valid SHARED tests/session_fixture.c)
target_compile_definitions(nve_bridge_fixture_valid PRIVATE NVE_BUILD_SHARED_LIB=1)
target_include_directories(nve_bridge_fixture_valid PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/include)

add_library(nve_bridge_fixture_no_entry SHARED tests/session_fixture.c)
target_compile_definitions(nve_bridge_fixture_no_entry PRIVATE NVE_BUILD_SHARED_LIB=1 NVE_FIXTURE_NO_ENTRY=1)
target_include_directories(nve_bridge_fixture_no_entry PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/include)

add_library(nve_bridge_fixture_no_surface SHARED tests/session_fixture.c)
target_compile_definitions(nve_bridge_fixture_no_surface PRIVATE NVE_BUILD_SHARED_LIB=1 NVE_FIXTURE_NO_SURFACE=1)
target_include_directories(nve_bridge_fixture_no_surface PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/include)

add_executable(nve_fabric_bridge_test
  tests/test_main.cpp
  tests/test_abi_and_bridge.cpp
  tests/test_flexbox_layout.cpp
  tests/test_flexbox_parity.cpp
  tests/test_fuzz.cpp
  tests/test_host_runtime.cpp
  tests/test_mmap_snapshot.cpp
  tests/test_scene_view.cpp
)
target_include_directories(nve_fabric_bridge_test PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src)
target_compile_definitions(nve_fabric_bridge_test PRIVATE
  NVE_BRIDGE_FIXTURE_VALID="$<TARGET_FILE:nve_bridge_fixture_valid>"
  NVE_BRIDGE_FIXTURE_NO_ENTRY="$<TARGET_FILE:nve_bridge_fixture_no_entry>"
  NVE_BRIDGE_FIXTURE_NO_SURFACE="$<TARGET_FILE:nve_bridge_fixture_no_surface>"
)
if(MSVC)
  target_compile_options(nve_fabric_bridge_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
  target_link_options(nve_fabric_bridge_test PRIVATE /STACK:8388608)
else()
  target_compile_options(nve_fabric_bridge_test PRIVATE -Wall -Wextra -Werror)
endif()
target_link_libraries(nve_fabric_bridge_test PRIVATE nve_fabric_bridge)
add_dependencies(nve_fabric_bridge_test
  nve_bridge_fixture_valid
  nve_bridge_fixture_no_entry
  nve_bridge_fixture_no_surface
)
add_test(NAME bridge_abi_test COMMAND nve_fabric_bridge_test)

add_executable(nve_pair_client_test
  tests/test_pair_client.cpp
)
target_include_directories(nve_pair_client_test PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src)
if(MSVC)
  target_compile_options(nve_pair_client_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_pair_client_test PRIVATE -Wall -Wextra -Werror)
endif()
target_link_libraries(nve_pair_client_test PRIVATE nve_fabric_bridge)
add_test(NAME pair_client_test COMMAND nve_pair_client_test)

add_executable(nve_native_view_test
  tests/test_native_view.cpp
)
target_include_directories(nve_native_view_test PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src)
if(MSVC)
  target_compile_options(nve_native_view_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
else()
  target_compile_options(nve_native_view_test PRIVATE -Wall -Wextra -Werror)
endif()
target_link_libraries(nve_native_view_test PRIVATE nve_fabric_bridge)
add_test(NAME native_view_test COMMAND nve_native_view_test)

if(NVE_BUILD_BENCHMARKS)
  add_executable(nve_fabric_bridge_bench bench/bridge_bench.cpp)
  target_include_directories(nve_fabric_bridge_bench PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src)
  if(MSVC)
    target_compile_options(nve_fabric_bridge_bench PRIVATE /Zc:__cplusplus /utf-8 /W4 /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
  else()
    target_compile_options(nve_fabric_bridge_bench PRIVATE -Wall -Wextra)
  endif()
  target_link_libraries(nve_fabric_bridge_bench PRIVATE nve_fabric_bridge)
endif()
