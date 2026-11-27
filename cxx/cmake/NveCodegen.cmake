# Shared CMake helper: keeps the generated host ABI headers (`nve codegen abi` ->
# `packages/core/host-cxx/include/nve/generated/`) fresh relative to their inputs (`nve-abi`,
# `nve-components` tables, the codegen crate and its Askama templates), and adds the
# static `nve::host_cxx` runtime target that carries them.
#
# Usage:
#   include("${CMAKE_CURRENT_LIST_DIR}/../cmake/NveCodegen.cmake")
#   nve_ensure_abi_codegen()
#   target_link_libraries(my_target PRIVATE nve::host_cxx)
#
# Opt out (e.g. cross-compile environments without Cargo where headers were pre-generated):
#   -DNVE_SKIP_CODEGEN=ON

include_guard(GLOBAL)

option(NVE_SKIP_CODEGEN "Skip automatic nve-cli ABI codegen during CMake configure" OFF)

function(nve_ensure_abi_codegen)
  get_filename_component(_nve_repo_root "${CMAKE_CURRENT_FUNCTION_LIST_DIR}/../.." ABSOLUTE)
  if(NOT TARGET nve::host_cxx)
    add_subdirectory("${_nve_repo_root}/packages/core/host-cxx" "${CMAKE_BINARY_DIR}/nve_host_cxx")
  endif()

  if(NVE_SKIP_CODEGEN)
    return()
  endif()

  get_property(_nve_codegen_done GLOBAL PROPERTY NVE_ABI_CODEGEN_DONE)
  if(_nve_codegen_done)
    return()
  endif()

  set(_nve_generated_dir "${_nve_repo_root}/packages/core/host-cxx/include/nve/generated")
  set(_nve_generated_headers
    "${_nve_generated_dir}/nve_abi.generated.h"
    "${_nve_generated_dir}/nve_tables.generated.h"
  )

  set(_nve_needs_codegen FALSE)
  set(_nve_reason "")

  foreach(_hdr IN LISTS _nve_generated_headers)
    if(NOT EXISTS "${_hdr}")
      set(_nve_needs_codegen TRUE)
      set(_nve_reason "missing ${_hdr}")
      break()
    endif()
  endforeach()

  if(NOT _nve_needs_codegen)
    file(GLOB_RECURSE _nve_abi_sources
      "${_nve_repo_root}/packages/core/abi/src/*.rs"
      "${_nve_repo_root}/packages/core/abi-macros/src/*.rs"
      "${_nve_repo_root}/packages/core/components/src/tables/*.rs"
      "${_nve_repo_root}/packages/core/codegen/src/*.rs"
      "${_nve_repo_root}/packages/core/codegen/templates/*"
    )
    foreach(_src IN LISTS _nve_abi_sources)
      foreach(_hdr IN LISTS _nve_generated_headers)
        if("${_src}" IS_NEWER_THAN "${_hdr}" AND NOT "${_hdr}" IS_NEWER_THAN "${_src}")
          set(_nve_needs_codegen TRUE)
          set(_nve_reason "${_src} is newer than ${_hdr}")
          break()
        endif()
      endforeach()
      if(_nve_needs_codegen)
        break()
      endif()
    endforeach()
  endif()

  find_program(_nve_cargo_bin cargo
    HINTS
      "$ENV{CARGO_HOME}/bin"
      "$ENV{HOME}/.cargo/bin"
      "$ENV{USERPROFILE}/.cargo/bin"
  )

  if(_nve_needs_codegen)
    if(NOT _nve_cargo_bin)
      message(FATAL_ERROR
        "NVE generated host ABI headers need regeneration (${_nve_reason}), "
        "but 'cargo' was not found. Run 'cargo run --locked -p nve-cli -- codegen abi' "
        "from ${_nve_repo_root} or pass -DNVE_SKIP_CODEGEN=ON."
      )
    endif()

    message(STATUS "NVE codegen: running 'cargo run --locked -p nve-cli -- codegen abi' (${_nve_reason})")
    execute_process(
      COMMAND "${_nve_cargo_bin}" run --locked -p nve-cli -- codegen abi
      WORKING_DIRECTORY "${_nve_repo_root}"
      RESULT_VARIABLE _nve_codegen_rc
    )
    if(NOT _nve_codegen_rc EQUAL 0)
      message(FATAL_ERROR
        "Automatic NVE ABI codegen failed with exit code ${_nve_codegen_rc}. "
        "Run 'cargo run --locked -p nve-cli -- codegen abi' manually to inspect."
      )
    endif()
  endif()

  if(_nve_cargo_bin AND NOT TARGET nve_codegen_abi)
    add_custom_target(nve_codegen_abi
      COMMAND "${_nve_cargo_bin}" run --locked -p nve-cli -- codegen abi
      WORKING_DIRECTORY "${_nve_repo_root}"
      COMMENT "Regenerating NVE host ABI / table / XXH64 vector files"
      VERBATIM
    )
  endif()

  set_property(GLOBAL PROPERTY NVE_ABI_CODEGEN_DONE TRUE)
endfunction()
