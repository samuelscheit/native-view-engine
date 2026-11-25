cmake_minimum_required(VERSION 3.24)
project(nve_camera_rnw LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

option(NVE_CAMERA_WIN32_MF "Enable native Windows Media Foundation (mfplat/mf/mfreadwrite) device enumeration" ON)

# Thin React Native Windows (0.88) CMake autolinking wrapper:
# Reuses shared Nitro bridge and C++20 JSI camera bridge with zero duplicated camera source code.
if(NOT TARGET nve_nitro_bridge)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../../../nitro-bridge" "${CMAKE_CURRENT_BINARY_DIR}/nve_nitro_bridge")
endif()

if(NOT TARGET nve_rn_camera_jsi)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../cxx" "${CMAKE_CURRENT_BINARY_DIR}/nve_rn_camera_jsi")
endif()

add_library(nve_camera_rnw INTERFACE)
target_link_libraries(nve_camera_rnw
  INTERFACE
    nve_rn_camera_jsi
    nve_nitro_bridge
)

if(WIN32 AND NVE_CAMERA_WIN32_MF)
  target_compile_definitions(nve_camera_rnw INTERFACE NVE_ENABLE_WIN32_MF=1)
  target_link_libraries(nve_camera_rnw
    INTERFACE
      mfplat
      mf
      mfreadwrite
      mfuuid
      ole32
  )
endif()
