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

  set(_nve_rt_is_multiconfig FALSE)
  if(NVE_RUNTIME_PREBUILT_LIB)
    if(NOT EXISTS "${NVE_RUNTIME_PREBUILT_LIB}")
      message(FATAL_ERROR
        "NveRuntime.cmake: NVE_RUNTIME_PREBUILT_LIB does not exist: '${NVE_RUNTIME_PREBUILT_LIB}'"
      )
    endif()
    set(_nve_rt_lib_path "${NVE_RUNTIME_PREBUILT_LIB}")
  else()
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
    if(NVE_RUNTIME_NO_DEFAULT_FEATURES)
      list(APPEND _nve_rt_cargo_args --no-default-features)
    endif()
    if(NVE_RUNTIME_FEATURES)
      # Accept a CMake list (`a;b`) or a comma list; cargo takes one comma-separated argument.
      list(JOIN NVE_RUNTIME_FEATURES "," _nve_rt_features)
      list(APPEND _nve_rt_cargo_args --features "${_nve_rt_features}")
    endif()
    set(_nve_rt_cargo_target "${NVE_RUNTIME_CARGO_TARGET}")
    if("${_nve_rt_cargo_target}" STREQUAL "" AND ANDROID AND DEFINED ANDROID_ABI)
      if(ANDROID_ABI STREQUAL "arm64-v8a")
        set(_nve_rt_cargo_target "aarch64-linux-android")
      elseif(ANDROID_ABI STREQUAL "armeabi-v7a")
        set(_nve_rt_cargo_target "armv7-linux-androideabi")
      elseif(ANDROID_ABI STREQUAL "x86")
        set(_nve_rt_cargo_target "i686-linux-android")
      elseif(ANDROID_ABI STREQUAL "x86_64")
        set(_nve_rt_cargo_target "x86_64-linux-android")
      elseif(ANDROID_ABI STREQUAL "riscv64")
        set(_nve_rt_cargo_target "riscv64-linux-android")
      endif()
    endif()

    set(_nve_rt_cargo_env)
    if(_nve_rt_cargo_target)
      list(APPEND _nve_rt_cargo_args --target "${_nve_rt_cargo_target}")
      set(_nve_rt_target_prefix "${_nve_rt_target_dir}/${_nve_rt_cargo_target}")

      string(REPLACE "-" "_" _nve_rt_triple_lc "${_nve_rt_cargo_target}")
      string(TOLOWER "${_nve_rt_triple_lc}" _nve_rt_triple_lc)
      string(TOUPPER "${_nve_rt_triple_lc}" _nve_rt_triple_up)

      # Derive target-specific CC/AR/Cargo linker from the active CMake toolchain (e.g. Android
      # NDK) when not already provided in the caller's environment.
      set(_nve_rt_cc "")
      set(_nve_rt_linker "")
      set(_nve_rt_ar "")
      set(_nve_rt_used_ndk_wrapper FALSE)
      set(_nve_rt_ndk_bin "")
      if(DEFINED ANDROID_TOOLCHAIN_ROOT AND EXISTS "${ANDROID_TOOLCHAIN_ROOT}/bin")
        set(_nve_rt_ndk_bin "${ANDROID_TOOLCHAIN_ROOT}/bin")
      elseif(DEFINED CMAKE_C_COMPILER AND NOT "${CMAKE_C_COMPILER}" STREQUAL "")
        get_filename_component(_nve_rt_cc_dir "${CMAKE_C_COMPILER}" DIRECTORY)
        if(EXISTS "${_nve_rt_cc_dir}")
          set(_nve_rt_ndk_bin "${_nve_rt_cc_dir}")
        endif()
      endif()

      if(ANDROID OR _nve_rt_cargo_target MATCHES "-android")
        if(_nve_rt_cargo_target STREQUAL "armv7-linux-androideabi")
          set(_nve_rt_ndk_triple "armv7a-linux-androideabi")
        else()
          set(_nve_rt_ndk_triple "${_nve_rt_cargo_target}")
        endif()
        set(_nve_rt_api "")
        if(DEFINED ANDROID_PLATFORM_LEVEL AND NOT "${ANDROID_PLATFORM_LEVEL}" STREQUAL "")
          set(_nve_rt_api "${ANDROID_PLATFORM_LEVEL}")
        elseif(DEFINED ANDROID_NATIVE_API_LEVEL AND NOT "${ANDROID_NATIVE_API_LEVEL}" STREQUAL "")
          set(_nve_rt_api "${ANDROID_NATIVE_API_LEVEL}")
        elseif(DEFINED ANDROID_PLATFORM AND ANDROID_PLATFORM MATCHES "([0-9]+)$")
          set(_nve_rt_api "${CMAKE_MATCH_1}")
        elseif(DEFINED CMAKE_C_COMPILER_TARGET AND CMAKE_C_COMPILER_TARGET MATCHES "([0-9]+)$")
          set(_nve_rt_api "${CMAKE_MATCH_1}")
        endif()
        if(_nve_rt_ndk_bin AND _nve_rt_api)
          set(_nve_rt_wrapper_base "${_nve_rt_ndk_bin}/${_nve_rt_ndk_triple}${_nve_rt_api}-clang")
          if(CMAKE_HOST_WIN32 AND EXISTS "${_nve_rt_wrapper_base}.cmd")
            set(_nve_rt_cc "${_nve_rt_wrapper_base}.cmd")
            set(_nve_rt_linker "${_nve_rt_wrapper_base}.cmd")
            set(_nve_rt_used_ndk_wrapper TRUE)
          elseif(EXISTS "${_nve_rt_wrapper_base}")
            set(_nve_rt_cc "${_nve_rt_wrapper_base}")
            set(_nve_rt_linker "${_nve_rt_wrapper_base}")
            set(_nve_rt_used_ndk_wrapper TRUE)
          endif()
        endif()
      endif()

      if("${_nve_rt_cc}" STREQUAL "" AND DEFINED CMAKE_C_COMPILER AND NOT "${CMAKE_C_COMPILER}" STREQUAL "")
        set(_nve_rt_cc "${CMAKE_C_COMPILER}")
      endif()
      if("${_nve_rt_linker}" STREQUAL "" AND DEFINED CMAKE_C_COMPILER AND NOT "${CMAKE_C_COMPILER}" STREQUAL "")
        set(_nve_rt_linker "${CMAKE_C_COMPILER}")
      endif()
      if(DEFINED CMAKE_AR AND NOT "${CMAKE_AR}" STREQUAL "")
        set(_nve_rt_ar "${CMAKE_AR}")
      elseif(_nve_rt_ndk_bin)
        if(CMAKE_HOST_WIN32 AND EXISTS "${_nve_rt_ndk_bin}/llvm-ar.exe")
          set(_nve_rt_ar "${_nve_rt_ndk_bin}/llvm-ar.exe")
        elseif(EXISTS "${_nve_rt_ndk_bin}/llvm-ar")
          set(_nve_rt_ar "${_nve_rt_ndk_bin}/llvm-ar")
        endif()
      endif()

      if((NOT DEFINED ENV{CC_${_nve_rt_triple_lc}} OR "$ENV{CC_${_nve_rt_triple_lc}}" STREQUAL "") AND _nve_rt_cc)
        list(APPEND _nve_rt_cargo_env "CC_${_nve_rt_triple_lc}=${_nve_rt_cc}")
      endif()
      if(NOT _nve_rt_used_ndk_wrapper
         AND DEFINED CMAKE_C_COMPILER_TARGET AND NOT "${CMAKE_C_COMPILER_TARGET}" STREQUAL ""
         AND (NOT DEFINED ENV{CFLAGS_${_nve_rt_triple_lc}} OR "$ENV{CFLAGS_${_nve_rt_triple_lc}}" STREQUAL ""))
        list(APPEND _nve_rt_cargo_env "CFLAGS_${_nve_rt_triple_lc}=--target=${CMAKE_C_COMPILER_TARGET}")
      endif()
      if((NOT DEFINED ENV{AR_${_nve_rt_triple_lc}} OR "$ENV{AR_${_nve_rt_triple_lc}}" STREQUAL "") AND _nve_rt_ar)
        list(APPEND _nve_rt_cargo_env "AR_${_nve_rt_triple_lc}=${_nve_rt_ar}")
      endif()
      if((NOT DEFINED ENV{CARGO_TARGET_${_nve_rt_triple_up}_LINKER} OR "$ENV{CARGO_TARGET_${_nve_rt_triple_up}_LINKER}" STREQUAL "") AND _nve_rt_linker)
        list(APPEND _nve_rt_cargo_env "CARGO_TARGET_${_nve_rt_triple_up}_LINKER=${_nve_rt_linker}")
      endif()
    else()
      set(_nve_rt_target_prefix "${_nve_rt_target_dir}")
    endif()

    if(_nve_rt_cargo_env)
      set(_nve_rt_cargo_cmd "${CMAKE_COMMAND}" -E env ${_nve_rt_cargo_env} "${_nve_rt_cargo_bin}")
    else()
      set(_nve_rt_cargo_cmd "${_nve_rt_cargo_bin}")
    endif()

    if("${NVE_RUNTIME_PROFILE}" STREQUAL "" AND CMAKE_CONFIGURATION_TYPES)
      # Multi-config generators (Xcode, Visual Studio): derive Cargo profile per `$<CONFIG>`.
      set(_nve_rt_is_multiconfig TRUE)
      set(_nve_rt_debug_path "${_nve_rt_target_prefix}/debug/${_nve_rt_lib_name}")
      set(_nve_rt_release_path "${_nve_rt_target_prefix}/release/${_nve_rt_lib_name}")
      add_custom_target(nve_runtime_cargo_build
        COMMAND ${_nve_rt_cargo_cmd} ${_nve_rt_cargo_args} $<$<NOT:$<CONFIG:Debug>>:--release>
        WORKING_DIRECTORY "${_nve_rt_repo_root}"
        BYPRODUCTS "${_nve_rt_debug_path}" "${_nve_rt_release_path}"
        COMMENT "Building nve-runtime staticlib ($<CONFIG>)"
        COMMAND_EXPAND_LISTS
        VERBATIM
      )
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
      if(_nve_rt_profile STREQUAL "release")
        list(APPEND _nve_rt_cargo_args --release)
      endif()
      set(_nve_rt_lib_path "${_nve_rt_target_prefix}/${_nve_rt_profile}/${_nve_rt_lib_name}")
      add_custom_target(nve_runtime_cargo_build
        COMMAND ${_nve_rt_cargo_cmd} ${_nve_rt_cargo_args}
        WORKING_DIRECTORY "${_nve_rt_repo_root}"
        BYPRODUCTS "${_nve_rt_lib_path}"
        COMMENT "Building nve-runtime staticlib (${_nve_rt_profile})"
        COMMAND_EXPAND_LISTS
        VERBATIM
      )
    endif()
  endif()

  add_library(nve_runtime STATIC IMPORTED GLOBAL)
  add_library(nve::runtime ALIAS nve_runtime)
  if(_nve_rt_is_multiconfig)
    set_target_properties(nve_runtime PROPERTIES
      IMPORTED_CONFIGURATIONS "DEBUG;RELEASE;RELWITHDEBINFO;MINSIZEREL"
      IMPORTED_LOCATION_DEBUG "${_nve_rt_debug_path}"
      IMPORTED_LOCATION_RELEASE "${_nve_rt_release_path}"
      IMPORTED_LOCATION_RELWITHDEBINFO "${_nve_rt_release_path}"
      IMPORTED_LOCATION_MINSIZEREL "${_nve_rt_release_path}"
      INTERFACE_INCLUDE_DIRECTORIES "${_nve_rt_repo_root}/packages/core/host-cxx/include"
    )
  else()
    set_target_properties(nve_runtime PROPERTIES
      IMPORTED_LOCATION "${_nve_rt_lib_path}"
      INTERFACE_INCLUDE_DIRECTORIES "${_nve_rt_repo_root}/packages/core/host-cxx/include"
    )
  endif()
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
