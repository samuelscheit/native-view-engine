cmake_minimum_required(VERSION 3.24)
project(nve_nitro_jni LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

add_subdirectory(../.. nve-nitro-bridge-build)

add_library(nve_nitro_jni SHARED
  nve_nitro_jni.cpp
)

target_include_directories(nve_nitro_jni
  PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(nve_nitro_jni
  PUBLIC
    nve_nitro_bridge
)

if(MSVC)
  target_compile_options(nve_nitro_jni PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX)
else()
  target_compile_options(nve_nitro_jni PRIVATE -Wall -Wextra -Werror)
endif()

if(NOT ANDROID)
  find_package(JNI REQUIRED)
  target_include_directories(nve_nitro_jni PUBLIC ${JNI_INCLUDE_DIRS})

  include(CTest)
  if(BUILD_TESTING)
    add_executable(nve_nitro_jni_test tests/nitro_jni_test.cpp)
    if(MSVC)
      target_compile_options(nve_nitro_jni_test PRIVATE /Zc:__cplusplus /utf-8 /W4 /WX /DNOMINMAX)
    else()
      target_compile_options(nve_nitro_jni_test PRIVATE -Wall -Wextra -Werror)
    endif()
    target_link_libraries(nve_nitro_jni_test
      PRIVATE
        nve_nitro_jni
        nve_nitro_bridge
    )
    add_test(NAME nitro_jni_test COMMAND nve_nitro_jni_test)
  endif()
endif()
