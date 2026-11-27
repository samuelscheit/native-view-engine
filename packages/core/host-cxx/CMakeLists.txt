cmake_minimum_required(VERSION 3.20)
project(nve_host_cxx LANGUAGES C CXX)

# Header-only C11 + C++20 NVE host runtime: XXH64, little-endian codecs, table/scene readers.
add_library(nve_host_cxx INTERFACE)
add_library(nve::host_cxx ALIAS nve_host_cxx)
target_include_directories(nve_host_cxx INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_compile_features(nve_host_cxx INTERFACE c_std_11 cxx_std_20)

option(NVE_HOST_CXX_BUILD_TESTS "Build the nve_host_cxx test suite" ${PROJECT_IS_TOP_LEVEL})
option(NVE_HOST_CXX_WITH_RUNTIME "Build and run live C++20 RuntimeSession tests linked against libnve_runtime" OFF)

if(NVE_HOST_CXX_BUILD_TESTS)
  enable_testing()

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

  add_executable(nve_host_c11_test tests/test_c11.c)
  set_target_properties(nve_host_c11_test PROPERTIES C_STANDARD 11 C_STANDARD_REQUIRED ON C_EXTENSIONS OFF)

  foreach(_suite xxh64 bin_codec table_scene)
    add_executable(nve_host_${_suite}_test tests/test_${_suite}.cpp)
    set_target_properties(nve_host_${_suite}_test PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF)
  endforeach()

  foreach(_test c11 xxh64 bin_codec table_scene)
    target_link_libraries(nve_host_${_test}_test PRIVATE nve_host_cxx)
    target_compile_options(nve_host_${_test}_test PRIVATE ${_nve_host_warnings} ${_nve_host_sanitize})
    target_link_options(nve_host_${_test}_test PRIVATE ${_nve_host_sanitize})
    add_test(NAME nve_host_${_test} COMMAND nve_host_${_test}_test)
  endforeach()

  # Compile-time guards: invalid `field()` specs and non-UTF-8 `Key64::of` names must not compile.
  set(_nve_host_syntax_only ${CMAKE_CXX_COMPILER} -std=c++20 -fsyntax-only
      -I${CMAKE_CURRENT_SOURCE_DIR}/include ${CMAKE_CURRENT_SOURCE_DIR}/tests/compile_fail.cpp)
  add_test(NAME nve_host_compile_ok COMMAND ${_nve_host_syntax_only})
  foreach(_case BAD_FIELD_BITS BAD_FIELD_ZERO BAD_FIELD_WORD BAD_KEY)
    add_test(NAME nve_host_compile_fail_${_case} COMMAND ${_nve_host_syntax_only} -DNVE_EXPECT_${_case})
    set_tests_properties(nve_host_compile_fail_${_case} PROPERTIES WILL_FAIL TRUE)
  endforeach()

  if(NVE_HOST_CXX_WITH_RUNTIME)
    include("${CMAKE_CURRENT_SOURCE_DIR}/../../../cxx/cmake/NveRuntime.cmake")
    add_executable(nve_host_runtime_session_test tests/test_runtime_session.cpp)
    set_target_properties(nve_host_runtime_session_test PROPERTIES
      CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF)
    target_link_libraries(nve_host_runtime_session_test PRIVATE nve_host_cxx nve::runtime)
    target_compile_options(nve_host_runtime_session_test PRIVATE ${_nve_host_warnings} ${_nve_host_sanitize})
    target_link_options(nve_host_runtime_session_test PRIVATE ${_nve_host_sanitize})
    add_test(NAME nve_host_runtime_session COMMAND nve_host_runtime_session_test)
  endif()
endif()
