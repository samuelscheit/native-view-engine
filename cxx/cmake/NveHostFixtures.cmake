# App `cdylib` fixtures shared by the native host test suites (host-cxx, GTK, Qt, Windows).
#
#   include("${NVE_ROOT}/cxx/cmake/NveHostFixtures.cmake")
#   nve_add_app_module_fixtures(my_prefix)   # -> my_prefix_counter / _no_surface / _no_entry
#   target_compile_definitions(my_test PRIVATE
#     NVE_TEST_COUNTER_MODULE="$<TARGET_FILE:my_prefix_counter>" ...)

include_guard(GLOBAL)

set(_NVE_HOST_FIXTURES_ROOT "${CMAKE_CURRENT_LIST_DIR}/../..")

function(nve_add_app_module_fixtures prefix)
  foreach(_variant counter no_surface no_entry)
    set(_target "${prefix}_${_variant}")
    add_library(${_target} SHARED "${_NVE_HOST_FIXTURES_ROOT}/packages/core/host-cxx/tests/fixtures/app_module.cpp")
    string(TOUPPER "${_variant}" _upper)
    target_compile_definitions(${_target} PRIVATE NVE_BUILD_SHARED_LIB=1 NVE_FIXTURE_${_upper}=1)
    target_include_directories(${_target} PRIVATE "${_NVE_HOST_FIXTURES_ROOT}/packages/core/host-cxx/include")
    set_target_properties(${_target} PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF
      CXX_VISIBILITY_PRESET hidden)
  endforeach()
endfunction()
