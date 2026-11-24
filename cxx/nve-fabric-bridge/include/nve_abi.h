#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NVE_ABI_MAGIC 0x3145564Eu /* "NVE1" little-endian */
#define NVE_ABI_VERSION 1u
#define NVE_APP_ABI_VERSION 1u
#define NVE_ROOT_NODE_ID 1u

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

typedef void* NveAppHandle;
typedef void* NveFabricHostHandle;

/* Exported by every Tier-2 hot-swappable Rust app cdylib as `nve_app_vtable`. */
typedef struct NveAppVTable {
  uint32_t abi_version;
  NveAppHandle (*create)(uint32_t surface_id);
  void (*destroy)(NveAppHandle handle);
  NveBufferSlice (*render)(NveAppHandle handle, uint64_t frame_id);
  uint32_t (*dispatch_events)(NveAppHandle handle, NveBufferSlice events);
  uint32_t (*apply_hot_patch)(NveAppHandle handle, NveBufferSlice patch_json);
  NveBufferSlice (*snapshot_state)(NveAppHandle handle);
  void (*restore_state)(NveAppHandle handle, NveBufferSlice state);
  uint32_t (*set_props)(NveAppHandle handle, NveBufferSlice props);
  NveBufferSlice (*drain_emitted_events)(NveAppHandle handle);
} NveAppVTable;

/* C API implemented by cxx/nve-fabric-bridge for iOS, Android, and headless Fabric tests. */
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
