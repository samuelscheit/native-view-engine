cmake_minimum_required(VERSION 3.24)
project(nve_rn_camera_jsi LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_RN_CAMERA_ENABLE_JSI "Link against React Native JSI when RN_REACT_COMMON_DIR is available" ON)
set(RN_REACT_COMMON_DIR "" CACHE PATH "Path to react-native/packages/react-native/ReactCommon")
set(RN_COMMON_DIR "" CACHE PATH "Alias for RN_REACT_COMMON_DIR")

if(NOT RN_REACT_COMMON_DIR AND RN_COMMON_DIR)
  set(RN_REACT_COMMON_DIR "${RN_COMMON_DIR}")
elseif(NOT RN_REACT_COMMON_DIR AND DEFINED ENV{RN_REACT_COMMON_DIR} AND EXISTS "$ENV{RN_REACT_COMMON_DIR}")
  set(RN_REACT_COMMON_DIR "$ENV{RN_REACT_COMMON_DIR}")
elseif(NOT RN_REACT_COMMON_DIR AND DEFINED ENV{RN_COMMON_DIR} AND EXISTS "$ENV{RN_COMMON_DIR}")
  set(RN_REACT_COMMON_DIR "$ENV{RN_COMMON_DIR}")
endif()

set(NVE_HAS_RN_JSI OFF)
if(RN_REACT_COMMON_DIR AND NVE_RN_CAMERA_ENABLE_JSI)
  if(EXISTS "${RN_REACT_COMMON_DIR}/jsi/jsi/CMakeLists.txt")
    set(NVE_HAS_RN_JSI ON)
  else()
    message(WARNING "RN_REACT_COMMON_DIR set, but JSI CMakeLists.txt not found at ${RN_REACT_COMMON_DIR}/jsi/jsi")
  endif()
endif()

if(NOT TARGET nve_camera_cxx)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx" "${CMAKE_CURRENT_BINARY_DIR}/nve_camera_cxx")
endif()

add_library(nve_rn_camera_jsi INTERFACE)
target_include_directories(nve_rn_camera_jsi
  INTERFACE
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)
target_link_libraries(nve_rn_camera_jsi INTERFACE nve_camera_cxx)

if(NVE_HAS_RN_JSI)
  if(NOT TARGET jsi)
    add_subdirectory("${RN_REACT_COMMON_DIR}/jsi/jsi" "${CMAKE_CURRENT_BINARY_DIR}/rn_jsi" EXCLUDE_FROM_ALL)
  endif()
  target_link_libraries(nve_rn_camera_jsi INTERFACE jsi)
  target_compile_definitions(nve_rn_camera_jsi INTERFACE NVE_WITH_RN_JSI=1)
  message(STATUS "nve_rn_camera_jsi: Linked React Native JSI from ${RN_REACT_COMMON_DIR}/jsi")
endif()

if(PROJECT_IS_TOP_LEVEL)
  enable_testing()

  add_executable(nve_rn_camera_jsi_test tests/rn_camera_jsi_test.cpp)
  if(MSVC)
    target_compile_options(nve_rn_camera_jsi_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
  else()
    target_compile_options(nve_rn_camera_jsi_test PRIVATE -Wall -Wextra -Werror)
  endif()
  target_link_libraries(nve_rn_camera_jsi_test PRIVATE nve_rn_camera_jsi)
  add_test(NAME rn_camera_jsi_test COMMAND nve_rn_camera_jsi_test)

  # When ReactCommon is not locally present, also compile & verify the full NVE_WITH_RN_JSI=1
  # HostObject / NativeState code path against our test-only facebook::jsi runtime mock.
  if(NOT NVE_HAS_RN_JSI)
    add_executable(nve_rn_camera_jsi_mock_test tests/rn_camera_jsi_test.cpp)
    target_include_directories(nve_rn_camera_jsi_mock_test PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests/mock_jsi)
    target_compile_definitions(nve_rn_camera_jsi_mock_test PRIVATE NVE_WITH_RN_JSI=1)
    if(MSVC)
      target_compile_options(nve_rn_camera_jsi_mock_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX /DWIN32_LEAN_AND_MEAN)
    else()
      target_compile_options(nve_rn_camera_jsi_mock_test PRIVATE -Wall -Wextra -Werror)
    endif()
    target_link_libraries(nve_rn_camera_jsi_mock_test PRIVATE nve_rn_camera_jsi)
    add_test(NAME rn_camera_jsi_mock_test COMMAND nve_rn_camera_jsi_mock_test)
  endif()
endif()
