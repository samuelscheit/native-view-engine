cmake_minimum_required(VERSION 3.24)
project(nve_android_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
add_subdirectory(../../cxx/nve-fabric-bridge nve-fabric-bridge-build)

add_library(nve_android_shell SHARED
  NveAndroidSurfaceJni.cpp
)

if(NOT MSVC)
  target_compile_options(nve_android_shell PRIVATE -Wall -Wextra -Werror)
endif()

target_link_libraries(nve_android_shell
  PRIVATE
    nve_fabric_bridge
    dl
)

if(NOT ANDROID)
  find_package(JNI REQUIRED)
  target_include_directories(nve_android_shell PRIVATE ${JNI_INCLUDE_DIRS})

  include(CTest)
  if(BUILD_TESTING)
    add_library(nve_jni_test_recorder SHARED tests/jni_test_recorder.cpp)
    target_include_directories(nve_jni_test_recorder PUBLIC tests)

    add_library(nve_mock_app_v1 SHARED tests/mock_app_v1.cpp)
    target_link_libraries(nve_mock_app_v1 PRIVATE nve_fabric_bridge nve_jni_test_recorder)

    add_library(nve_mock_app_v2 SHARED tests/mock_app_v2.cpp)
    target_link_libraries(nve_mock_app_v2 PRIVATE nve_fabric_bridge nve_jni_test_recorder)

    add_library(nve_mock_app_bad_iface SHARED tests/mock_app_bad_iface.cpp)
    target_link_libraries(nve_mock_app_bad_iface PRIVATE nve_fabric_bridge)

    add_library(nve_mock_scene_app SHARED tests/mock_scene_app.cpp)
    target_link_libraries(nve_mock_scene_app PRIVATE nve_fabric_bridge)

    add_executable(nve_android_scene_test tests/android_scene_jni_test.cpp)
    if(NOT MSVC)
      target_compile_options(nve_android_scene_test PRIVATE -Wall -Wextra -Werror)
    endif()
    target_include_directories(nve_android_scene_test PRIVATE ${JNI_INCLUDE_DIRS})
    target_link_libraries(nve_android_scene_test
      PRIVATE
        nve_android_shell
        nve_fabric_bridge
        nve_jni_test_recorder
    )
    add_test(
      NAME android_scene_jni_test
      COMMAND nve_android_scene_test
        $<TARGET_FILE:nve_mock_scene_app>
        $<TARGET_FILE:nve_mock_app_v1>
        $<TARGET_FILE:nve_mock_app_v2>
        $<TARGET_FILE:nve_mock_app_bad_iface>
        $<TARGET_FILE:nve_jni_test_recorder>
    )
  endif()
endif()
