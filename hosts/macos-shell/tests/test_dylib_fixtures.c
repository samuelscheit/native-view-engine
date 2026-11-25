#include "nve_abi.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(NVE_FIXTURE_NO_ENTRY)

int nve_fixture_dummy_symbol(void) {
  return 42;
}

#elif defined(NVE_FIXTURE_NO_SURFACE)

const void *nve_get_interface(uint32_t interface_id) {
  (void)interface_id;
  return NULL;
}

#elif defined(NVE_FIXTURE_STATEFUL)

typedef struct StatefulFixtureApp {
  uint32_t surface_id;
  int32_t counter;
  uint8_t cmd_buf[512];
  size_t cmd_len;
  int32_t snapshot_word;
} StatefulFixtureApp;

static void push_bytes(StatefulFixtureApp *app, const void *src, size_t n) {
  if (app->cmd_len + n <= sizeof(app->cmd_buf)) {
    memcpy(app->cmd_buf + app->cmd_len, src, n);
    app->cmd_len += n;
  }
}

static void push_u8(StatefulFixtureApp *app, uint8_t v) { push_bytes(app, &v, sizeof(v)); }
static void push_u16(StatefulFixtureApp *app, uint16_t v) { push_bytes(app, &v, sizeof(v)); }
static void push_u32(StatefulFixtureApp *app, uint32_t v) { push_bytes(app, &v, sizeof(v)); }
static void push_u64(StatefulFixtureApp *app, uint64_t v) { push_bytes(app, &v, sizeof(v)); }

static NveAppHandle fixture_create(uint32_t surface_id) {
  StatefulFixtureApp *app = (StatefulFixtureApp *)calloc(1, sizeof(StatefulFixtureApp));
  if (app) {
    app->surface_id = surface_id;
    app->counter = 0;
  }
  return app;
}

static void fixture_destroy(NveAppHandle h) {
  free(h);
}

static NveBufferSlice fixture_render(NveAppHandle h, uint64_t frame_id) {
  StatefulFixtureApp *app = (StatefulFixtureApp *)h;
  app->cmd_len = 0;
  uint32_t ops = 0;

  push_u32(app, NVE_ABI_MAGIC);
  push_u16(app, NVE_ABI_VERSION);
  push_u16(app, 0);
  push_u32(app, app->surface_id);
  push_u64(app, frame_id);
  push_u32(app, 0);

  // Create node 2 (text) and node 3 (pressable)
  push_u8(app, NVE_OP_CREATE_NODE);
  push_u32(app, 2);
  push_u16(app, NVE_COMP_TEXT);
  push_u16(app, 0);
  ++ops;

  push_u8(app, NVE_OP_CREATE_NODE);
  push_u32(app, 3);
  push_u16(app, NVE_COMP_PRESSABLE);
  push_u16(app, 0);
  ++ops;

  // Root -> {2, 3}
  push_u8(app, NVE_OP_SET_CHILDREN);
  push_u32(app, NVE_ROOT_NODE_ID);
  push_u32(app, 2);
  push_u32(app, 2);
  push_u32(app, 3);
  ++ops;

  // Node 2 text: "state=<counter>"
  char label[64];
  const int label_len = snprintf(label, sizeof(label), "state=%d", app->counter);
  push_u8(app, NVE_OP_SET_TEXT);
  push_u32(app, 2);
  push_u32(app, (uint32_t)(label_len > 0 ? label_len : 0));
  if (label_len > 0) {
    push_bytes(app, label, (size_t)label_len);
  }
  ++ops;

  push_u8(app, NVE_OP_COMMIT);
  push_u8(app, 1);
  ++ops;

  memcpy(app->cmd_buf + 20, &ops, sizeof(uint32_t));
  NveBufferSlice out = {app->cmd_buf, app->cmd_len};
  return out;
}

static uint32_t fixture_dispatch_events(NveAppHandle h, NveBufferSlice events) {
  StatefulFixtureApp *app = (StatefulFixtureApp *)h;
  if (events.ptr && events.len > 0) {
    app->counter += 1;
    return 1;
  }
  return 0;
}

static uint32_t fixture_apply_hot_patch(NveAppHandle h, NveBufferSlice patch_json) {
  StatefulFixtureApp *app = (StatefulFixtureApp *)h;
  if (!patch_json.ptr || patch_json.len == 0) {
    return 0;
  }
  char buf[32];
  const size_t n = patch_json.len < sizeof(buf) - 1 ? patch_json.len : sizeof(buf) - 1;
  memcpy(buf, patch_json.ptr, n);
  buf[n] = '\0';
  app->counter = (int32_t)strtol(buf, NULL, 10);
  return 1;
}

static NveBufferSlice fixture_snapshot_state(NveAppHandle h) {
  StatefulFixtureApp *app = (StatefulFixtureApp *)h;
  app->snapshot_word = app->counter;
  NveBufferSlice out = {(const uint8_t *)&app->snapshot_word, sizeof(app->snapshot_word)};
  return out;
}

static void fixture_restore_state(NveAppHandle h, NveBufferSlice state) {
  StatefulFixtureApp *app = (StatefulFixtureApp *)h;
  if (state.ptr && state.len == sizeof(int32_t)) {
    memcpy(&app->counter, state.ptr, sizeof(int32_t));
  }
}

const void *nve_get_interface(uint32_t interface_id) {
  static const NveSurfaceApi kSurface = {
      .create = &fixture_create,
      .destroy = &fixture_destroy,
      .render = &fixture_render,
      .dispatch_events = &fixture_dispatch_events,
  };
  static const NveDevApi kDev = {
      .apply_hot_patch = &fixture_apply_hot_patch,
      .snapshot_state = &fixture_snapshot_state,
      .restore_state = &fixture_restore_state,
  };
  switch (interface_id) {
    case NVE_IFACE_SURFACE:
      return &kSurface;
    case NVE_IFACE_DEV:
      return &kDev;
    default:
      return NULL;
  }
}

#else
#error "Define one of NVE_FIXTURE_NO_ENTRY, NVE_FIXTURE_NO_SURFACE, or NVE_FIXTURE_STATEFUL"
#endif
