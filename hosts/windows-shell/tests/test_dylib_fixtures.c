#include "nve_abi.generated.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(NVE_FIXTURE_NO_ENTRY)

NVE_API int nve_win_fixture_dummy_symbol(void) {
  return 42;
}

#elif defined(NVE_FIXTURE_NO_SURFACE)

NVE_API const void* nve_get_interface(uint32_t interface_id) {
  (void)interface_id;
  return NULL;
}

#elif defined(NVE_FIXTURE_STATEFUL)

typedef struct WinStatefulFixtureApp {
  uint32_t surface_id;
  int32_t counter;
  char prefix[32];
  uint8_t cmd_buf[512];
  size_t cmd_len;
  int32_t snapshot_word;
} WinStatefulFixtureApp;

static void push_bytes(WinStatefulFixtureApp* app, const void* src, size_t n) {
  if (app->cmd_len + n <= sizeof(app->cmd_buf)) {
    memcpy(app->cmd_buf + app->cmd_len, src, n);
    app->cmd_len += n;
  }
}

static void push_u8(WinStatefulFixtureApp* app, uint8_t v) { push_bytes(app, &v, sizeof(v)); }
static void push_u16(WinStatefulFixtureApp* app, uint16_t v) { push_bytes(app, &v, sizeof(v)); }
static void push_u32(WinStatefulFixtureApp* app, uint32_t v) { push_bytes(app, &v, sizeof(v)); }
static void push_u64(WinStatefulFixtureApp* app, uint64_t v) { push_bytes(app, &v, sizeof(v)); }

static NveAppHandle fixture_create(uint32_t surface_id) {
  WinStatefulFixtureApp* app = (WinStatefulFixtureApp*)calloc(1, sizeof(WinStatefulFixtureApp));
  if (app != NULL) {
    app->surface_id = surface_id;
    app->counter = 0;
    snprintf(app->prefix, sizeof(app->prefix), "state");
  }
  return app;
}

static void fixture_destroy(NveAppHandle h) {
  free(h);
}

static NveBufferSlice fixture_render(NveAppHandle h, uint64_t frame_id) {
  WinStatefulFixtureApp* app = (WinStatefulFixtureApp*)h;
  app->cmd_len = 0;
  uint32_t ops = 0;

  push_u32(app, NVE_ABI_MAGIC);
  push_u16(app, (uint16_t)NVE_ABI_VERSION);
  push_u16(app, 0u);
  push_u32(app, app->surface_id);
  push_u64(app, frame_id);
  push_u32(app, 0u);

  /* Node 2: leaf text, Node 3: pressable */
  push_u8(app, (uint8_t)NVE_OP_CREATE_NODE);
  push_u32(app, 2u);
  push_u16(app, (uint16_t)NVE_COMP_TEXT);
  push_u16(app, 0u);
  ++ops;

  push_u8(app, (uint8_t)NVE_OP_CREATE_NODE);
  push_u32(app, 3u);
  push_u16(app, (uint16_t)NVE_COMP_PRESSABLE);
  push_u16(app, 0u);
  ++ops;

  push_u8(app, (uint8_t)NVE_OP_SET_CHILDREN);
  push_u32(app, NVE_ROOT_NODE_ID);
  push_u32(app, 2u);
  push_u32(app, 2u);
  push_u32(app, 3u);
  ++ops;

  char label[64];
  const int label_len = snprintf(label, sizeof(label), "%s=%d", app->prefix, (int)app->counter);
  push_u8(app, (uint8_t)NVE_OP_SET_TEXT);
  push_u32(app, 2u);
  push_u32(app, (uint32_t)(label_len > 0 ? label_len : 0));
  if (label_len > 0) {
    push_bytes(app, label, (size_t)label_len);
  }
  ++ops;

  push_u8(app, (uint8_t)NVE_OP_COMMIT);
  push_u8(app, 1u);
  ++ops;

  memcpy(app->cmd_buf + 20, &ops, sizeof(uint32_t));
  NveBufferSlice out = {app->cmd_buf, app->cmd_len};
  return out;
}

static uint32_t fixture_dispatch_events(NveAppHandle h, NveBufferSlice events) {
  WinStatefulFixtureApp* app = (WinStatefulFixtureApp*)h;
  if (app != NULL && events.ptr != NULL && events.len > 0) {
    app->counter += 1;
    return 1u;
  }
  return 0u;
}

static uint32_t fixture_set_props(NveAppHandle h, NveBufferSlice props) {
  WinStatefulFixtureApp* app = (WinStatefulFixtureApp*)h;
  if (app == NULL || props.ptr == NULL || props.len == 0) {
    return 0u;
  }
  if (strstr((const char*)props.ptr, "Updated") != NULL) {
    snprintf(app->prefix, sizeof(app->prefix), "Updated");
    return 1u;
  }
  return 0u;
}

static NveBufferSlice fixture_drain_emitted_events(NveAppHandle h) {
  (void)h;
  NveBufferSlice empty = {NULL, 0};
  return empty;
}

static uint32_t fixture_apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
  WinStatefulFixtureApp* app = (WinStatefulFixtureApp*)h;
  if (app == NULL || patch.ptr == NULL || patch.len == 0) {
    return 0u;
  }
  char buf[32];
  const size_t n = patch.len < sizeof(buf) - 1 ? patch.len : sizeof(buf) - 1;
  memcpy(buf, patch.ptr, n);
  buf[n] = '\0';
  app->counter = (int32_t)strtol(buf, NULL, 10);
  return 1u;
}

static NveBufferSlice fixture_snapshot_state(NveAppHandle h) {
  WinStatefulFixtureApp* app = (WinStatefulFixtureApp*)h;
  app->snapshot_word = app->counter;
  NveBufferSlice out = {(const uint8_t*)&app->snapshot_word, sizeof(app->snapshot_word)};
  return out;
}

static void fixture_restore_state(NveAppHandle h, NveBufferSlice state) {
  WinStatefulFixtureApp* app = (WinStatefulFixtureApp*)h;
  if (app != NULL && state.ptr != NULL && state.len == sizeof(int32_t)) {
    memcpy(&app->counter, state.ptr, sizeof(int32_t));
  }
}

NVE_API const void* nve_get_interface(uint32_t interface_id) {
  static const NveSurfaceApi kSurface = {
      fixture_create,
      fixture_destroy,
      fixture_render,
      fixture_dispatch_events,
      NULL,
  };
  static const NveComponentApi kComponent = {
      fixture_set_props,
      fixture_drain_emitted_events,
      NULL,
  };
  static const NveDevApi kDev = {
      fixture_apply_hot_patch,
      fixture_snapshot_state,
      fixture_restore_state,
  };
  switch (interface_id) {
    case NVE_IFACE_SURFACE:
      return &kSurface;
    case NVE_IFACE_COMPONENT:
      return &kComponent;
    case NVE_IFACE_DEV:
      return &kDev;
    default:
      return NULL;
  }
}

#else
#error "Define one of NVE_FIXTURE_NO_ENTRY, NVE_FIXTURE_NO_SURFACE, or NVE_FIXTURE_STATEFUL"
#endif
