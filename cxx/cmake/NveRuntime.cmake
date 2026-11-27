# Shared CMake helper exposing `nve::runtime` (`libnve_runtime`) to native C/C++/Apple/Android/Win32
# hosts via the single-entry `nve_runtime_get_interface` C ABI.
#
# Usage:
#   include("${CMAKE_CURRENT_LIST_DIR}/NveRuntime.cmake")
#   target_link_libraries(my_host PRIVATE nve::host_cxx nve::runtime)
#
# Configuration knobs:
#   -DNVE_RUNTIME_PREBUILT_LIB=/path/to/libnve_runtime.a   Use a prebuilt staticlib (skips Cargo)
#   -DNVE_RUNTIME_PROFILE=debug|release                    Cargo profile (defaults from CMAKE_BUILD_TYPE)
#   -DNVE_RUNTIME_NO_DEFAULT_FEATURES=ON                   Omit default embedded `wire` feature
#   -DNVE_RUNTIME_FEATURES="feat1,feat2"                   Optional extra Cargo features
#   -DNVE_RUNTIME_CARGO_TARGET=<triple>                    Optional Cargo cross-compile target

include_guard(GLOBAL)

include("${CMAKE_CURRENT_LIST_DIR}/NveCodegen.cmake")
nve_ensure_abi_codegen()

if(NOT TARGET nve_runtime)
  get_filename_component(_nve_rt_repo_root "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)

  set(NVE_RUNTIME_PREBUILT_LIB "" CACHE FILEPATH "Optional prebuilt libnve_runtime static library path")
  set(NVE_RUNTIME_PROFILE "" CACHE STRING "Cargo profile for building nve-runtime (debug or release; empty derives from CMAKE_BUILD_TYPE)")
  option(NVE_RUNTIME_NO_DEFAULT_FEATURES "Build nve-runtime with --no-default-features (disables embedded wire pairing)" OFF)
  set(NVE_RUNTIME_FEATURES "" CACHE STRING "Optional comma-separated extra Cargo features for nve-runtime")
  set(NVE_RUNTIME_CARGO_TARGET "" CACHE STRING "Optional Rust target triple for cross-compiling nve-runtime")

  if(WIN32 AND MSVC)
    set(_nve_rt_lib_name "nve_runtime.lib")
  else()
    set(_nve_rt_lib_name "libnve_runtime.a")
  endif()

  if(NVE_RUNTIME_PREBUILT_LIB)
    if(NOT EXISTS "${NVE_RUNTIME_PREBUILT_LIB}")
      message(FATAL_ERROR
        "NveRuntime.cmake: NVE_RUNTIME_PREBUILT_LIB does not exist: '${NVE_RUNTIME_PREBUILT_LIB}'"
      )
    endif()
    set(_nve_rt_lib_path "${NVE_RUNTIME_PREBUILT_LIB}")
  else()
    if("${NVE_RUNTIME_PROFILE}" STREQUAL "")
      string(TOLOWER "${CMAKE_BUILD_TYPE}" _nve_rt_build_type)
      if(_nve_rt_build_type STREQUAL "release"
         OR _nve_rt_build_type STREQUAL "relwithdebinfo"
         OR _nve_rt_build_type STREQUAL "minsizerel")
        set(_nve_rt_profile "release")
      else()
        set(_nve_rt_profile "debug")
      endif()
    else()
      string(TOLOWER "${NVE_RUNTIME_PROFILE}" _nve_rt_profile)
    endif()
    if(NOT _nve_rt_profile STREQUAL "debug" AND NOT _nve_rt_profile STREQUAL "release")
      message(FATAL_ERROR
        "NveRuntime.cmake: invalid NVE_RUNTIME_PROFILE='${NVE_RUNTIME_PROFILE}' (expected 'debug' or 'release')"
      )
    endif()

    find_program(_nve_rt_cargo_bin cargo
      HINTS
        "$ENV{CARGO_HOME}/bin"
        "$ENV{HOME}/.cargo/bin"
        "$ENV{USERPROFILE}/.cargo/bin"
    )
    if(NOT _nve_rt_cargo_bin)
      message(FATAL_ERROR
        "NveRuntime.cmake: 'cargo' was not found. Install Rust/Cargo or pass "
        "-DNVE_RUNTIME_PREBUILT_LIB=/path/to/${_nve_rt_lib_name}."
      )
    endif()

    if(DEFINED ENV{CARGO_TARGET_DIR} AND NOT "$ENV{CARGO_TARGET_DIR}" STREQUAL "")
      set(_nve_rt_target_dir "$ENV{CARGO_TARGET_DIR}")
    else()
      set(_nve_rt_target_dir "${_nve_rt_repo_root}/target")
    endif()

    set(_nve_rt_cargo_args rustc --locked -p nve-runtime --lib --crate-type staticlib)
    if(_nve_rt_profile STREQUAL "release")
      list(APPEND _nve_rt_cargo_args --release)
    endif()
    if(NVE_RUNTIME_NO_DEFAULT_FEATURES)
      list(APPEND _nve_rt_cargo_args --no-default-features)
    endif()
    if(NVE_RUNTIME_FEATURES)
      list(APPEND _nve_rt_cargo_args --features "${NVE_RUNTIME_FEATURES}")
    endif()
    if(NVE_RUNTIME_CARGO_TARGET)
      list(APPEND _nve_rt_cargo_args --target "${NVE_RUNTIME_CARGO_TARGET}")
      set(_nve_rt_lib_path "${_nve_rt_target_dir}/${NVE_RUNTIME_CARGO_TARGET}/${_nve_rt_profile}/${_nve_rt_lib_name}")
    else()
      set(_nve_rt_lib_path "${_nve_rt_target_dir}/${_nve_rt_profile}/${_nve_rt_lib_name}")
    endif()

    add_custom_target(nve_runtime_cargo_build
      COMMAND "${_nve_rt_cargo_bin}" ${_nve_rt_cargo_args}
      WORKING_DIRECTORY "${_nve_rt_repo_root}"
      BYPRODUCTS "${_nve_rt_lib_path}"
      COMMENT "Building nve-runtime staticlib (${_nve_rt_profile})"
      VERBATIM
    )
  endif()

  add_library(nve_runtime STATIC IMPORTED GLOBAL)
  add_library(nve::runtime ALIAS nve_runtime)
  set_target_properties(nve_runtime PROPERTIES
    IMPORTED_LOCATION "${_nve_rt_lib_path}"
    INTERFACE_INCLUDE_DIRECTORIES "${_nve_rt_repo_root}/packages/core/host-cxx/include"
  )
  if(TARGET nve_runtime_cargo_build)
    add_dependencies(nve_runtime nve_runtime_cargo_build)
  endif()

  # Per-platform native static libs matching `cargo rustc -p nve-runtime --lib --crate-type staticlib -- --print native-static-libs`.
  if(APPLE)
    set_property(TARGET nve_runtime APPEND PROPERTY INTERFACE_LINK_LIBRARIES
      "-framework Security"
      "-framework CoreFoundation"
      iconv
    )
  elseif(ANDROID)
    # Android NDK integrates pthreads into bionic libc; passing `-lpthread` fails linking.
    set_property(TARGET nve_runtime APPEND PROPERTY INTERFACE_LINK_LIBRARIES
      log
      dl
      m
    )
  elseif(WIN32)
    set_property(TARGET nve_runtime APPEND PROPERTY INTERFACE_LINK_LIBRARIES
      ws2_32
      bcrypt
      userenv
      ntdll
      crypt32
      secur32
      ncrypt
      advapi32
    )
  elseif(UNIX)
    set_property(TARGET nve_runtime APPEND PROPERTY INTERFACE_LINK_LIBRARIES
      pthread
      dl
      m
    )
  endif()
endif()
