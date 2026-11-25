#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#ifndef NVE_API
#if defined(NVE_BUILD_SHARED_LIB)
#if defined(_WIN32)
#define NVE_API __declspec(dllexport)
#elif defined(__GNUC__) || defined(__clang__)
#define NVE_API __attribute__((visibility("default")))
#else
#define NVE_API
#endif
#else
#define NVE_API
#endif
#endif

#define NVE_ABI_MAGIC 0x3145564Eu /* "NVE1" little-endian */
#define NVE_ABI_VERSION 1u
#define NVE_ROOT_NODE_ID 1u

/* Modular interface IDs resolved once via `nve_get_interface(uint32_t)`. */
#define NVE_IFACE_SURFACE 1u   /* const NveSurfaceApi*   (required) */
#define NVE_IFACE_LAYOUT 2u    /* const NveLayoutApi*    (optional) */
#define NVE_IFACE_COMPONENT 3u /* const NveComponentApi* (optional) */
#define NVE_IFACE_DEV 4u       /* const NveDevApi*       (optional; NULL in release) */

typedef enum NveOpcode {
  NVE_OP_CREATE_NODE = 1,
  NVE_OP_REMOVE_NODE = 2,
  NVE_OP_SET_CHILDREN = 3,
  NVE_OP_SET_TEXT = 4,
  NVE_OP_SET_LAYOUT = 5,
  NVE_OP_SET_STYLE = 6,
  NVE_OP_SET_RAW_PROP = 7,
  NVE_OP_SET_EVENT_MASK = 8,
  NVE_OP_DISPATCH_COMMAND = 9,
  NVE_OP_COMMIT = 10,
} NveOpcode;

typedef enum NveComponentKind {
  NVE_COMP_ROOT = 0,
  NVE_COMP_VIEW = 1,
  NVE_COMP_TEXT = 2,
  NVE_COMP_SCROLL_VIEW = 3,
  NVE_COMP_IMAGE = 4,
  NVE_COMP_TEXT_INPUT = 5,
  NVE_COMP_PRESSABLE = 6,
  NVE_COMP_SKIA_CANVAS = 7,
  NVE_COMP_CUSTOM = 0xFFFF,
} NveComponentKind;

typedef enum NveGestureDisposition {
  NVE_GESTURE_CLAIM = 1,
  NVE_GESTURE_PASS_THROUGH = 2,
  NVE_GESTURE_DEFER_TO_PARENT = 3,
} NveGestureDisposition;

typedef struct NveBufferSlice {
  const uint8_t* ptr;
  size_t len;
} NveBufferSlice;

typedef struct NveSurfaceConfig {
  uint32_t surface_id;
  float width;
  float height;
  float scale_factor;
} NveSurfaceConfig;

typedef struct NveFrameStats {
  uint32_t op_count;
  uint32_t mutation_count;
  uint64_t decode_us;
  uint64_t layout_us;
  uint64_t diff_us;
  uint64_t mount_us;
} NveFrameStats;

typedef struct NveSizeConstraints {
  float min_w;
  float min_h;
  float max_w;
  float max_h;
} NveSizeConstraints;

typedef struct NveIntrinsicSize {
  float width;
  float height;
} NveIntrinsicSize;

typedef void* NveAppHandle;
typedef void* NveFabricHostHandle;

/* Required core surface interface (`NVE_IFACE_SURFACE = 1`). */
typedef struct NveSurfaceApi {
  NveAppHandle (*create)(uint32_t surface_id);
  void (*destroy)(NveAppHandle handle);
  NveBufferSlice (*render)(NveAppHandle handle, uint64_t frame_id);
  uint32_t (*dispatch_events)(NveAppHandle handle, NveBufferSlice events);
} NveSurfaceApi;

/* Optional layout & gesture interface (`NVE_IFACE_LAYOUT = 2`). Zero-serialization register calls. */
typedef struct NveLayoutApi {
  NveIntrinsicSize (*measure)(NveAppHandle handle, NveSizeConstraints constraints);
  uint8_t (*hit_test)(NveAppHandle handle, float x, float y, uint8_t pointer_phase);
} NveLayoutApi;

/* Optional embedded component props & outbound event interface (`NVE_IFACE_COMPONENT = 3`). */
typedef struct NveComponentApi {
  uint32_t (*set_props)(NveAppHandle handle, NveBufferSlice props);
  NveBufferSlice (*drain_emitted_events)(NveAppHandle handle);
} NveComponentApi;

/* Optional dev / 2-tier hot-reload interface (`NVE_IFACE_DEV = 4`, may be NULL in release builds). */
typedef struct NveDevApi {
  uint32_t (*apply_hot_patch)(NveAppHandle handle, NveBufferSlice patch_json);
  NveBufferSlice (*snapshot_state)(NveAppHandle handle);
  void (*restore_state)(NveAppHandle handle, NveBufferSlice state);
} NveDevApi;

/* Single exported symbol per NVE module (`nve_get_interface`). */
typedef const void* (*NveGetInterfaceFn)(uint32_t interface_id);
NVE_API const void* nve_get_interface(uint32_t interface_id);

/* Host-side resolved cache of modular interface pointers (not an ABI struct). */
typedef struct NveModule {
  const NveSurfaceApi* surface;
  const NveLayoutApi* layout;
  const NveComponentApi* component;
  const NveDevApi* dev;
} NveModule;

static inline NveModule nve_resolve_module(NveGetInterfaceFn get_iface) {
  NveModule m = {NULL, NULL, NULL, NULL};
  if (!get_iface) {
    return m;
  }
  m.surface = (const NveSurfaceApi*)get_iface(NVE_IFACE_SURFACE);
  m.layout = (const NveLayoutApi*)get_iface(NVE_IFACE_LAYOUT);
  m.component = (const NveComponentApi*)get_iface(NVE_IFACE_COMPONENT);
  m.dev = (const NveDevApi*)get_iface(NVE_IFACE_DEV);
  return m;
}

/* C API implemented by cxx/nve-fabric-bridge for iOS, macOS, Android, and headless Fabric tests. */
NveFabricHostHandle nve_fabric_host_create(NveSurfaceConfig config);
void nve_fabric_host_destroy(NveFabricHostHandle host);
int32_t nve_fabric_host_submit_frame(
    NveFabricHostHandle host,
    NveBufferSlice command_buffer,
    NveFrameStats* out_stats);
NveBufferSlice nve_fabric_host_drain_events(NveFabricHostHandle host);
uint32_t nve_fabric_host_node_count(NveFabricHostHandle host);

#ifdef __cplusplus
}
#endif
