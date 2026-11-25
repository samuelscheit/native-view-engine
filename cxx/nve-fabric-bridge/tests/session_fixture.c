#include "nve_abi.generated.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(NVE_FIXTURE_NO_ENTRY)

NVE_API int nve_session_fixture_dummy(void) {
  return 7;
}

#elif defined(NVE_FIXTURE_NO_SURFACE)

NVE_API const void* nve_get_interface(uint32_t interface_id) {
  (void)interface_id;
  return NULL;
}

#else

typedef struct SessionFixtureApp {
  uint32_t surface_id;
  int32_t count;
  uint8_t cmd_buf[512];
  size_t cmd_len;
  uint8_t ev_buf[256];
  size_t ev_len;
  char state_buf[32];
  char label_prefix[32];
} SessionFixtureApp;

static void push_bytes(uint8_t* dst, size_t cap, size_t* len, const void* src, size_t n) {
  if (*len + n <= cap) {
    memcpy(dst + *len, src, n);
    *len += n;
  }
}

static void push_u8(SessionFixtureApp* app, uint8_t v) {
  push_bytes(app->cmd_buf, sizeof(app->cmd_buf), &app->cmd_len, &v, sizeof(v));
}
static void push_u16(SessionFixtureApp* app, uint16_t v) {
  push_bytes(app->cmd_buf, sizeof(app->cmd_buf), &app->cmd_len, &v, sizeof(v));
}
static void push_u32(SessionFixtureApp* app, uint32_t v) {
  push_bytes(app->cmd_buf, sizeof(app->cmd_buf), &app->cmd_len, &v, sizeof(v));
}
static void push_u64(SessionFixtureApp* app, uint64_t v) {
  push_bytes(app->cmd_buf, sizeof(app->cmd_buf), &app->cmd_len, &v, sizeof(v));
}

static NveAppHandle fixture_create(uint32_t surface_id) {
  SessionFixtureApp* app = (SessionFixtureApp*)calloc(1, sizeof(SessionFixtureApp));
  if (!app) {
    return NULL;
  }
  app->surface_id = surface_id;
  app->count = 0;
  snprintf(app->label_prefix, sizeof(app->label_prefix), "Fixture");
  return app;
}

static void fixture_destroy(NveAppHandle handle) {
  free(handle);
}

static NveBufferSlice fixture_render(NveAppHandle handle, uint64_t frame_id) {
  SessionFixtureApp* app = (SessionFixtureApp*)handle;
  NveBufferSlice empty = {NULL, 0};
  if (!app) {
    return empty;
  }
  app->cmd_len = 0;

  char text_buf[64];
  const int text_n =
      snprintf(text_buf, sizeof(text_buf), "%s:%d", app->label_prefix, (int)app->count);
  const uint32_t text_len = text_n > 0 ? (uint32_t)text_n : 0u;

  /* Header (24 bytes): magic, version, flags, surface_id, frame_id, op_count=6 */
  push_u32(app, NVE_ABI_MAGIC);
  push_u16(app, (uint16_t)NVE_ABI_VERSION);
  push_u16(app, 0u);
  push_u32(app, app->surface_id);
  push_u64(app, frame_id);
  push_u32(app, 6u);

  /* Op 1: CREATE_NODE 2 (PRESSABLE) */
  push_u8(app, (uint8_t)NVE_OP_CREATE_NODE);
  push_u32(app, 2u);
  push_u16(app, (uint16_t)NVE_COMP_PRESSABLE);
  push_u16(app, 0u);

  /* Op 2: CREATE_NODE 3 (TEXT) */
  push_u8(app, (uint8_t)NVE_OP_CREATE_NODE);
  push_u32(app, 3u);
  push_u16(app, (uint16_t)NVE_COMP_TEXT);
  push_u16(app, 0u);

  /* Op 3: SET_CHILDREN root -> [2] */
  push_u8(app, (uint8_t)NVE_OP_SET_CHILDREN);
  push_u32(app, NVE_ROOT_NODE_ID);
  push_u32(app, 1u);
  push_u32(app, 2u);

  /* Op 4: SET_CHILDREN 2 -> [3] */
  push_u8(app, (uint8_t)NVE_OP_SET_CHILDREN);
  push_u32(app, 2u);
  push_u32(app, 1u);
  push_u32(app, 3u);

  /* Op 5: SET_TEXT 3 */
  push_u8(app, (uint8_t)NVE_OP_SET_TEXT);
  push_u32(app, 3u);
  push_u32(app, text_len);
  push_bytes(app->cmd_buf, sizeof(app->cmd_buf), &app->cmd_len, text_buf, text_len);

  /* Op 6: COMMIT sync=1 */
  push_u8(app, (uint8_t)NVE_OP_COMMIT);
  push_u8(app, 1u);

  NveBufferSlice out = {app->cmd_buf, app->cmd_len};
  return out;
}

static uint32_t fixture_dispatch_events(NveAppHandle handle, NveBufferSlice events) {
  SessionFixtureApp* app = (SessionFixtureApp*)handle;
  if (!app || !events.ptr || events.len < 4) {
    return 0u;
  }
  uint32_t count = 0;
  memcpy(&count, events.ptr, sizeof(count));
  if (count == 0) {
    return 0u;
  }
  app->count += 1;
  return 1u;
}

static uint32_t fixture_set_props(NveAppHandle handle, NveBufferSlice props) {
  SessionFixtureApp* app = (SessionFixtureApp*)handle;
  if (!app || !props.ptr || props.len == 0) {
    return 0u;
  }
  if (props.len >= 4 && memcmp(props.ptr, "{\"v\"", 4) == 0) {
    snprintf(app->label_prefix, sizeof(app->label_prefix), "Updated");
    return 1u;
  }
  return 0u;
}

static NveBufferSlice fixture_drain_events(NveAppHandle handle) {
  (void)handle;
  NveBufferSlice empty = {NULL, 0};
  return empty;
}

static uint32_t fixture_apply_hot_patch(NveAppHandle handle, NveBufferSlice patch_json) {
  (void)handle;
  (void)patch_json;
  return 0u;
}

static NveBufferSlice fixture_snapshot_state(NveAppHandle handle) {
  SessionFixtureApp* app = (SessionFixtureApp*)handle;
  NveBufferSlice empty = {NULL, 0};
  if (!app) {
    return empty;
  }
  const int n = snprintf(app->state_buf, sizeof(app->state_buf), "%d", (int)app->count);
  NveBufferSlice out = {(const uint8_t*)app->state_buf, n > 0 ? (size_t)n : 0u};
  return out;
}

static void fixture_restore_state(NveAppHandle handle, NveBufferSlice state) {
  SessionFixtureApp* app = (SessionFixtureApp*)handle;
  if (!app || !state.ptr || state.len == 0 || state.len >= sizeof(app->state_buf)) {
    return;
  }
  char tmp[32];
  memcpy(tmp, state.ptr, state.len);
  tmp[state.len] = '\0';
  app->count = (int32_t)atoi(tmp);
}

static uint32_t fixture_dispatch_events_direct(NveAppHandle handle, NveEventBatch batch) {
  SessionFixtureApp* app = (SessionFixtureApp*)handle;
  if (!app || !batch.ptr || batch.len == 0) {
    return 0u;
  }
  app->count += (int32_t)batch.len;
  return 1u;
}

static uint32_t fixture_set_props_direct(
    NveAppHandle handle,
    const NveMapEntry* entries,
    uint32_t count) {
  SessionFixtureApp* app = (SessionFixtureApp*)handle;
  if (!app || !entries || count == 0) {
    return 0u;
  }
  for (uint32_t i = 0; i < count; ++i) {
    if (entries[i].key.len == 1 && entries[i].key.ptr && entries[i].key.ptr[0] == 'v') {
      snprintf(app->label_prefix, sizeof(app->label_prefix), "Updated");
      return 1u;
    }
  }
  return 0u;
}

static const NveSurfaceApi kSurfaceApi = {
    fixture_create,
    fixture_destroy,
    fixture_render,
    fixture_dispatch_events,
    fixture_dispatch_events_direct,
};

static const NveComponentApi kComponentApi = {
    fixture_set_props,
    fixture_drain_events,
    fixture_set_props_direct,
};

static const NveDevApi kDevApi = {
    fixture_apply_hot_patch,
    fixture_snapshot_state,
    fixture_restore_state,
};

NVE_API const void* nve_get_interface(uint32_t interface_id) {
  switch (interface_id) {
    case NVE_IFACE_SURFACE:
      return &kSurfaceApi;
    case NVE_IFACE_COMPONENT:
      return &kComponentApi;
    case NVE_IFACE_DEV:
      return &kDevApi;
    default:
      return NULL;
  }
}

#endif
