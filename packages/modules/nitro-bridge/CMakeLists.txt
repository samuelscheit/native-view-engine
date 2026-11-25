cmake_minimum_required(VERSION 3.24)
project(nve_nitro_bridge LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(NVE_WITH_RN_JSI "Enable optional facebook::jsi::NativeState hook in nve_nitro_core.hpp" OFF)

add_library(nve_nitro_bridge STATIC
  src/nve_nitro_c_api.cpp
)

target_include_directories(nve_nitro_bridge
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)

find_package(Threads REQUIRED)
target_link_libraries(nve_nitro_bridge PUBLIC Threads::Threads)

if(MSVC)
  target_compile_options(nve_nitro_bridge PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX)
else()
  target_compile_options(nve_nitro_bridge PRIVATE -Wall -Wextra -Werror)
endif()

if(NVE_WITH_RN_JSI)
  target_compile_definitions(nve_nitro_bridge PUBLIC NVE_WITH_RN_JSI=1)
endif()

if(PROJECT_IS_TOP_LEVEL)
  enable_testing()
  add_executable(nve_nitro_bridge_test tests/nitro_bridge_test.cpp)
  if(MSVC)
    target_compile_options(nve_nitro_bridge_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX)
  else()
    target_compile_options(nve_nitro_bridge_test PRIVATE -Wall -Wextra -Werror)
  endif()
  target_link_libraries(nve_nitro_bridge_test PRIVATE nve_nitro_bridge)
  add_test(NAME nitro_bridge_test COMMAND nve_nitro_bridge_test)
endif()
