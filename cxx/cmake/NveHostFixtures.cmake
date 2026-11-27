# App `cdylib` fixtures shared by the native host test suites (host-cxx, GTK, Qt, Windows).
#
#   include("${NVE_ROOT}/cxx/cmake/NveHostFixtures.cmake")
#   nve_add_app_module_fixtures(my_prefix)          # -> my_prefix_counter / _counter_double / ...
#   nve_use_app_module_fixtures(my_test my_prefix)  # -> NVE_TEST_COUNTER_MODULE="<path>" / ...
#
# Variants (`tests/fixtures/app_module.cpp`): counter, counter_double (the "rebuilt" counter for
# hot-reload tests), partial_surface, no_surface, no_entry.

include_guard(GLOBAL)

set(_NVE_HOST_FIXTURES_ROOT "${CMAKE_CURRENT_LIST_DIR}/../..")

function(_nve_app_module_fixture_variants out)
  set(${out} counter counter_double partial_surface no_surface no_entry PARENT_SCOPE)
endfunction()

function(nve_add_app_module_fixtures prefix)
  _nve_app_module_fixture_variants(_variants)
  foreach(_variant ${_variants})
    set(_target "${prefix}_${_variant}")
    add_library(${_target} SHARED "${_NVE_HOST_FIXTURES_ROOT}/packages/core/host-cxx/tests/fixtures/app_module.cpp")
    string(TOUPPER "${_variant}" _upper)
    target_compile_definitions(${_target} PRIVATE NVE_BUILDING_LIBRARY=1 NVE_FIXTURE_${_upper}=1)
    target_include_directories(${_target} PRIVATE "${_NVE_HOST_FIXTURES_ROOT}/packages/core/host-cxx/include")
    set_target_properties(${_target} PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED ON CXX_EXTENSIONS OFF
      CXX_VISIBILITY_PRESET hidden)
  endforeach()
endfunction()

# Gives `target` every fixture path as `NVE_TEST_<VARIANT>_MODULE` and builds the fixtures first.
function(nve_use_app_module_fixtures target prefix)
  _nve_app_module_fixture_variants(_variants)
  foreach(_variant ${_variants})
    string(TOUPPER "${_variant}" _upper)
    target_compile_definitions(${target} PRIVATE NVE_TEST_${_upper}_MODULE="$<TARGET_FILE:${prefix}_${_variant}>")
    add_dependencies(${target} ${prefix}_${_variant})
  endforeach()
endfunction()
