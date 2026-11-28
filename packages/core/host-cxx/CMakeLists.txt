cmake_minimum_required(VERSION 3.20)
project(nve_host_cxx LANGUAGES C CXX)

# Header-only C11 + C++20 NVE host runtime: XXH64, little-endian codecs, table/scene readers.
add_library(nve_host_cxx INTERFACE)
add_library(nve::host_cxx ALIAS nve_host_cxx)
target_include_directories(nve_host_cxx INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_compile_features(nve_host_cxx INTERFACE c_std_11 cxx_std_20)

option(NVE_HOST_CXX_BUILD_TESTS "Build the nve_host_cxx test suite" ${PROJECT_IS_TOP_LEVEL})
option(NVE_HOST_CXX_WITH_RUNTIME "Build and run live C++20 RuntimeSession tests linked against libnve_runtime" OFF)
option(NVE_HOST_CXX_BUILD_BENCH "Build nve_host_cxx_bench (SceneReader / TableReader read path; JSON for scripts/bench-regress.py)" OFF)

if(NVE_HOST_CXX_BUILD_BENCH)
  add_executable(nve_host_cxx_bench bench/host_bench.cpp)
  set_target_properties(nve_host_cxx_bench PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF)
  target_link_libraries(nve_host_cxx_bench PRIVATE nve_host_cxx)
  if(MSVC)
    target_compile_options(nve_host_cxx_bench PRIVATE /W4 /WX)
  else()
    target_compile_options(nve_host_cxx_bench PRIVATE -Wall -Wextra -Werror -pedantic -Wconversion -Wsign-conversion -Wshadow)
  endif()
endif()

if(NVE_HOST_CXX_BUILD_TESTS)
  enable_testing()

  if(MSVC)
    set(_nve_host_warnings /W4 /WX)
    set(_nve_host_sanitize "")
  else()
    set(_nve_host_warnings -Wall -Wextra -Werror -pedantic -Wconversion -Wsign-conversion -Wshadow)
    include(CheckCXXSourceCompiles)
    set(CMAKE_REQUIRED_FLAGS "-fsanitize=address,undefined")
    set(CMAKE_REQUIRED_LINK_OPTIONS "-fsanitize=address,undefined")
    check_cxx_source_compiles("int main() { return 0; }" NVE_HOST_HAS_SANITIZERS)
    unset(CMAKE_REQUIRED_FLAGS)
    unset(CMAKE_REQUIRED_LINK_OPTIONS)
    set(_nve_host_sanitize "")
    if(NVE_HOST_HAS_SANITIZERS)
      set(_nve_host_sanitize -fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer)
    endif()
  endif()

  add_executable(nve_host_c11_test tests/test_c11.c)
  set_target_properties(nve_host_c11_test PROPERTIES C_STANDARD 11 C_STANDARD_REQUIRED ON C_EXTENSIONS OFF)

  foreach(_suite xxh64 bin_codec table_scene generated resource_users)
    add_executable(nve_host_${_suite}_test tests/test_${_suite}.cpp)
    set_target_properties(nve_host_${_suite}_test PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF)
  endforeach()

  foreach(_test c11 xxh64 bin_codec table_scene generated resource_users)
    target_link_libraries(nve_host_${_test}_test PRIVATE nve_host_cxx)
    target_compile_options(nve_host_${_test}_test PRIVATE ${_nve_host_warnings} ${_nve_host_sanitize})
    target_link_options(nve_host_${_test}_test PRIVATE ${_nve_host_sanitize})
    add_test(NAME nve_host_${_test} COMMAND nve_host_${_test}_test)
  endforeach()

  # Compile-time guards: invalid `field()` specs and non-UTF-8 `Key64::of` names must not compile.
  if(MSVC)
    set(_nve_host_syntax_only ${CMAKE_CXX_COMPILER} /nologo /std:c++20 /Zs
        /I${CMAKE_CURRENT_SOURCE_DIR}/include ${CMAKE_CURRENT_SOURCE_DIR}/tests/compile_fail.cpp)
    set(_nve_host_def_flag /D)
  else()
    set(_nve_host_syntax_only ${CMAKE_CXX_COMPILER} -std=c++20 -fsyntax-only
        -I${CMAKE_CURRENT_SOURCE_DIR}/include ${CMAKE_CURRENT_SOURCE_DIR}/tests/compile_fail.cpp)
    set(_nve_host_def_flag -D)
  endif()
  add_test(NAME nve_host_compile_ok COMMAND ${_nve_host_syntax_only})
  foreach(_case BAD_FIELD_BITS BAD_FIELD_ZERO BAD_FIELD_WORD BAD_KEY)
    add_test(NAME nve_host_compile_fail_${_case} COMMAND ${_nve_host_syntax_only} ${_nve_host_def_flag}NVE_EXPECT_${_case})
    set_tests_properties(nve_host_compile_fail_${_case} PROPERTIES WILL_FAIL TRUE)
  endforeach()

  # Every public header compiles on its own (includes what it uses), warnings as errors, with Qt's
  # keyword macros defined (`slots`, `signals`, `emit`: Qt hosts include these headers after
  # `<QObject>`). `jni.hpp` needs a JDK's `jni.h` and is covered by the Android host build.
  file(GLOB _nve_host_headers RELATIVE "${CMAKE_CURRENT_SOURCE_DIR}/include"
       "${CMAKE_CURRENT_SOURCE_DIR}/include/nve/host/*.hpp")
  list(REMOVE_ITEM _nve_host_headers nve/host/jni.hpp)
  foreach(_header ${_nve_host_headers})
    string(MAKE_C_IDENTIFIER "${_header}" _header_id)
    set(_header_tu "${CMAKE_CURRENT_BINARY_DIR}/header_checks/${_header_id}.cpp")
    file(WRITE "${_header_tu}" "#include <${_header}>\n")
    if(MSVC)
      set(_header_cmd ${CMAKE_CXX_COMPILER} /nologo /std:c++20 /Zs ${_nve_host_warnings}
          "/Dslots=" "/Dsignals=public" "/Demit=" "/Dforeach=qt_foreach"
          /I${CMAKE_CURRENT_SOURCE_DIR}/include ${_header_tu})
    else()
      set(_header_cmd ${CMAKE_CXX_COMPILER} -std=c++20 -fsyntax-only ${_nve_host_warnings}
          "-Dslots=" "-Dsignals=public" "-Demit=" "-Dforeach=qt_foreach"
          -I${CMAKE_CURRENT_SOURCE_DIR}/include ${_header_tu})
    endif()
    add_test(NAME nve_host_header_${_header_id} COMMAND ${_header_cmd})
  endforeach()

  if(NVE_HOST_CXX_WITH_RUNTIME)
    include("${CMAKE_CURRENT_SOURCE_DIR}/../../../cxx/cmake/NveRuntime.cmake")
    foreach(_suite runtime_session runtime_hooks runtime_scene_builder retained_tree)
      set(_target nve_host_${_suite}_test)
      add_executable(${_target} tests/test_${_suite}.cpp)
      set_target_properties(${_target} PROPERTIES
        CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF)
      target_include_directories(${_target} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/tests/support")
      target_link_libraries(${_target} PRIVATE nve_host_cxx nve::runtime)
      if(NOT NVE_RUNTIME_NO_DEFAULT_FEATURES)
        target_compile_definitions(${_target} PRIVATE NVE_RUNTIME_TEST_HAS_WIRE=1)
      endif()
      target_compile_options(${_target} PRIVATE ${_nve_host_warnings} ${_nve_host_sanitize})
      target_link_options(${_target} PRIVATE ${_nve_host_sanitize})
      add_test(NAME nve_host_${_suite} COMMAND ${_target})
    endforeach()

    # Retained-view host stack (`Nve2Writer`, `RetainedTree`, `HostSession`, app modules).
    include("${CMAKE_CURRENT_SOURCE_DIR}/../../../cxx/cmake/NveHostFixtures.cmake")
    nve_add_app_module_fixtures(nve_host_fixture)
    foreach(_suite runtime session policy events)
      set(_target nve_host_${_suite}_test)
      add_executable(${_target} tests/test_host_${_suite}.cpp)
      set_target_properties(${_target} PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF)
      target_include_directories(${_target} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/tests/support")
      target_link_libraries(${_target} PRIVATE nve_host_cxx nve::runtime)
      nve_use_app_module_fixtures(${_target} nve_host_fixture)
      if(NOT NVE_RUNTIME_NO_DEFAULT_FEATURES)
        target_compile_definitions(${_target} PRIVATE NVE_RUNTIME_TEST_HAS_WIRE=1)
      endif()
      target_compile_options(${_target} PRIVATE ${_nve_host_warnings} ${_nve_host_sanitize})
      target_link_options(${_target} PRIVATE ${_nve_host_sanitize})
      add_test(NAME nve_host_${_suite} COMMAND ${_target})
    endforeach()
  endif()
endif()
