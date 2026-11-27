cmake_minimum_required(VERSION 3.24)
project(nve_android_shell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# `nve::runtime` (`libnve_runtime` staticlib) + `nve/host/*` headers (`RuntimeSession`, generated ABI).
include("${CMAKE_CURRENT_SOURCE_DIR}/../../cxx/cmake/NveRuntime.cmake")

get_filename_component(NVE_REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
set(NVE_HOST_JAVA_SRC "${NVE_REPO_ROOT}/packages/core/host-java/src/main/java")
set(NVE_ANDROID_GENERATED "${CMAKE_CURRENT_SOURCE_DIR}/android-app/src/dev/nve/android/generated")

# Lock the Java half of the JNI contract (packed scene frame, schema, statuses) at compile time:
# the constants are read from the Java sources and `static_assert`ed in `NveAndroidSurface.hpp`.
function(nve_java_constant out_var java_file pattern)
  file(READ "${java_file}" _src)
  if(NOT _src MATCHES "${pattern}")
    message(FATAL_ERROR "android-shell: `${pattern}` not found in ${java_file}")
  endif()
  set(${out_var} "${CMAKE_MATCH_1}" PARENT_SCOPE)
  set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${java_file}")
endfunction()
set(_nve_reader "${NVE_HOST_JAVA_SRC}/dev/nve/host/RuntimeSceneReader.java")
set(_nve_native "${NVE_HOST_JAVA_SRC}/dev/nve/host/NativeRuntime.java")
nve_java_constant(_schema "${_nve_reader}" "SCHEMA = Key64\\.of\\((0x[0-9A-Fa-f]+)L\\)")
nve_java_constant(_max_tables "${_nve_reader}" "MAX_TABLES = ([0-9]+);")
nve_java_constant(_h_tables "${_nve_reader}" "H_TABLES = ([0-9]+);")
nve_java_constant(_h_stride "${_nve_reader}" "H_TABLE_STRIDE = ([0-9]+);")
nve_java_constant(_s_tables "${_nve_reader}" "S_TABLES = ([0-9]+);")
nve_java_constant(_s_stride "${_nve_reader}" "S_TABLE_STRIDE = ([0-9]+);")
nve_java_constant(_st_busy "${_nve_native}" "STATUS_BUSY = ([0-9]+);")
nve_java_constant(_st_error "${_nve_native}" "STATUS_ERROR = ([0-9]+);")

add_library(nve_android_shell SHARED
  NveAndroidRuntimeJni.cpp
  NveAndroidSceneJni.cpp
  NveAndroidEventsJni.cpp
)
target_compile_definitions(nve_android_shell PRIVATE
  NVE_JAVA_RUNTIME_SCHEMA=${_schema}ULL
  NVE_JAVA_MAX_TABLES=${_max_tables}u
  NVE_JAVA_H_TABLES=${_h_tables}u
  NVE_JAVA_H_TABLE_STRIDE=${_h_stride}u
  NVE_JAVA_S_TABLES=${_s_tables}u
  NVE_JAVA_S_TABLE_STRIDE=${_s_stride}u
  NVE_JAVA_STATUS_BUSY=${_st_busy}
  NVE_JAVA_STATUS_ERROR=${_st_error}
)
if(NOT MSVC)
  target_compile_options(nve_android_shell PRIVATE -Wall -Wextra -Werror)
endif()
target_link_libraries(nve_android_shell PRIVATE nve::runtime ${CMAKE_DL_LIBS})

if(NOT ANDROID)
  # Desktop build: the same JNI library runs under a real JVM for the end-to-end suites.
  find_package(JNI REQUIRED)
  target_include_directories(nve_android_shell PRIVATE ${JNI_INCLUDE_DIRS})

  include(CTest)
  if(BUILD_TESTING)
    find_package(Java 11 REQUIRED COMPONENTS Runtime Development)
    find_program(NVE_CARGO cargo HINTS "$ENV{CARGO_HOME}/bin" "$ENV{HOME}/.cargo/bin" REQUIRED)

    # Real NVE2 frames from the production `NodeStore` + standard tables.
    set(NVE_FIXTURES "${CMAKE_CURRENT_BINARY_DIR}/fixtures")
    set(_nve_fixture_files
      "${NVE_FIXTURES}/showcase_1.nve2" "${NVE_FIXTURES}/showcase_2.nve2"
      "${NVE_FIXTURES}/hot_v1.nve2" "${NVE_FIXTURES}/hot_v2.nve2" "${NVE_FIXTURES}/showcase.ids")
    add_custom_command(
      OUTPUT ${_nve_fixture_files}
      COMMAND "${NVE_CARGO}" run --locked -q -p nve-host-fixtures -- "${NVE_FIXTURES}"
      WORKING_DIRECTORY "${NVE_REPO_ROOT}"
      DEPENDS "${NVE_REPO_ROOT}/packages/tools/host-fixtures/src/main.rs"
      COMMENT "Writing NVE2 host fixtures"
      VERBATIM)
    add_custom_target(nve_host_fixtures ALL DEPENDS ${_nve_fixture_files})

    function(nve_mock_app name)
      add_library(${name} MODULE tests/mock_app.cpp)
      target_include_directories(${name} PRIVATE "${NVE_REPO_ROOT}/packages/core/host-cxx/include")
      target_compile_definitions(${name} PRIVATE ${ARGN})
      target_compile_options(${name} PRIVATE -Wall -Wextra -Werror -fvisibility=hidden)
      add_dependencies(${name} nve_host_fixtures)
    endfunction()
    nve_mock_app(nve_mock_showcase
      NVE_MOCK_FRAME="${NVE_FIXTURES}/showcase_1.nve2" NVE_MOCK_NEXT_FRAME="${NVE_FIXTURES}/showcase_2.nve2")
    nve_mock_app(nve_mock_hot_v1 NVE_MOCK_FRAME="${NVE_FIXTURES}/hot_v1.nve2")
    nve_mock_app(nve_mock_hot_v2 NVE_MOCK_FRAME="${NVE_FIXTURES}/hot_v2.nve2")
    nve_mock_app(nve_mock_bad_iface NVE_MOCK_BAD_IFACE=1)

    # JVM suites: `dev.nve.host` + generated tables/ABI + the tests, compiled with `javac`.
    file(GLOB_RECURSE _nve_jvm_sources CONFIGURE_DEPENDS
      "${NVE_HOST_JAVA_SRC}/*.java"
      "${CMAKE_CURRENT_SOURCE_DIR}/tests/jvm/*.java")
    list(APPEND _nve_jvm_sources "${NVE_ANDROID_GENERATED}/NveTables.java" "${NVE_ANDROID_GENERATED}/NveCoreAbi.java")
    set(NVE_JVM_CLASSES "${CMAKE_CURRENT_BINARY_DIR}/jvm-classes")
    add_custom_command(
      OUTPUT "${NVE_JVM_CLASSES}/.stamp"
      COMMAND "${CMAKE_COMMAND}" -E rm -rf "${NVE_JVM_CLASSES}"
      COMMAND "${Java_JAVAC_EXECUTABLE}" --release 11 -Xlint:all -Werror -d "${NVE_JVM_CLASSES}" ${_nve_jvm_sources}
      COMMAND "${CMAKE_COMMAND}" -E touch "${NVE_JVM_CLASSES}/.stamp"
      DEPENDS ${_nve_jvm_sources}
      COMMENT "Compiling the JVM JNI suites"
      VERBATIM)
    add_custom_target(nve_jvm_suites ALL DEPENDS "${NVE_JVM_CLASSES}/.stamp")

    set(_nve_java -Djava.library.path=$<TARGET_FILE_DIR:nve_android_shell> -cp "${NVE_JVM_CLASSES}")
    add_test(NAME android_scene_jni_test
      COMMAND "${Java_JAVA_EXECUTABLE}" ${_nve_java} dev.nve.android.jni.SceneJniTest "${NVE_FIXTURES}"
        $<TARGET_FILE:nve_mock_hot_v1> $<TARGET_FILE:nve_mock_hot_v2> $<TARGET_FILE:nve_mock_bad_iface>)
    set(_nve_event_log "${CMAKE_CURRENT_BINARY_DIR}/mock_events.log")
    add_test(NAME android_events_jni_test
      COMMAND "${Java_JAVA_EXECUTABLE}" ${_nve_java} dev.nve.android.jni.EventsJniTest "${NVE_FIXTURES}"
        $<TARGET_FILE:nve_mock_showcase> "${_nve_event_log}")
    set_tests_properties(android_events_jni_test PROPERTIES ENVIRONMENT "NVE_MOCK_EVENT_LOG=${_nve_event_log}")
    add_test(NAME android_pair_jni_test
      COMMAND "${Java_JAVA_EXECUTABLE}" ${_nve_java} dev.nve.android.jni.PairJniTest "${NVE_FIXTURES}")
    set_tests_properties(android_scene_jni_test android_events_jni_test android_pair_jni_test PROPERTIES TIMEOUT 60)
  endif()
endif()
