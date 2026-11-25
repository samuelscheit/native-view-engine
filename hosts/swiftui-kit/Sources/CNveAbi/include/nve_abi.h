#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NVE_ABI_MAGIC 0x3145564Eu /* "NVE1" little-endian */
#define NVE_ABI_VERSION 1u
#define NVE_ROOT_NODE_ID 1u

#define NVE_IFACE_SURFACE 1u
#define NVE_IFACE_LAYOUT 2u
#define NVE_IFACE_COMPONENT 3u
#define NVE_IFACE_DEV 4u

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

typedef struct NveSurfaceApi {
  NveAppHandle (*create)(uint32_t surface_id);
  void (*destroy)(NveAppHandle handle);
  NveBufferSlice (*render)(NveAppHandle handle, uint64_t frame_id);
  uint32_t (*dispatch_events)(NveAppHandle handle, NveBufferSlice events);
} NveSurfaceApi;

typedef struct NveLayoutApi {
  NveIntrinsicSize (*measure)(NveAppHandle handle, NveSizeConstraints constraints);
  uint8_t (*hit_test)(NveAppHandle handle, float x, float y, uint8_t pointer_phase);
} NveLayoutApi;

typedef struct NveComponentApi {
  uint32_t (*set_props)(NveAppHandle handle, NveBufferSlice props);
  NveBufferSlice (*drain_emitted_events)(NveAppHandle handle);
} NveComponentApi;

typedef struct NveDevApi {
  uint32_t (*apply_hot_patch)(NveAppHandle handle, NveBufferSlice patch_json);
  NveBufferSlice (*snapshot_state)(NveAppHandle handle);
  void (*restore_state)(NveAppHandle handle, NveBufferSlice state);
} NveDevApi;

typedef const void* (*NveGetInterfaceFn)(uint32_t interface_id);

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

#ifdef __cplusplus
}
#endif
