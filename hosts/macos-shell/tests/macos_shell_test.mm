#import <AppKit/AppKit.h>
#import "NveAppleCanvasView.h"
#import "NveAppleCamera.h"
#import "NveAppleComponentHost.h"
#import "NveAppleImageCache.h"
#import "NveAppleNativeViews.h"
#import "NveMacFabricHost.h"
#import "RCTNveMacComponentView.h"

#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_pair_client.hpp"
#include "nve_rn_component.hpp"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

#undef assert
#define assert(expr) NVE_CHECK(expr)

namespace {

bool approx_eq(CGFloat a, CGFloat b, CGFloat eps = 1.0) {
  return std::fabs(a - b) <= eps;
}

struct MockMacRustComponent {
  uint32_t surface_id{1};
  int count{0};
  std::string title{"Initial"};
  std::vector<uint8_t> cmd_buf;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;

  static NveAppHandle create(uint32_t sid) {
    auto *self = new MockMacRustComponent();
    self->surface_id = sid;
    self->outbound.clear();
    return self;
  }

  static void destroy(NveAppHandle h) { delete static_cast<MockMacRustComponent *>(h); }

  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  void push_dim(uint8_t tag, float val) {
    push<uint8_t>(tag);
    push<float>(val);
  }

  void push_insets(float t, float r, float b, float l) {
    push<float>(t);
    push<float>(r);
    push<float>(b);
    push<float>(l);
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0;

    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0);
      ++ops;
    };

    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };

    auto set_text = [&](uint32_t id, const std::string &s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };

    auto set_layout = [&](uint32_t id,
                          uint8_t dir,
                          uint8_t wtag,
                          float w,
                          uint8_t htag,
                          float h,
                          float grow,
                          float pad,
                          float gap) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push_dim(wtag, w);
      self->push_dim(htag, h);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push<uint8_t>(dir);
      self->push<uint8_t>(0); /* FlexStart */
      self->push<uint8_t>(0); /* Stretch */
      self->push<float>(grow);
      self->push<float>(1.0f);
      self->push<float>(gap);
      self->push_insets(pad, pad, pad, pad);
      self->push_insets(0, 0, 0, 0);
      ++ops;
    };

    auto set_style = [&](uint32_t id, uint32_t bg, uint32_t fg, float radius, float font_size) {
      self->push<uint8_t>(NVE_OP_SET_STYLE);
      self->push<uint32_t>(id);
      self->push<uint32_t>(bg);
      self->push<uint32_t>(fg);
      self->push<uint32_t>(0xFF0000FFu);
      self->push<float>(1.5f);
      self->push<float>(radius);
      self->push<float>(0.95f);
      self->push<float>(font_size);
      self->push<int32_t>(2);
      ++ops;
    };

    create_node(2, NVE_COMP_VIEW);
    create_node(3, NVE_COMP_TEXT);
    create_node(4, NVE_COMP_PRESSABLE);

    set_children(NVE_ROOT_NODE_ID, {2});
    set_children(2, {3, 4});

    // Row container: 100% x 100%, padding=20, gap=10
    set_layout(2, 1, 2, 100.0f, 2, 100.0f, 0.0f, 20.0f, 10.0f);
    set_style(2, 0xFF202020u, 0xFFFFFFFFu, 8.0f, 16.0f);

    // Label: fixed width=120, height=40
    set_layout(3, 0, 1, 120.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    set_text(3, self->title + ":" + std::to_string(self->count));
    set_style(3, 0x00000000u, 0xFFFFFFFFu, 0.0f, 18.0f);

    // Pressable button: flex_grow=1, height=40
    set_layout(4, 0, 0, 0.0f, 1, 40.0f, 1.0f, 0.0f, 0.0f);
    set_style(4, 0xFFFF5500u, 0xFFFFFFFFu, 12.0f, 16.0f);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;

    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    for (const auto &ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4) {
        ++self->count;
        const NveMapEntry entry{
            nve::make_str_slice("count"),
            nve::make_i64_value(static_cast<int64_t>(self->count)),
        };
        const NveValue payload = nve::make_map_value(&entry, 1);
        self->outbound.push_custom_value(
            self->surface_id, 4, ev.timestamp_us, "onIncrement", payload);
        return 1;
      }
    }
    return 0;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent &ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 4) {
        ++self->count;
        const NveMapEntry entry{
            nve::make_str_slice("count"),
            nve::make_i64_value(static_cast<int64_t>(self->count)),
        };
        const NveValue payload = nve::make_map_value(&entry, 1);
        self->outbound.push_custom_value(
            self->surface_id, 4, ev.timestamp_us, "onIncrement", payload);
        return 1;
      }
    }
    return 0;
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    self->title.assign(reinterpret_cast<const char *>(patch.ptr), patch.len);
    return 1;
  }

  static NveBufferSlice snapshot_state(NveAppHandle) { return {nullptr, 0}; }
  static void restore_state(NveAppHandle, NveBufferSlice) {}

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    if (const auto view = nve::BinValueView::parse(props)) {
      if (const auto title = view->map_get("title"); title && title->as_str()) {
        self->title = std::string(*title->as_str());
      }
    } else if (props.ptr && props.len > 0) {
      std::string raw(reinterpret_cast<const char *>(props.ptr), props.len);
      if (raw.find("Updated") != std::string::npos) {
        self->title = "Updated";
      }
    }
    const NveMapEntry entry{nve::make_str_slice("ok"), nve::make_bool_value(true)};
    const NveValue payload = nve::make_map_value(&entry, 1);
    self->outbound.push_custom_value(
        self->surface_id, NVE_ROOT_NODE_ID, 500, "onPropsApplied", payload);
    return 1;
  }

  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry *entries, uint32_t count) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    for (uint32_t i = 0; i < count; ++i) {
      if (nve::str_slice_view(entries[i].key) == "title" &&
          entries[i].value.tag == NVE_VAL_STR) {
        self->title = std::string(nve::str_slice_view(entries[i].value.data.str_val));
      }
    }
    const NveMapEntry entry{nve::make_str_slice("ok"), nve::make_bool_value(true)};
    const NveValue payload = nve::make_map_value(&entry, 1);
    self->outbound.push_custom_value(
        self->surface_id, NVE_ROOT_NODE_ID, 500, "onPropsApplied", payload);
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static const void *get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
        .dispatch_events_direct = &dispatch_events_direct,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
        .set_props_direct = &set_props_direct,
    };
    static const NveDevApi kDev{
        .apply_hot_patch = &apply_hot_patch,
        .snapshot_state = &snapshot_state,
        .restore_state = &restore_state,
    };
    switch (interface_id) {
      case NVE_IFACE_SURFACE: return &kSurface;
      case NVE_IFACE_COMPONENT: return &kComponent;
      case NVE_IFACE_DEV: return &kDev;
      default: return nullptr;
    }
  }
};

struct HostEdgeApp {
  uint32_t surface_id{1};
  int stage{0};
  std::string last_typed;
  std::vector<uint8_t> cmd_buf;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;

  static NveAppHandle create(uint32_t sid) {
    auto *self = new HostEdgeApp();
    self->surface_id = sid;
    self->outbound.clear();
    return self;
  }
  static void destroy(NveAppHandle h) { delete static_cast<HostEdgeApp *>(h); }

  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<HostEdgeApp *>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_box = [&](uint32_t id, float w, float h) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push<uint8_t>(1); self->push<float>(w);
      self->push<uint8_t>(1); self->push<float>(h);
      for (int i = 0; i < 4; ++i) { self->push<uint8_t>(0); self->push<float>(0.0f); }
      self->push<uint8_t>(0); self->push<uint8_t>(0); self->push<uint8_t>(0);
      self->push<float>(0.0f); self->push<float>(0.0f); self->push<float>(0.0f);
      for (int i = 0; i < 8; ++i) self->push<float>(0.0f);
      ++ops;
    };
    auto set_mask = [&](uint32_t id, uint64_t mask) {
      self->push<uint8_t>(NVE_OP_SET_EVENT_MASK);
      self->push<uint32_t>(id);
      self->push<uint64_t>(mask);
      ++ops;
    };

    if (self->stage == 0) {
      create_node(2, NVE_COMP_SCROLL_VIEW);
      create_node(3, NVE_COMP_VIEW);
      create_node(4, NVE_COMP_VIEW);
      create_node(5, NVE_COMP_TEXT_INPUT);
      set_box(2, 200.0f, 100.0f);
      set_box(3, 180.0f, 150.0f);
      set_box(4, 180.0f, 150.0f);
      set_box(5, 180.0f, 32.0f);
      set_mask(3, 1ULL << 0); /* interactive Press initially */
      set_children(NVE_ROOT_NODE_ID, {2});
      set_children(2, {3, 4, 5});
    } else if (self->stage == 1) {
      // Reorder children (4 before 3), drop 5 from children WITHOUT REMOVE_NODE, disable Press on 3.
      set_mask(3, 0ULL);
      set_children(2, {4, 3});
    }

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto *self = static_cast<HostEdgeApp *>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    for (const auto &ev : decoded) {
      if (ev.kind == nve::EventKind::TextChange && ev.node_id == 5) {
        self->last_typed = ev.text;
        const NveMapEntry entry{
            nve::make_str_slice("typed"),
            nve::make_str_value(nve::make_str_slice(ev.text)),
        };
        const NveValue payload = nve::make_map_value(&entry, 1);
        self->outbound.push_custom_value(
            self->surface_id, 5, ev.timestamp_us, "onTextTyped", payload);
        return 0;
      }
    }
    return 0;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto *self = static_cast<HostEdgeApp *>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent &ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::TextChange) && ev.node_id == 5) {
        self->last_typed = std::string(nve::str_slice_view(ev.text));
        const NveMapEntry entry{
            nve::make_str_slice("typed"),
            nve::make_str_value(nve::make_str_slice(self->last_typed)),
        };
        const NveValue payload = nve::make_map_value(&entry, 1);
        self->outbound.push_custom_value(
            self->surface_id, 5, ev.timestamp_us, "onTextTyped", payload);
        return 0;
      }
    }
    return 0;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto *self = static_cast<HostEdgeApp *>(h);
    if (const auto view = nve::BinValueView::parse(props)) {
      if (const auto stage = view->map_get("stage"); stage && stage->as_i64() == 1) {
        self->stage = 1;
        return 1;
      }
    } else if (props.ptr && props.len > 0) {
      std::string json(reinterpret_cast<const char *>(props.ptr), props.len);
      if (json.find("\"stage\":1") != std::string::npos) {
        self->stage = 1;
        return 1;
      }
    }
    return 0;
  }

  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry *entries, uint32_t count) {
    auto *self = static_cast<HostEdgeApp *>(h);
    for (uint32_t i = 0; i < count; ++i) {
      if (nve::str_slice_view(entries[i].key) == "stage") {
        if ((entries[i].value.tag == NVE_VAL_I64 && entries[i].value.data.i64_val == 1) ||
            (entries[i].value.tag == NVE_VAL_F64 && entries[i].value.data.f64_val == 1.0)) {
          self->stage = 1;
          return 1;
        }
      }
    }
    return 0;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto *self = static_cast<HostEdgeApp *>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static const void *get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveComponentApi kComp{&set_props, &drain_emitted_events, &set_props_direct};
    if (interface_id == NVE_IFACE_SURFACE) return &kSurface;
    if (interface_id == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

struct A11yImeMacApp {
  uint32_t surface_id{1};
  bool disabled_btn{false};
  bool secure_input{false};
  std::string input_text{"init"};
  int focus_count{0};
  int blur_count{0};
  int text_change_count{0};
  int submit_count{0};
  std::string last_text;
  std::string last_submit;
  std::vector<uint8_t> cmd_buf;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;
  static inline A11yImeMacApp *last_instance = nullptr;

  static NveAppHandle create(uint32_t sid) {
    auto *self = new A11yImeMacApp();
    self->surface_id = sid;
    self->outbound.clear();
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto *self = static_cast<A11yImeMacApp *>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<A11yImeMacApp *>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_text = [&](uint32_t id, const std::string &s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };
    auto set_box = [&](uint32_t id, float w, float h) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push<uint8_t>(1); self->push<float>(w);
      self->push<uint8_t>(1); self->push<float>(h);
      for (int i = 0; i < 4; ++i) { self->push<uint8_t>(0); self->push<float>(0.0f); }
      self->push<uint8_t>(0); self->push<uint8_t>(0); self->push<uint8_t>(0);
      self->push<float>(0.0f); self->push<float>(1.0f); self->push<float>(0.0f);
      for (int i = 0; i < 8; ++i) self->push<float>(0.0f);
      ++ops;
    };
    auto set_raw_str = [&](uint32_t id, std::string_view k, std::string_view v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(4);
      self->push<uint32_t>(static_cast<uint32_t>(v.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), v.begin(), v.end());
      ++ops;
    };
    auto set_raw_bool = [&](uint32_t id, std::string_view k, bool v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(1);
      self->push<uint8_t>(v ? 1 : 0);
      ++ops;
    };

    create_node(2, NVE_COMP_PRESSABLE);
    create_node(3, NVE_COMP_TEXT);
    create_node(4, NVE_COMP_VIEW);
    create_node(5, NVE_COMP_VIEW);
    create_node(6, NVE_COMP_TEXT_INPUT);
    create_node(7, NVE_COMP_SCROLL_VIEW);

    set_children(NVE_ROOT_NODE_ID, {2, 3, 4, 5, 6, 7});
    set_box(2, 140.0f, 36.0f);
    set_box(3, 180.0f, 24.0f);
    set_box(4, 100.0f, 24.0f);
    set_box(5, 80.0f, 20.0f);
    set_box(6, 200.0f, 32.0f);
    set_box(7, 200.0f, 60.0f);

    // 2: Pressable button with full a11y metadata + dynamic disabled toggle
    set_raw_str(2, "accessibilityLabel", "Save Document");
    set_raw_str(2, "accessibilityHint", "Persists your current work");
    set_raw_str(2, "testID", "save-btn");
    set_raw_bool(2, "accessibilityDisabled", self->disabled_btn);

    // 3: Static text with aria-label / role / data-testid
    set_text(3, "Visible Title");
    set_raw_str(3, "aria-label", "Settings Header");
    set_raw_str(3, "role", "header");
    set_raw_str(3, "data-testid", "hdr-1");

    // 4: Explicit checkbox role + value
    set_raw_str(4, "accessibilityRole", "checkbox");
    set_raw_str(4, "accessibilityLabel", "Enable Sync");
    set_raw_str(4, "accessibilityValue", "checked");

    // 5: Explicit none role -> accessibilityElement == NO
    set_raw_str(5, "role", "none");

    // 6: TextInput with placeholder, secureTextEntry, testID, a11y label
    set_text(6, self->input_text);
    set_raw_str(6, "placeholder", "Enter name...");
    set_raw_str(6, "accessibilityLabel", "Display Name");
    set_raw_str(6, "accessibilityHint", "Type your display name");
    set_raw_str(6, "testID", "name-input");
    set_raw_bool(6, "secureTextEntry", self->secure_input);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto *self = static_cast<A11yImeMacApp *>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    for (const auto &ev : decoded) {
      if (ev.node_id == 6) {
        if (ev.kind == nve::EventKind::Focus) {
          ++self->focus_count;
        } else if (ev.kind == nve::EventKind::Blur) {
          ++self->blur_count;
        } else if (ev.kind == nve::EventKind::TextChange) {
          ++self->text_change_count;
          self->last_text = ev.text;
        } else if (ev.kind == nve::EventKind::Custom && ev.name == "onSubmit") {
          ++self->submit_count;
          if (const auto v = nve::BinValueView::parse(ev.payload_bytes.data(), ev.payload_bytes.size());
              v && v->as_str()) {
            self->last_submit = std::string(*v->as_str());
          } else {
            self->last_submit = std::string(ev.payload_as_string());
          }
        }
      }
    }
    return 0;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto *self = static_cast<A11yImeMacApp *>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent &ev = batch.ptr[i];
      if (ev.node_id == 6) {
        if (ev.kind == static_cast<uint8_t>(nve::EventKind::Focus)) {
          ++self->focus_count;
        } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Blur)) {
          ++self->blur_count;
        } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::TextChange)) {
          ++self->text_change_count;
          self->last_text = std::string(nve::str_slice_view(ev.text));
        } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Custom) &&
                   nve::str_slice_view(ev.name) == "onSubmit") {
          ++self->submit_count;
          if (ev.value.tag == NVE_VAL_STR) {
            self->last_submit = std::string(nve::str_slice_view(ev.value.data.str_val));
          } else {
            self->last_submit.clear();
          }
        }
      }
    }
    return 0;
  }

  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry *entries, uint32_t count) {
    auto *self = static_cast<A11yImeMacApp *>(h);
    for (uint32_t i = 0; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "disabled" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->disabled_btn = entries[i].value.data.bool_val != 0;
      } else if (k == "secure" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->secure_input = entries[i].value.data.bool_val != 0;
      } else if (k == "inputText" && entries[i].value.tag == NVE_VAL_STR) {
        self->input_text = std::string(nve::str_slice_view(entries[i].value.data.str_val));
      }
    }
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto *self = static_cast<A11yImeMacApp *>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static const void *get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveComponentApi kComp{nullptr, &drain_emitted_events, &set_props_direct};
    if (interface_id == NVE_IFACE_SURFACE) return &kSurface;
    if (interface_id == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

struct ImageScrollIncrementalApp {
  uint32_t surface_id{1};
  int stage{0};
  std::string image_src;
  std::string resize_mode{"cover"};
  std::string counter_text{"Tick 0"};
  int scroll_count{0};
  float last_scroll_x{-1.0f};
  float last_scroll_y{-1.0f};
  float last_scroll_cw{0.0f};
  float last_scroll_ch{0.0f};
  std::vector<uint8_t> cmd_buf;
  static inline ImageScrollIncrementalApp *last_instance = nullptr;

  static NveAppHandle create(uint32_t sid) {
    auto *self = new ImageScrollIncrementalApp();
    self->surface_id = sid;
    self->image_src =
        "data:image/png;base64,"
        "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto *self = static_cast<ImageScrollIncrementalApp *>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<ImageScrollIncrementalApp *>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0);
      ++ops;
    };
    auto remove_node = [&](uint32_t id) {
      self->push<uint8_t>(NVE_OP_REMOVE_NODE);
      self->push<uint32_t>(id);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_text = [&](uint32_t id, const std::string &s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };
    auto set_box = [&](uint32_t id, float w, float h) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push<uint8_t>(1); self->push<float>(w);
      self->push<uint8_t>(1); self->push<float>(h);
      for (int i = 0; i < 4; ++i) { self->push<uint8_t>(0); self->push<float>(0.0f); }
      self->push<uint8_t>(0); self->push<uint8_t>(0); self->push<uint8_t>(0);
      self->push<float>(0.0f); self->push<float>(0.0f); self->push<float>(0.0f);
      for (int i = 0; i < 8; ++i) self->push<float>(0.0f);
      ++ops;
    };
    auto set_raw_str = [&](uint32_t id, std::string_view k, std::string_view v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(4);
      self->push<uint32_t>(static_cast<uint32_t>(v.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), v.begin(), v.end());
      ++ops;
    };

    if (frame_id == 1) {
      create_node(2, NVE_COMP_IMAGE);
      create_node(3, NVE_COMP_SCROLL_VIEW);
      create_node(4, NVE_COMP_VIEW);
      create_node(5, NVE_COMP_TEXT);
      create_node(6, NVE_COMP_TEXT);
      set_box(2, 32.0f, 32.0f);
      set_raw_str(2, "src", self->image_src);
      set_raw_str(2, "resizeMode", self->resize_mode);
      set_box(3, 240.0f, 120.0f);
      set_box(4, 300.0f, 400.0f);
      set_box(5, 160.0f, 24.0f);
      set_box(6, 160.0f, 24.0f);
      set_text(5, self->counter_text);
      set_text(6, "Removable");
      set_children(NVE_ROOT_NODE_ID, {2, 3, 5, 6});
      set_children(3, {4});
    } else if (self->stage == 1) {
      // Incremental frame: update ONLY text of node 5 (O(1) dirty!).
      set_text(5, self->counter_text);
    } else if (self->stage == 2) {
      // Raw-prop-only update on existing Image node 2 (resizeMode / src).
      set_raw_str(2, "src", self->image_src);
      set_raw_str(2, "resizeMode", self->resize_mode);
    } else if (self->stage == 3) {
      // Incremental removal of node 6.
      set_children(NVE_ROOT_NODE_ID, {2, 3, 5});
      remove_node(6);
    }

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto *self = static_cast<ImageScrollIncrementalApp *>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent &ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Scroll) && ev.node_id == 3) {
        ++self->scroll_count;
        self->last_scroll_x = ev.x;
        self->last_scroll_y = ev.y;
        self->last_scroll_cw = ev.width;
        self->last_scroll_ch = ev.height;
      }
    }
    return 0;
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto *self = static_cast<ImageScrollIncrementalApp *>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    for (const auto &ev : decoded) {
      if (ev.kind == nve::EventKind::Scroll && ev.node_id == 3) {
        ++self->scroll_count;
        self->last_scroll_x = ev.x;
        self->last_scroll_y = ev.y;
        self->last_scroll_cw = ev.width;
        self->last_scroll_ch = ev.height;
      }
    }
    return 0;
  }

  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry *entries, uint32_t count) {
    auto *self = static_cast<ImageScrollIncrementalApp *>(h);
    for (uint32_t i = 0; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "stage" && entries[i].value.tag == NVE_VAL_I64) {
        self->stage = static_cast<int>(entries[i].value.data.i64_val);
      } else if (k == "resizeMode" && entries[i].value.tag == NVE_VAL_STR) {
        self->resize_mode = std::string(nve::str_slice_view(entries[i].value.data.str_val));
      } else if (k == "src" && entries[i].value.tag == NVE_VAL_STR) {
        self->image_src = std::string(nve::str_slice_view(entries[i].value.data.str_val));
      } else if (k == "counterText" && entries[i].value.tag == NVE_VAL_STR) {
        self->counter_text = std::string(nve::str_slice_view(entries[i].value.data.str_val));
      }
    }
    return 1;
  }

  static const void *get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        &create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveComponentApi kComp{nullptr, nullptr, &set_props_direct};
    if (interface_id == NVE_IFACE_SURFACE) return &kSurface;
    if (interface_id == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

struct CanvasSparklineApp {
  uint32_t surface_id{1};
  int stage{0};
  std::vector<uint8_t> draw_bytes;
  std::string draw_b64;
  std::vector<uint8_t> cmd_buf;
  static inline CanvasSparklineApp *last_instance = nullptr;

  static NveAppHandle create(uint32_t sid) {
    auto *self = new CanvasSparklineApp();
    self->surface_id = sid;
    // Studio-style rising telemetry sparkline: bg=(15,23,42), stroke=(56,189,248)
    nve::DrawListEncoder enc;
    const std::array<float, 6> samples = {10.0f, 18.0f, 25.0f, 42.0f, 68.0f, 95.0f};
    enc.sparkline(
        samples,
        240.0f,
        80.0f,
        nve::DrawColorRgba::rgb(56, 189, 248),
        nve::DrawColorRgba::rgb(15, 23, 42));
    self->draw_bytes = enc.finish();
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto *self = static_cast<CanvasSparklineApp *>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }
  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<CanvasSparklineApp *>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0u;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_box = [&](uint32_t id, float w, float h) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push<uint8_t>(1); self->push<float>(w);
      self->push<uint8_t>(1); self->push<float>(h);
      for (int i = 0; i < 4; ++i) { self->push<uint8_t>(0); self->push<float>(0.0f); }
      self->push<uint8_t>(0); self->push<uint8_t>(0); self->push<uint8_t>(0);
      self->push<float>(0.0f); self->push<float>(0.0f); self->push<float>(0.0f);
      for (int i = 0; i < 8; ++i) self->push<float>(0.0f);
      ++ops;
    };
    auto set_raw_bytes = [&](uint32_t id, std::string_view k, const std::vector<uint8_t> &b) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(5);
      self->push<uint32_t>(static_cast<uint32_t>(b.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), b.begin(), b.end());
      ++ops;
    };
    auto set_raw_str = [&](uint32_t id, std::string_view k, std::string_view v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(4);
      self->push<uint32_t>(static_cast<uint32_t>(v.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), v.begin(), v.end());
      ++ops;
    };

    if (frame_id == 1u) {
      create_node(2u, NVE_COMP_SKIA_CANVAS);
      set_box(2u, 240.0f, 80.0f);
      set_raw_bytes(2u, "drawList", self->draw_bytes);
      set_children(NVE_ROOT_NODE_ID, {2u});
    } else if (self->stage == 1) {
      set_raw_bytes(2u, "drawList", self->draw_bytes);
    } else if (self->stage == 2) {
      set_raw_str(2u, "draw_list", self->draw_b64);
    }

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle, NveBufferSlice) { return 0u; }
  static uint32_t dispatch_events_direct(NveAppHandle, NveEventBatch) { return 0u; }
  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry *entries, uint32_t count) {
    auto *self = static_cast<CanvasSparklineApp *>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "stage" && entries[i].value.tag == NVE_VAL_I64) {
        self->stage = static_cast<int>(entries[i].value.data.i64_val);
      } else if (k == "b64" && entries[i].value.tag == NVE_VAL_STR) {
        self->draw_b64 = std::string(nve::str_slice_view(entries[i].value.data.str_val));
      } else if (k == "bytes" && entries[i].value.tag == NVE_VAL_BYTES) {
        const auto b = entries[i].value.data.bytes_val;
        self->draw_bytes.assign(b.ptr, b.ptr + b.len);
      }
    }
    return 1u;
  }
  static NveBufferSlice drain_emitted_events(NveAppHandle) { return {nullptr, 0u}; }
  static const void *get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveComponentApi kComp{nullptr, &drain_emitted_events, &set_props_direct};
    if (interface_id == NVE_IFACE_SURFACE) return &kSurface;
    if (interface_id == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

} // namespace

@interface TestMacComponentDelegate : NSObject <RCTNveMacComponentViewDelegate>
@property (nonatomic, strong) NSMutableArray<NSString *> *events;
@end

@implementation TestMacComponentDelegate
- (instancetype)init {
  self = [super init];
  if (self) {
    _events = [NSMutableArray array];
  }
  return self;
}
- (void)nveComponentView:(NSView *)view
            didEmitEvent:(NSString *)eventName
                 payload:(NSDictionary<NSString *, id> *)payload {
  (void)view;
  (void)payload;
  [self.events addObject:eventName];
}
@end

@interface MockVisionCameraBridge : NSObject <NveVisionCameraAppleBridge>
@property (nonatomic) NSUInteger createCount;
@property (nonatomic) NSUInteger updateCount;
@property (nonatomic) NSUInteger commandCount;
@property (nonatomic) NSUInteger unmountCount;
@property (nonatomic) nve::apple::CameraPreviewFacing lastFacing;
@property (nonatomic) nve::apple::TorchMode lastTorch;
@property (nonatomic) float lastZoom;
@property (nonatomic) nve::apple::CameraPreviewCommandKind lastCommandKind;
@property (nonatomic, copy, nullable) NSString *lastRecordingPath;
@end

@implementation MockVisionCameraBridge
- (NvePlatformView *)createVisionCameraViewForNodeId:(uint32_t)nodeId frame:(CGRect)frame {
  (void)nodeId;
  ++self.createCount;
  return [[NSView alloc] initWithFrame:frame];
}
- (void)updateVisionCameraView:(NvePlatformView *)view
                         props:(const nve::apple::CameraPreviewProps &)props {
  (void)view;
  ++self.updateCount;
  self.lastFacing = props.facing;
  self.lastTorch = props.torch;
  self.lastZoom = props.zoom;
}
- (BOOL)dispatchVisionCameraCommand:(NvePlatformView *)view
                            command:(const nve::apple::CameraPreviewCommand &)cmd {
  (void)view;
  ++self.commandCount;
  self.lastCommandKind = cmd.kind;
  if (cmd.kind == nve::apple::CameraPreviewCommandKind::SwitchFacing) {
    self.lastFacing = cmd.facing;
  } else if (cmd.kind == nve::apple::CameraPreviewCommandKind::StartRecording) {
    self.lastRecordingPath = nve::apple::nsstring_from_sv(cmd.output_path);
  }
  return YES;
}
- (void)unmountVisionCameraView:(NvePlatformView *)view {
  (void)view;
  ++self.unmountCount;
}
@end

int main() {
  @autoreleasepool {
    // 1. Test RCTNveMacComponentView with live AppKit subviews + Yoga layout
    RCTNveMacComponentView *view =
        [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 400, 160)];
    assert(view.isFlipped == YES);

    TestMacComponentDelegate *delegate = [[TestMacComponentDelegate alloc] init];
    view.delegate = delegate;

    __block NSMutableDictionary<NSString *, NSDictionary *> *received =
        [NSMutableDictionary dictionary];
    view.onNveEvent = ^(NSString *eventName, NSDictionary<NSString *, id> *payload) {
      received[eventName] = payload;
    };

    assert([view attachNveInterface:&MockMacRustComponent::get_interface surfaceId:9]);
    assert(view.nveSurfaceId == 9);
    assert(view.nveNodeCount == 4);

    NSView *container = [view nativeViewForNodeId:2];
    NSView *labelView = [view nativeViewForNodeId:3];
    NSView *buttonView = [view nativeViewForNodeId:4];
    assert(container != nil && labelView != nil && buttonView != nil);
    assert([labelView isKindOfClass:[NSTextField class]]);
    assert(labelView.superview == container);
    assert(buttonView.superview == container);
    assert(buttonView.gestureRecognizers.count == 1);

    // Check computed Flexbox layout on actual AppKit frames:
    // container = 400x160, pad=20, gap=10 -> inner width = 360.
    // label = (20, 20, 120, 40), button = (150, 20, 230, 40).
    assert(approx_eq(container.frame.size.width, 400.0));
    assert(approx_eq(container.frame.size.height, 160.0));
    assert(approx_eq(labelView.frame.origin.x, 20.0));
    assert(approx_eq(labelView.frame.origin.y, 20.0));
    assert(approx_eq(labelView.frame.size.width, 120.0));
    assert(approx_eq(labelView.frame.size.height, 40.0));
    assert(approx_eq(buttonView.frame.origin.x, 150.0));
    assert(approx_eq(buttonView.frame.origin.y, 20.0));
    assert(approx_eq(buttonView.frame.size.width, 230.0));
    assert(approx_eq(buttonView.frame.size.height, 40.0));

    NSTextField *label = (NSTextField *)labelView;
    assert([label.stringValue isEqualToString:@"Initial:0"]);
    assert(approx_eq(buttonView.layer.cornerRadius, 12.0));

    // 2. Apply props via NSDictionary -> JSON -> set_props -> re-render + outbound event
    assert([view applyPropsDictionary:@{@"title" : @"Updated"}]);
    assert([label.stringValue isEqualToString:@"Updated:0"]);
    assert(received[@"onPropsApplied"] != nil);
    assert([received[@"onPropsApplied"][@"ok"] boolValue] == YES);

    // 3. Simulate click on pressable node 4 -> count increments + onIncrement emitted
    [view simulatePressOnNodeId:4 atPoint:NSMakePoint(12.0, 12.0)];
    assert([label.stringValue isEqualToString:@"Updated:1"]);
    assert(received[@"onIncrement"] != nil);
    assert([received[@"onIncrement"][@"count"] intValue] == 1);
    assert(delegate.events.count == 2);

    // 4. Resize view -> Yoga recalculates flex_grow button width dynamically!
    [view setFrameSize:NSMakeSize(500.0, 160.0)];
    // New inner width = 460; label = 120; gap = 10 -> button = 330
    assert(approx_eq(buttonView.frame.size.width, 330.0));

    // 5. Test NveMacFabricHostController standalone lifecycle + shared error codes (1/2/3)
    NveSurfaceConfig cfg{.surface_id = 3, .width = 320.0f, .height = 240.0f, .scale_factor = 2.0f};
    NveMacFabricHostController *ctrl =
        [[NveMacFabricHostController alloc] initWithSurfaceConfig:cfg];
    assert([ctrl attachInterface:&MockMacRustComponent::get_interface]);
    assert(ctrl.isAppLoaded == YES);
    assert(ctrl.nodeCount == 4);
    assert([ctrl applyHotPatch:[@"Patched" dataUsingEncoding:NSUTF8StringEncoding]]);

    NSString *noEntryPath = @NVE_TEST_NO_ENTRY_DYLIB;
    NSString *noSurfacePath = @NVE_TEST_NO_SURFACE_DYLIB;
    NSString *statefulPath = @NVE_TEST_STATEFUL_DYLIB;
    NSString *createFailPath = @NVE_TEST_CREATE_FAIL_DYLIB;

    auto assertDylibError = [](NSError *err, NSString *expectedDomain, NSInteger expectedCode) {
      assert(err != nil);
      assert([err.domain isEqualToString:expectedDomain]);
      assert(err.code == expectedCode);
      assert(err.localizedDescription.length > 0);
    };

    // Error code 1: missing path and empty path
    NSError *loadErr = nil;
    assert([ctrl loadRustAppDylibAtPath:@"/nonexistent/libmissing.dylib" error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 1);
    loadErr = nil;
    assert([ctrl loadRustAppDylibAtPath:@"" error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 1);

    // Error code 2: valid dylib missing `nve_get_interface`
    loadErr = nil;
    assert([ctrl loadRustAppDylibAtPath:noEntryPath error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 2);

    // Error code 3: exports `nve_get_interface` but missing `NVE_IFACE_SURFACE`
    loadErr = nil;
    assert([ctrl loadRustAppDylibAtPath:noSurfacePath error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 3);

    // Prior statically-attached app must still be alive after all failed dylib loads.
    assert(ctrl.isAppLoaded == YES);
    assert(ctrl.nodeCount == 4);

    // 5b. Test NveMacFabricHostController hot-reload edge cases:
    //     cold reload, state preservation across warm reload, failed reload preserves live app,
    //     and fresh load resets state.
    NveMacFabricHostController *reloadCtrl =
        [[NveMacFabricHostController alloc] initWithSurfaceConfig:cfg];
    assert(reloadCtrl.isAppLoaded == NO);
    loadErr = nil;
    assert([reloadCtrl reloadRustAppDylibAtPath:statefulPath error:&loadErr] == YES);
    assert(loadErr == nil);
    assert(reloadCtrl.isAppLoaded == YES);
    assert(reloadCtrl.nodeCount == 3);

    auto findSnapshotText = [](NveFabricHostHandle host, uint32_t nodeId) -> std::string {
      for (const auto &snap : nve::fabric_host_snapshot_nodes(host)) {
        if (snap.node_id == nodeId) {
          return snap.text;
        }
      }
      return "";
    };
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=0");

    // Mutate state via hot patch, then reload -> snapshot_state/restore_state preserves "state=77"
    assert([reloadCtrl applyHotPatch:[@"77" dataUsingEncoding:NSUTF8StringEncoding]] == YES);
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=77");

    // Failed reloads (codes 1/2/3) must leave the live app and its state intact
    loadErr = nil;
    assert([reloadCtrl reloadRustAppDylibAtPath:@"/nonexistent/fail.dylib" error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 1);
    assert(reloadCtrl.isAppLoaded == YES);
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=77");

    loadErr = nil;
    assert([reloadCtrl reloadRustAppDylibAtPath:noEntryPath error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 2);
    assert(reloadCtrl.isAppLoaded == YES);
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=77");

    loadErr = nil;
    assert([reloadCtrl reloadRustAppDylibAtPath:noSurfacePath error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 3);
    assert(reloadCtrl.isAppLoaded == YES);
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=77");

    // Successful warm reload preserves state=77
    loadErr = nil;
    assert([reloadCtrl reloadRustAppDylibAtPath:statefulPath error:&loadErr] == YES);
    assert(loadErr == nil);
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=77");

    // Fresh load (non-reload) resets state back to 0
    loadErr = nil;
    assert([reloadCtrl loadRustAppDylibAtPath:statefulPath error:&loadErr] == YES);
    assert(loadErr == nil);
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=0");

    // 5c. Test RCTNveMacComponentView shared error codes (1/2/3) & hot-reload edge cases
    RCTNveMacComponentView *reloadView =
        [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 320, 200)];
    loadErr = nil;
    assert([reloadView loadNveDylibAtPath:@"" surfaceId:5 error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.rncomponent", 1);
    loadErr = nil;
    assert([reloadView loadNveDylibAtPath:noEntryPath surfaceId:5 error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.rncomponent", 2);
    loadErr = nil;
    assert([reloadView loadNveDylibAtPath:noSurfacePath surfaceId:5 error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.rncomponent", 3);

    // Cold reload on component view
    loadErr = nil;
    assert([reloadView reloadNveDylibAtPath:statefulPath error:&loadErr] == YES);
    assert(loadErr == nil);
    NSTextField *stateLabel = (NSTextField *)[reloadView nativeViewForNodeId:2];
    assert(stateLabel != nil && [stateLabel isKindOfClass:[NSTextField class]]);
    assert([stateLabel.stringValue isEqualToString:@"state=0"]);

    // Dispatch press events to advance state to 2
    [reloadView simulatePressOnNodeId:3 atPoint:NSMakePoint(4.0, 4.0)];
    [reloadView simulatePressOnNodeId:3 atPoint:NSMakePoint(4.0, 4.0)];
    assert([stateLabel.stringValue isEqualToString:@"state=2"]);

    // Failed reload leaves mounted views and state intact (missing surface + create() returns NULL)
    loadErr = nil;
    assert([reloadView reloadNveDylibAtPath:noSurfacePath error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.rncomponent", 3);
    NSTextField *stillMounted = (NSTextField *)[reloadView nativeViewForNodeId:2];
    assert(stillMounted != nil && [stillMounted.stringValue isEqualToString:@"state=2"]);

    // Transactional rollback: valid dylib whose surface->create() returns NULL must keep live app intact!
    loadErr = nil;
    assert([reloadView reloadNveDylibAtPath:createFailPath error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.rncomponent", 3);
    NSTextField *stillAliveAfterCreateFail = (NSTextField *)[reloadView nativeViewForNodeId:2];
    assert(stillAliveAfterCreateFail == stillMounted);
    assert([stillAliveAfterCreateFail.stringValue isEqualToString:@"state=2"]);
    // Live bridge must still process events after failed transactional reload.
    [reloadView simulatePressOnNodeId:3 atPoint:NSMakePoint(4.0, 4.0)];
    assert([stillAliveAfterCreateFail.stringValue isEqualToString:@"state=3"]);

    // Also verify transactional rollback on NveMacFabricHostController.
    loadErr = nil;
    assert([reloadCtrl reloadRustAppDylibAtPath:createFailPath error:&loadErr] == NO);
    assertDylibError(loadErr, @"nve.macos.shell", 3);
    assert(reloadCtrl.isAppLoaded == YES);
    assert(findSnapshotText(reloadCtrl.fabricHost, 2) == "state=0");

    // Warm reload preserves state=3
    loadErr = nil;
    assert([reloadView reloadNveDylibAtPath:statefulPath error:&loadErr] == YES);
    assert(loadErr == nil);
    NSTextField *reloadedLabel = (NSTextField *)[reloadView nativeViewForNodeId:2];
    assert(reloadedLabel != nil && [reloadedLabel.stringValue isEqualToString:@"state=3"]);

    // 6. Real Rust Dioxus cdylib test: verify nested Text->Text inheritance, visible colors,
    //    non-zero AppKit label frames, live reload state preservation, and optional PNG render.
    const char *dylibEnv = std::getenv("NVE_TEST_COUNTER_DYLIB");
    if (dylibEnv && dylibEnv[0] != '\0') {
      NSString *counterPath = [NSString stringWithUTF8String:dylibEnv];
      RCTNveMacComponentView *counterView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 640, 420)];
      NSError *counterErr = nil;
      assert([counterView loadNveDylibAtPath:counterPath
                                   surfaceId:1
                                       error:&counterErr]);
      assert(counterErr == nil);

      // Collect all leaf NSTextField labels recursively.
      auto collectLabels = [](NSView *root) {
        std::vector<NSTextField *> out;
        std::function<void(NSView *)> walk = [&](NSView *v) {
          if ([v isKindOfClass:[NSTextField class]]) {
            out.push_back((NSTextField *)v);
          }
          for (NSView *sub in v.subviews) {
            walk(sub);
          }
        };
        walk(root);
        return out;
      };
      std::vector<NSTextField *> labels = collectLabels(counterView);
      assert(labels.size() >= 4);

      NSTextField *titleLbl = nil;
      NSTextField *countLbl = nil;
      NSTextField *incLbl = nil;
      NSTextField *resetLbl = nil;
      for (NSTextField *lbl : labels) {
        assert(lbl.frame.size.width > 20.0);
        assert(lbl.frame.size.height >= 16.0);
        NSColor *srgb = [lbl.textColor colorUsingColorSpace:[NSColorSpace sRGBColorSpace]];
        assert(srgb != nil && srgb.alphaComponent > 0.99);
        if ([lbl.stringValue isEqualToString:@"NATIVE VIEW ENGINE"]) titleLbl = lbl;
        if ([lbl.stringValue isEqualToString:@"Count: 0"]) {
          countLbl = lbl;
          assert(approx_eq(lbl.font.pointSize, 28.0));
        }
        if ([lbl.stringValue isEqualToString:@"Increment +"]) incLbl = lbl;
        if ([lbl.stringValue isEqualToString:@"Reset"]) resetLbl = lbl;
      }
      assert(titleLbl != nil && countLbl != nil && incLbl != nil && resetLbl != nil);

      // Helper to find the nearest ancestor view with an opaque layer background color.
      auto findOpaqueAncestor = [](NSView *leaf, NSView *stopAt) -> NSView * {
        for (NSView *cur = leaf.superview; cur != nil && cur != stopAt; cur = cur.superview) {
          CGColorRef bg = cur.layer.backgroundColor;
          if (bg != nullptr && CGColorGetAlpha(bg) > 0.9) {
            return cur;
          }
        }
        return nil;
      };

      // Verify top dark slate card (#1e293b) exists with real geometry & contains both card labels.
      NSView *cardView = findOpaqueAncestor(titleLbl, counterView);
      assert(cardView != nil);
      assert(findOpaqueAncestor(countLbl, counterView) == cardView);
      assert(cardView.frame.size.width > 200.0);
      assert(cardView.frame.size.height > 60.0);

      NSColor *cardColor =
          [[NSColor colorWithCGColor:cardView.layer.backgroundColor]
              colorUsingColorSpace:[NSColorSpace sRGBColorSpace]];
      assert(cardColor != nil);
      assert(approx_eq(cardColor.redComponent, 0x1e / 255.0, 0.03));
      assert(approx_eq(cardColor.greenComponent, 0x29 / 255.0, 0.03));
      assert(approx_eq(cardColor.blueComponent, 0x3b / 255.0, 0.03));
      assert(cardColor.alphaComponent > 0.99);

      const NSRect cardInRoot = [cardView convertRect:cardView.bounds toView:counterView];
      const NSRect titleInRoot = [titleLbl convertRect:titleLbl.bounds toView:counterView];
      const NSRect countInRoot = [countLbl convertRect:countLbl.bounds toView:counterView];
      assert(NSContainsRect(NSInsetRect(cardInRoot, -1.0, -1.0), titleInRoot));
      assert(NSContainsRect(NSInsetRect(cardInRoot, -1.0, -1.0), countInRoot));
      assert(titleInRoot.origin.y < countInRoot.origin.y);

      // Verify both buttons sit strictly below the top card, side by side, with centered labels.
      NSView *incBtn = findOpaqueAncestor(incLbl, counterView);
      NSView *resetBtn = findOpaqueAncestor(resetLbl, counterView);
      assert(incBtn != nil && resetBtn != nil && incBtn != cardView && resetBtn != cardView);
      assert(incBtn.gestureRecognizers.count >= 1);
      assert(resetBtn.gestureRecognizers.count >= 1);
      assert(approx_eq(incBtn.frame.size.width, 160.0, 1.0));
      assert(approx_eq(incBtn.frame.size.height, 48.0, 1.0));
      assert(approx_eq(resetBtn.frame.size.width, 120.0, 1.0));
      assert(approx_eq(resetBtn.frame.size.height, 48.0, 1.0));

      const NSRect incBtnInRoot = [incBtn convertRect:incBtn.bounds toView:counterView];
      const NSRect resetBtnInRoot = [resetBtn convertRect:resetBtn.bounds toView:counterView];
      assert(incBtnInRoot.origin.y > NSMaxY(cardInRoot));
      assert(resetBtnInRoot.origin.y > NSMaxY(cardInRoot));
      assert(approx_eq(incBtnInRoot.origin.y, resetBtnInRoot.origin.y, 1.0));
      assert(resetBtnInRoot.origin.x > NSMaxX(incBtnInRoot));

      const NSRect incLblInBtn = [incLbl convertRect:incLbl.bounds toView:incBtn];
      const NSRect resetLblInBtn = [resetLbl convertRect:resetLbl.bounds toView:resetBtn];
      assert(std::fabs(NSMidX(incLblInBtn) - incBtn.bounds.size.width * 0.5) <= 4.0);
      assert(std::fabs(NSMidY(incLblInBtn) - incBtn.bounds.size.height * 0.5) <= 4.0);
      assert(std::fabs(NSMidX(resetLblInBtn) - resetBtn.bounds.size.width * 0.5) <= 4.0);
      assert(std::fabs(NSMidY(resetLblInBtn) - resetBtn.bounds.size.height * 0.5) <= 4.0);

      // Verify hit-testing a point directly over the "Increment +" label passes through the
      // label to the interactive Pressable button (not swallowed by NSTextField).
      // Note: AppKit `-[NSView hitTest:]` takes a point in the receiver's superview coordinates.
      const NSPoint incCenterInSuper =
          NSMakePoint(NSMidX(incBtn.frame), NSMidY(incBtn.frame));
      NSView *hitView = [incBtn hitTest:incCenterInSuper];
      assert(hitView != nil);
      assert(![hitView isKindOfClass:[NSTextField class]]);
      assert(hitView == incBtn || [hitView isDescendantOf:incBtn]);

      // Verify state preservation across live Rust dylib hot-reload
      assert([counterView applyPropsDictionary:@{@"initialCount" : @5}]);
      bool sawCount5 = false;
      for (NSTextField *lbl : collectLabels(counterView)) {
        if ([lbl.stringValue isEqualToString:@"Count: 5"]) sawCount5 = true;
      }
      assert(sawCount5);

      counterErr = nil;
      assert([counterView reloadNveDylibAtPath:counterPath error:&counterErr] == YES);
      assert(counterErr == nil);
      bool sawCount5AfterReload = false;
      for (NSTextField *lbl : collectLabels(counterView)) {
        if ([lbl.stringValue isEqualToString:@"Count: 5"]) sawCount5AfterReload = true;
      }
      assert(sawCount5AfterReload);

      // Verify Frame-0 mmap snapshot save -> cold replay BEFORE dylib load -> restore_state!
      NSString *counterSnapPath =
          [NSTemporaryDirectory() stringByAppendingPathComponent:
                                      [NSString stringWithFormat:@"nve_mac_counter_%d.nvesnap",
                                                                 getpid()]];
      [[NSFileManager defaultManager] removeItemAtPath:counterSnapPath error:nil];
      assert([counterView saveSnapshotAtPath:counterSnapPath schemaVersion:1]);

      // Reset CounterModule's process-global static counter to 0 before booting fresh view.
      assert([counterView applyPropsDictionary:@{@"initialCount" : @0}]);

      RCTNveMacComponentView *coldCounter =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 640, 420)];
      uint64_t counterRestoreUs = 0;
      assert([coldCounter replayFrame0SnapshotAtPath:counterSnapPath
                               expectedSchemaVersion:1
                                       restoreTimeUs:&counterRestoreUs]);
      std::cout << "[macos_shell_test] Cold mmap Frame-0 restore (counter dylib): "
                << counterRestoreUs << " us\n";

      // Frame 0 MUST already display "Count: 5" BEFORE loadNveDylibAtPath runs!
      bool sawCount5Frame0 = false;
      for (NSTextField *lbl : collectLabels(coldCounter)) {
        if ([lbl.stringValue isEqualToString:@"Count: 5"]) sawCount5Frame0 = true;
      }
      assert(sawCount5Frame0);

      // Loading the dylib must seamlessly restore Count: 5 via restore_state!
      counterErr = nil;
      assert([coldCounter loadNveDylibAtPath:counterPath surfaceId:1 error:&counterErr] == YES);
      assert(counterErr == nil);
      bool sawCount5Live = false;
      for (NSTextField *lbl : collectLabels(coldCounter)) {
        if ([lbl.stringValue isEqualToString:@"Count: 5"]) sawCount5Live = true;
      }
      assert(sawCount5Live);
      [[NSFileManager defaultManager] removeItemAtPath:counterSnapPath error:nil];

      // Restore counterView back to Count: 5 in case screenshot dump follows.
      assert([counterView applyPropsDictionary:@{@"initialCount" : @5}]);

      const char *shotPath = std::getenv("NVE_TEST_SCREENSHOT_PATH");
      if (shotPath && shotPath[0] != '\0') {
        NSBitmapImageRep *rep =
            [counterView bitmapImageRepForCachingDisplayInRect:counterView.bounds];
        [counterView cacheDisplayInRect:counterView.bounds toBitmapImageRep:rep];
        NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
        assert(png != nil && png.length > 0);
        [png writeToFile:[NSString stringWithUTF8String:shotPath] atomically:YES];
      }
    }

    // 7. Test Apple host view reconciliation edge cases:
    //    subview reorder, detach without REMOVE_NODE, NSScrollView documentView sizing,
    //    TextInput controlTextDidChange -> TextChange event, and gesture recognizer removal.
    {
      RCTNveMacComponentView *edgeView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 240, 140)];
      __block NSString *typedOut = nil;
      edgeView.onNveEvent = ^(NSString *name, NSDictionary<NSString *, id> *payload) {
        if ([name isEqualToString:@"onTextTyped"]) {
          typedOut = payload[@"typed"];
        }
      };
      assert([edgeView attachNveInterface:&HostEdgeApp::get_interface surfaceId:21]);

      NSScrollView *scroll = (NSScrollView *)[edgeView nativeViewForNodeId:2];
      NSView *v3 = [edgeView nativeViewForNodeId:3];
      NSView *v4 = [edgeView nativeViewForNodeId:4];
      NSTextField *input5 = (NSTextField *)[edgeView nativeViewForNodeId:5];
      assert([scroll isKindOfClass:[NSScrollView class]]);
      assert(scroll.documentView != nil);
      // ScrollView documentView must fit all 3 stacked children (150 + 150 + 32 = 332pt > 100pt viewport).
      assert(scroll.documentView.frame.size.height >= 330.0);
      assert(scroll.documentView.subviews.count == 3);
      assert(scroll.documentView.subviews[0] == v3);
      assert(scroll.documentView.subviews[1] == v4);
      assert(scroll.documentView.subviews[2] == input5);
      assert(v3.gestureRecognizers.count == 1);

      // TextInput controlTextDidChange dispatches TextChange into the Rust app.
      input5.stringValue = @"hello-nve";
      [(id<NSTextFieldDelegate>)edgeView
          controlTextDidChange:[NSNotification notificationWithName:NSControlTextDidChangeNotification
                                                             object:input5]];
      assert([typedOut isEqualToString:@"hello-nve"]);

      // Transition to stage 1: reorder {4, 3}, detach 5 without REMOVE_NODE, remove press mask on 3.
      assert([edgeView applyPropsDictionary:@{@"stage" : @1}]);
      assert(input5.superview == nil);
      assert(scroll.documentView.subviews.count == 2);
      assert(scroll.documentView.subviews[0] == v4);
      assert(scroll.documentView.subviews[1] == v3);
      assert(v3.gestureRecognizers.count == 0);
    }

    // 8. Test Frame-0 MmapSnapshot save -> cold replay BEFORE dylib load -> restore_state
    //    on stateful fixture dylib (always runs) and optional Studio Workspace dylib.
    {
      NSString *snapPath =
          [NSTemporaryDirectory() stringByAppendingPathComponent:
                                      [NSString stringWithFormat:@"nve_mac_stateful_%d.nvesnap",
                                                                 getpid()]];
      [[NSFileManager defaultManager] removeItemAtPath:snapPath error:nil];

      RCTNveMacComponentView *writerView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 320, 200)];
      NSError *snapErr = nil;
      assert([writerView loadNveDylibAtPath:statefulPath surfaceId:1 error:&snapErr]);
      [writerView simulatePressOnNodeId:3 atPoint:NSMakePoint(4.0, 4.0)];
      [writerView simulatePressOnNodeId:3 atPoint:NSMakePoint(4.0, 4.0)];
      [writerView simulatePressOnNodeId:3 atPoint:NSMakePoint(4.0, 4.0)];
      NSTextField *writerLbl = (NSTextField *)[writerView nativeViewForNodeId:2];
      assert(writerLbl != nil && [writerLbl.stringValue isEqualToString:@"state=3"]);
      assert([writerView saveSnapshotAtPath:snapPath schemaVersion:4]);

      RCTNveMacComponentView *coldView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 320, 200)];
      uint64_t restoreUs = 0;
      assert(![coldView replayFrame0SnapshotAtPath:snapPath
                             expectedSchemaVersion:99
                                     restoreTimeUs:&restoreUs]);
      assert([coldView replayFrame0SnapshotAtPath:snapPath
                            expectedSchemaVersion:4
                                    restoreTimeUs:&restoreUs]);
      std::cout << "[macos_shell_test] Cold mmap Frame-0 restore (stateful dylib): "
                << restoreUs << " us\n";

      // Frame-0 AppKit subview renders exact mutated state BEFORE dylib loads!
      NSTextField *f0Lbl = (NSTextField *)[coldView nativeViewForNodeId:2];
      assert(f0Lbl != nil && [f0Lbl isKindOfClass:[NSTextField class]]);
      assert([f0Lbl.stringValue isEqualToString:@"state=3"]);

      // Loading dylib restores state=3 seamlessly via restore_state!
      snapErr = nil;
      assert([coldView loadNveDylibAtPath:statefulPath surfaceId:1 error:&snapErr]);
      NSTextField *liveLbl = (NSTextField *)[coldView nativeViewForNodeId:2];
      assert(liveLbl != nil && [liveLbl.stringValue isEqualToString:@"state=3"]);
      [[NSFileManager defaultManager] removeItemAtPath:snapPath error:nil];

      const char *studioEnv = std::getenv("NVE_TEST_STUDIO_DYLIB");
      if (studioEnv && studioEnv[0] != '\0') {
        NSString *studioPath = [NSString stringWithUTF8String:studioEnv];
        NSString *studioSnap =
            [NSTemporaryDirectory() stringByAppendingPathComponent:
                                        [NSString stringWithFormat:@"nve_mac_studio_%d.nvesnap",
                                                                   getpid()]];
        [[NSFileManager defaultManager] removeItemAtPath:studioSnap error:nil];

        auto hasLabelText = [](NSView *root, NSString *needle) -> BOOL {
          BOOL found = NO;
          std::function<void(NSView *)> walk = [&](NSView *v) {
            if (found) return;
            if ([v isKindOfClass:[NSTextField class]] &&
                [((NSTextField *)v).stringValue containsString:needle]) {
              found = YES;
              return;
            }
            for (NSView *sub in v.subviews) {
              walk(sub);
            }
          };
          walk(root);
          return found;
        };

        RCTNveMacComponentView *studioWriter =
            [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 960, 640)];
        NSError *studioErr = nil;
        assert([studioWriter loadNveDylibAtPath:studioPath surfaceId:1 error:&studioErr]);
        assert([studioWriter applyPropsJson:@"{\"accent\":\"violet\",\"view\":\"metrics\"}"]);
        assert(hasLabelText(studioWriter, @"Live Metrics"));
        assert([studioWriter saveSnapshotAtPath:studioSnap schemaVersion:1]);

        RCTNveMacComponentView *studioCold =
            [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 960, 640)];
        uint64_t studioRestoreUs = 0;
        assert([studioCold replayFrame0SnapshotAtPath:studioSnap
                                expectedSchemaVersion:1
                                        restoreTimeUs:&studioRestoreUs]);
        std::cout << "[macos_shell_test] Cold mmap Frame-0 restore (studio-workspace): "
                  << studioRestoreUs << " us\n";
        assert(hasLabelText(studioCold, @"Live Metrics"));

        studioErr = nil;
        assert([studioCold loadNveDylibAtPath:studioPath surfaceId:1 error:&studioErr]);
        assert(hasLabelText(studioCold, @"Live Metrics"));
        [[NSFileManager defaultManager] removeItemAtPath:studioSnap error:nil];
      }
    }

    // 9. Native AppKit Accessibility & TextInput / IME / Submit tests
    {
      RCTNveMacComponentView *a11yView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 320, 260)];
      assert([a11yView attachNveInterface:&A11yImeMacApp::get_interface surfaceId:42]);
      assert(A11yImeMacApp::last_instance != nullptr);
      A11yImeMacApp *mock = A11yImeMacApp::last_instance;

      NSView *btn = [a11yView nativeViewForNodeId:2];
      NSView *hdr = [a11yView nativeViewForNodeId:3];
      NSView *chk = [a11yView nativeViewForNodeId:4];
      NSView *hiddenA11y = [a11yView nativeViewForNodeId:5];
      NSTextField *input = (NSTextField *)[a11yView nativeViewForNodeId:6];
      NSView *scroll = [a11yView nativeViewForNodeId:7];

      assert(btn != nil && hdr != nil && chk != nil && hiddenA11y != nil && input != nil &&
             scroll != nil);

      // Button inferred role + explicit label/hint/testID/enabled
      assert(btn.isAccessibilityElement == YES);
      assert([btn.accessibilityRole isEqualToString:NSAccessibilityButtonRole]);
      assert([btn.accessibilityLabel isEqualToString:@"Save Document"]);
      assert([btn.accessibilityHelp isEqualToString:@"Persists your current work"]);
      assert([btn.accessibilityIdentifier isEqualToString:@"save-btn"]);
      assert(btn.isAccessibilityEnabled == YES);
      assert(btn.gestureRecognizers.count == 1);

      // Header with aria-label & data-testid
      assert(hdr.isAccessibilityElement == YES);
      assert([hdr.accessibilityRole isEqualToString:NSAccessibilityStaticTextRole]);
      assert([hdr.accessibilityLabel isEqualToString:@"Settings Header"]);
      assert([hdr.accessibilityIdentifier isEqualToString:@"hdr-1"]);

      // Explicit checkbox role + accessibilityValue
      assert(chk.isAccessibilityElement == YES);
      assert([chk.accessibilityRole isEqualToString:NSAccessibilityCheckBoxRole]);
      assert([chk.accessibilityLabel isEqualToString:@"Enable Sync"]);
      assert([chk.accessibilityValue isEqual:@"checked"]);

      // Explicit role="none" hides element from accessibility tree
      assert(hiddenA11y.isAccessibilityElement == NO);

      // Inferred ScrollArea role
      assert([scroll.accessibilityRole isEqualToString:NSAccessibilityScrollAreaRole]);

      // TextInput: placeholderString, a11y metadata, plain NSTextField initially
      assert([input isKindOfClass:[NSTextField class]]);
      assert(![input isKindOfClass:[NSSecureTextField class]]);
      assert([input.accessibilityRole isEqualToString:NSAccessibilityTextFieldRole]);
      assert([input.accessibilityLabel isEqualToString:@"Display Name"]);
      assert([input.accessibilityHelp isEqualToString:@"Type your display name"]);
      assert([input.accessibilityIdentifier isEqualToString:@"name-input"]);
      assert([input.placeholderString isEqualToString:@"Enter name..."]);
      assert([input.stringValue isEqualToString:@"init"]);

      // Focus -> TextChange (UTF-8 CJK/accented) -> Return-key Submit -> Blur
      id<NSTextFieldDelegate> textDelegate = (id<NSTextFieldDelegate>)a11yView;
      [textDelegate controlTextDidBeginEditing:
                        [NSNotification notificationWithName:NSControlTextDidBeginEditingNotification
                                                      object:input]];
      assert(mock->focus_count == 1);

      // Marked-text IME safety: live composition on field editor MUST NOT dispatch TextChange or Submit
      NSTextView *markedEditor = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, 180, 24)];
      [markedEditor setMarkedText:@"にほん"
                    selectedRange:NSMakeRange(3, 0)
                 replacementRange:NSMakeRange(NSNotFound, 0)];
      assert(markedEditor.hasMarkedText == YES);
      input.stringValue = @"にほん";
      [textDelegate controlTextDidChange:
                        [NSNotification notificationWithName:NSControlTextDidChangeNotification
                                                      object:input
                                                    userInfo:@{@"NSFieldEditor" : markedEditor}]];
      assert(mock->text_change_count == 0);
      assert([textDelegate control:input
                          textView:markedEditor
               doCommandBySelector:@selector(insertNewline:)] == NO);
      assert(mock->submit_count == 0);

      // Commit IME text (unmarked) -> dispatches UTF-8 TextChange + Return-key Submit
      [markedEditor unmarkText];
      assert(markedEditor.hasMarkedText == NO);
      input.stringValue = @"東京 café";
      [textDelegate controlTextDidChange:
                        [NSNotification notificationWithName:NSControlTextDidChangeNotification
                                                      object:input
                                                    userInfo:@{@"NSFieldEditor" : markedEditor}]];
      assert(mock->text_change_count == 1);
      assert(mock->last_text == "東京 café");

      assert([textDelegate control:input
                          textView:markedEditor
               doCommandBySelector:@selector(insertNewline:)] == YES);
      assert(mock->submit_count == 1);
      assert(mock->last_submit == "東京 café");

      [textDelegate controlTextDidEndEditing:
                        [NSNotification notificationWithName:NSControlTextDidEndEditingNotification
                                                      object:input]];
      assert(mock->blur_count == 1);

      // Toggle disabled=true on button and secureTextEntry=true on TextInput
      NSDictionary<NSString *, id> *toggleProps = @{@"disabled" : @YES, @"secure" : @YES};
      assert([a11yView applyPropsDictionary:toggleProps]);
      assert(btn.isAccessibilityEnabled == NO);
      assert(btn.gestureRecognizers.count == 0);

      NSView *secureField = [a11yView nativeViewForNodeId:6];
      assert([secureField isKindOfClass:[NSSecureTextField class]]);
      assert([((NSTextField *)secureField).placeholderString isEqualToString:@"Enter name..."]);
      assert([secureField.accessibilityIdentifier isEqualToString:@"name-input"]);
    }

    // 10. First-class Image (PNG data URI + file:// + bounded LRU + all 4 resizeModes),
    //     NSScrollView bounds-change -> zero-copy EventKind::Scroll, and O(changed) incremental
    //     NveSceneView reconciliation!
    {
      // 10a. Bounded-LRU cache eviction & invalid URI handling.
      nve::apple::AppleImageCache smallCache(2u);
      constexpr const char *kMayaPng =
          "data:image/png;base64,"
          "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";
      constexpr const char *kLiamPng =
          "data:image/png;base64,"
          "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAg9gVSsGEXB8u5fKHggFOB1JD4MAGcqfXUz+ORAAAAAAElFTkSuQmCC";
      constexpr const char *kSaraPng =
          "data:image/png;base64,"
          "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAhWPTGsGEWB0tE4FDwQCvA6Eh8GAE5RZ/09pCRNAAAAAElFTkSuQmCC";
      NSImage *imgA1 = smallCache.image_for_source(kMayaPng);
      NSImage *imgA2 = smallCache.image_for_source(kMayaPng);
      assert(imgA1 != nil && imgA1 == imgA2);
      assert(approx_eq(imgA1.size.width, 8.0, 0.01));
      assert(approx_eq(imgA1.size.height, 8.0, 0.01));
      assert(smallCache.size() == 1u);
      NSImage *imgB = smallCache.image_for_source(kLiamPng);
      assert(imgB != nil && smallCache.size() == 2u);
      // Touch Maya so Liam becomes LRU, then insert Sara -> evicts Liam, keeps capacity == 2.
      assert(smallCache.image_for_source(kMayaPng) == imgA1);
      NSImage *imgC = smallCache.image_for_source(kSaraPng);
      assert(imgC != nil && smallCache.size() == 2u);
      assert(smallCache.image_for_source(kMayaPng) == imgA1);
      assert(smallCache.image_for_source("data:image/png;base64,!!!not-png!!!") == nil);
      assert(smallCache.image_for_source("file:///nonexistent/nve_missing_avatar.png") == nil);

      // Write real 8x8 PNG to temp file to verify file:// loading.
      NSString *tmpPngPath =
          [NSTemporaryDirectory() stringByAppendingPathComponent:
                                      [NSString stringWithFormat:@"nve_avatar_%d.png", getpid()]];
      NSData *rawPngBytes = [[NSData alloc]
          initWithBase64EncodedString:
              @"iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC"
                              options:0];
      assert(rawPngBytes != nil && [rawPngBytes writeToFile:tmpPngPath atomically:YES]);
      NSString *fileUrlStr = [NSURL fileURLWithPath:tmpPngPath].absoluteString;
      NSImage *fileImg = smallCache.image_for_source(fileUrlStr.UTF8String);
      assert(fileImg != nil);
      assert(approx_eq(fileImg.size.width, 8.0, 0.01));
      assert(approx_eq(fileImg.size.height, 8.0, 0.01));

      // 10b. Live RCTNveMacComponentView test for Image + all 4 resizeModes + ScrollView + O(changed).
      RCTNveMacComponentView *sceneView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 320, 260)];
      assert([sceneView attachNveInterface:&ImageScrollIncrementalApp::get_interface surfaceId:55]);
      assert(ImageScrollIncrementalApp::last_instance != nullptr);
      ImageScrollIncrementalApp *app = ImageScrollIncrementalApp::last_instance;
      assert(sceneView.nveLastReconciledDirtyCount == 6u); // initial full sync of all 6 nodes

      NSView *imgRaw = [sceneView nativeViewForNodeId:2];
      NSScrollView *scrollView = (NSScrollView *)[sceneView nativeViewForNodeId:3];
      NSView *scrollContentChild = [sceneView nativeViewForNodeId:4];
      NSTextField *tickLabel = (NSTextField *)[sceneView nativeViewForNodeId:5];
      NSView *removableView = [sceneView nativeViewForNodeId:6];
      assert(imgRaw != nil && [imgRaw isKindOfClass:[NSImageView class]]);
      assert(scrollView != nil && [scrollView isKindOfClass:[NSScrollView class]]);
      assert(scrollContentChild != nil && tickLabel != nil && removableView != nil);

      NSImageView *imgView = (NSImageView *)imgRaw;
      assert(imgView.image != nil);
      assert(approx_eq(imgView.image.size.width, 8.0, 0.01));
      assert(approx_eq(imgView.image.size.height, 8.0, 0.01));
      // Default resizeMode = "cover"
      assert(imgView.imageScaling == NSImageScaleProportionallyUpOrDown);
      assert([imgView.layer.contentsGravity isEqualToString:kCAGravityResizeAspectFill]);
      assert(imgView.layer.masksToBounds == YES);

      // Verify all 4 resizeModes update NSImageView scaling & contentsGravity cleanly.
      NSDictionary<NSString *, id> *containProps = @{@"stage" : @2, @"resizeMode" : @"contain"};
      assert([sceneView applyPropsDictionary:containProps]);
      assert(imgView.imageScaling == NSImageScaleProportionallyUpOrDown);
      assert([imgView.layer.contentsGravity isEqualToString:kCAGravityResizeAspect]);

      NSDictionary<NSString *, id> *stretchProps = @{@"stage" : @2, @"resizeMode" : @"stretch"};
      assert([sceneView applyPropsDictionary:stretchProps]);
      assert(imgView.imageScaling == NSImageScaleAxesIndependently);
      assert([imgView.layer.contentsGravity isEqualToString:kCAGravityResize]);

      NSDictionary<NSString *, id> *centerProps = @{@"stage" : @2, @"resizeMode" : @"center"};
      assert([sceneView applyPropsDictionary:centerProps]);
      assert(imgView.imageScaling == NSImageScaleNone);
      assert([imgView.layer.contentsGravity isEqualToString:kCAGravityCenter]);
      assert(imgView.layer.masksToBounds == YES);

      NSDictionary<NSString *, id> *coverFileProps = @{
        @"stage" : @2,
        @"resizeMode" : @"cover",
        @"src" : fileUrlStr,
      };
      assert([sceneView applyPropsDictionary:coverFileProps]);
      assert(imgView.image != nil);
      assert(approx_eq(imgView.image.size.width, 8.0, 0.01));
      assert(approx_eq(imgView.image.size.height, 8.0, 0.01));
      assert([imgView.layer.contentsGravity isEqualToString:kCAGravityResizeAspectFill]);
      [[NSFileManager defaultManager] removeItemAtPath:tmpPngPath error:nil];

      // 10c. NSScrollView bounds change dispatches zero-copy EventKind::Scroll with exact (x, y).
      assert(scrollView.contentView.postsBoundsChangedNotifications == YES);
      assert(app->scroll_count == 0);
      [scrollView.contentView scrollToPoint:NSMakePoint(14.5, 62.25)];
      [scrollView reflectScrolledClipView:scrollView.contentView];
      assert(app->scroll_count >= 1);
      assert(approx_eq(app->last_scroll_x, 14.5, 0.01));
      assert(approx_eq(app->last_scroll_y, 62.25, 0.01));
      assert(app->last_scroll_cw >= 300.0f);
      assert(app->last_scroll_ch >= 400.0f);

      // 10d. Incremental O(changed) NveSceneView reconciliation updates ONLY the 1 dirty node!
      NSDictionary<NSString *, id> *tickProps = @{@"stage" : @1, @"counterText" : @"Tick 1"};
      assert([sceneView applyPropsDictionary:tickProps]);
      assert(sceneView.nveLastReconciledDirtyCount == 1u);
      assert(sceneView.nveLastReconciledRemovedCount == 0u);
      assert([tickLabel.stringValue isEqualToString:@"Tick 1"]);
      assert([sceneView nativeViewForNodeId:2] == imgRaw);
      assert([sceneView nativeViewForNodeId:3] == scrollView);
      assert([sceneView nativeViewForNodeId:5] == tickLabel);

      // Incremental removal destroys ONLY the 1 removed node!
      assert([sceneView applyPropsDictionary:@{@"stage" : @3}]);
      assert(sceneView.nveLastReconciledRemovedCount == 1u);
      assert([sceneView nativeViewForNodeId:6] == nil);
      assert(removableView.superview == nil);
      assert([sceneView nativeViewForNodeId:5] == tickLabel);
    }

    // 11. Shared Apple CoreGraphics `NveAppleCanvasView` + `NVE_DRAW1` sparkline offscreen
    //     CGBitmapContext pixel verification + caching + corrupt-buffer survival!
    {
      RCTNveMacComponentView *canvasHostView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 260, 100)];
      assert([canvasHostView attachNveInterface:&CanvasSparklineApp::get_interface surfaceId:88]);
      assert(CanvasSparklineApp::last_instance != nullptr);

      NSView *rawCanvas = [canvasHostView nativeViewForNodeId:2u];
      assert(rawCanvas != nil && [rawCanvas isKindOfClass:[NveAppleCanvasView class]]);
      NveAppleCanvasView *canvasView = (NveAppleCanvasView *)rawCanvas;
      assert(canvasView.isFlipped == YES);
      assert(approx_eq(canvasView.frame.size.width, 240.0, 0.5));
      assert(approx_eq(canvasView.frame.size.height, 80.0, 0.5));

      const std::span<const uint8_t> cachedBytes = [canvasView drawListBytes];
      assert(cachedBytes.size() == CanvasSparklineApp::last_instance->draw_bytes.size());
      assert(canvasView.redrawRequestCount == 1u);

      // Identical payload must NOT trigger a redundant setNeedsDisplay.
      assert([canvasView updateDrawListBytes:cachedBytes] == NO);
      assert(canvasView.redrawRequestCount == 1u);

      // Render NveAppleCanvasView into a real 240x80 offscreen CGBitmapContext via drawRect:.
      constexpr size_t kW = 240u;
      constexpr size_t kH = 80u;
      std::vector<uint8_t> rgba(kW * kH * 4u, 0u);
      CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
      assert(colorSpace != nullptr);
      CGContextRef bitmapCtx = CGBitmapContextCreate(
          rgba.data(),
          kW,
          kH,
          8u,
          kW * 4u,
          colorSpace,
          static_cast<uint32_t>(kCGImageAlphaPremultipliedLast) |
              static_cast<uint32_t>(kCGBitmapByteOrder32Big));
      assert(bitmapCtx != nullptr);

      auto pixel_at = [&](size_t x, size_t y) -> std::array<uint8_t, 4> {
        const size_t idx = (y * kW + x) * 4u;
        return {rgba[idx], rgba[idx + 1u], rgba[idx + 2u], rgba[idx + 3u]};
      };

      // Apply AppKit flipped top-left coordinate transform matching NSView.isFlipped == YES.
      CGContextSaveGState(bitmapCtx);
      CGContextTranslateCTM(bitmapCtx, 0.0, static_cast<CGFloat>(kH));
      CGContextScaleCTM(bitmapCtx, 1.0, -1.0);
      NSGraphicsContext *nsCtx = [NSGraphicsContext graphicsContextWithCGContext:bitmapCtx flipped:YES];
      [NSGraphicsContext saveGraphicsState];
      [NSGraphicsContext setCurrentContext:nsCtx];
      [canvasView drawRect:canvasView.bounds];
      [NSGraphicsContext restoreGraphicsState];
      CGContextRestoreGState(bitmapCtx);

      // Verify painted card background (15, 23, 42, 255) in untouched interior region.
      const auto bgPx = pixel_at(20u, 22u);
      assert(bgPx[0] == 15u && bgPx[1] == 23u && bgPx[2] == 42u && bgPx[3] == 255u);

      // Verify exact non-background sparkline stroke & terminal dot pixels (56, 189, 248).
      // Rising sparkline [10..95] has terminal point at (pad_x + plot_w, pad_y) = (~230, ~11)
      // in top-left coordinates (proving flipped orientation!).
      size_t cyanStrokePixels = 0u;
      size_t nonBgPixels = 0u;
      for (size_t y = 0u; y < kH; ++y) {
        for (size_t x = 0u; x < kW; ++x) {
          const auto px = pixel_at(x, y);
          if (px[3] > 0u && !(px[0] == 15u && px[1] == 23u && px[2] == 42u)) {
            ++nonBgPixels;
          }
          if (px[2] > 200u && px[1] > 140u && px[0] < 100u && px[3] == 255u) {
            ++cyanStrokePixels;
          }
        }
      }
      assert(nonBgPixels > 100u);
      assert(cyanStrokePixels > 80u);

      // Terminal dot at top-right (230, 11) must be exact stroke cyan (56, 189, 248, 255),
      // while bottom-right (230, 68) is NOT the terminal dot!
      const auto topDotPx = pixel_at(230u, 11u);
      assert(topDotPx[0] == 56u && topDotPx[1] == 189u && topDotPx[2] == 248u && topDotPx[3] == 255u);

      // Verify ClipRect + RestoreClip semantics via direct base64 string update through RCTNveMacComponentView!
      nve::DrawListEncoder clipEnc;
      clipEnc.clear(nve::DrawColorRgba::rgb(10, 10, 10))
          .clip_rect(0.0f, 0.0f, 40.0f, 40.0f)
          .fill_rect(0.0f, 0.0f, 120.0f, 80.0f, nve::DrawColorRgba::rgb(250, 20, 20))
          .restore_clip()
          .fill_rect(100.0f, 10.0f, 20.0f, 20.0f, nve::DrawColorRgba::rgb(20, 240, 40));
      const std::string clipB64 = clipEnc.to_attr_string();
      NSString *clipB64Ns = [NSString stringWithUTF8String:clipB64.c_str()];
      NSDictionary<NSString *, id> *clipProps = @{@"stage" : @2, @"b64" : clipB64Ns};
      assert([canvasHostView applyPropsDictionary:clipProps]);
      assert(canvasHostView.nveLastReconciledDirtyCount == 1u);
      assert(canvasView.redrawRequestCount == 2u);

      std::fill(rgba.begin(), rgba.end(), 0u);
      CGContextSaveGState(bitmapCtx);
      CGContextTranslateCTM(bitmapCtx, 0.0, static_cast<CGFloat>(kH));
      CGContextScaleCTM(bitmapCtx, 1.0, -1.0);
      const auto clipRes = [canvasView renderIntoContext:bitmapCtx bounds:canvasView.bounds];
      CGContextRestoreGState(bitmapCtx);
      assert(clipRes.ok() && clipRes.ops_decoded == 5u);
      // Inside clip (20, 20) -> red (250, 20, 20)
      const auto insideClip = pixel_at(20u, 20u);
      assert(insideClip[0] == 250u && insideClip[1] == 20u && insideClip[2] == 20u);
      // Clipped out region (60, 20) -> remains clear background (10, 10, 10)
      const auto outsideClip = pixel_at(60u, 20u);
      assert(outsideClip[0] == 10u && outsideClip[1] == 10u && outsideClip[2] == 10u);
      // After RestoreClip (110, 20) -> green (20, 240, 40)
      const auto afterRestore = pixel_at(110u, 20u);
      assert(afterRestore[0] == 20u && afterRestore[1] == 240u && afterRestore[2] == 40u);

      // Corrupt-buffer survival: truncated, bad magic, NaN float, and unbalanced ClipRect.
      const std::vector<uint8_t> corruptTrunc = {'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1', 1, 0};
      NSData *corruptData = [NSData dataWithBytes:corruptTrunc.data() length:corruptTrunc.size()];
      NSDictionary<NSString *, id> *corruptProps = @{@"stage" : @1, @"bytes" : corruptData};
      assert([canvasHostView applyPropsDictionary:corruptProps]);
      assert(canvasView.redrawRequestCount == 3u);
      CGContextSaveGState(bitmapCtx);
      CGContextTranslateCTM(bitmapCtx, 0.0, static_cast<CGFloat>(kH));
      CGContextScaleCTM(bitmapCtx, 1.0, -1.0);
      const auto corruptRes = [canvasView renderIntoContext:bitmapCtx bounds:canvasView.bounds];
      CGContextRestoreGState(bitmapCtx);
      assert(!corruptRes.ok() && corruptRes.error == nve::DrawListError::Truncated);

      // Unbalanced ClipRect (never closed) + extra RestoreClip must not corrupt CGContext GState!
      nve::DrawListEncoder unbalEnc;
      unbalEnc.restore_clip()
          .clip_rect(0.0f, 0.0f, 20.0f, 20.0f)
          .fill_rect(2.0f, 2.0f, 10.0f, 10.0f, nve::DrawColorRgba::rgb(255, 255, 0));
      const std::vector<uint8_t> unbalBytes = unbalEnc.finish();
      CGContextSaveGState(bitmapCtx);
      CGContextTranslateCTM(bitmapCtx, 0.0, static_cast<CGFloat>(kH));
      CGContextScaleCTM(bitmapCtx, 1.0, -1.0);
      const auto unbalRes =
          nve::apple::render_draw_list_to_cg_context(bitmapCtx, canvasView.bounds, unbalBytes);
      assert(unbalRes.ok());
      // Prove GState was fully restored: painting at (180, 50) (outside the unclosed 20x20 clip!) succeeds.
      nve::DrawListEncoder recoveryEnc;
      recoveryEnc.fill_rect(170.0f, 40.0f, 20.0f, 20.0f, nve::DrawColorRgba::rgb(200, 100, 250));
      const std::vector<uint8_t> recBytes = recoveryEnc.finish();
      assert(nve::apple::render_draw_list_to_cg_context(bitmapCtx, canvasView.bounds, recBytes).ok());
      CGContextRestoreGState(bitmapCtx);
      const auto recPx = pixel_at(180u, 50u);
      assert(recPx[0] == 200u && recPx[1] == 100u && recPx[2] == 250u && recPx[3] == 255u);

      CGContextRelease(bitmapCtx);
      CGColorSpaceRelease(colorSpace);
    }

    // 12. Paired thin-client mode on RCTNveMacComponentView (zero local dylib, live TCP NVT1/NVD1
    //     Keyframe + CommandOpDelta + dispatch_async main-queue wake + upstream InputEvents = 11).
    {
      auto build_frame = [](uint64_t fid, const std::string &title) {
        nve::CommandBufferEncoder enc(1u, fid);
        enc.create_node(2u, NVE_COMP_VIEW);
        enc.create_node(3u, NVE_COMP_TEXT);
        enc.create_node(4u, NVE_COMP_PRESSABLE);
        enc.set_children(NVE_ROOT_NODE_ID, {2u});
        enc.set_children(2u, {3u, 4u});
        nve::LayoutProps box{};
        box.width = {1u, 300.0f};
        box.height = {1u, 180.0f};
        enc.set_layout(2u, box);
        nve::LayoutProps btn{};
        btn.width = {1u, 140.0f};
        btn.height = {1u, 40.0f};
        enc.set_layout(4u, btn);
        enc.set_text(3u, title);
        enc.set_event_mask(4u, nve::apple::kEventMaskPress);
        enc.commit(true);
        const NveBufferSlice s = enc.finish();
        return std::vector<uint8_t>(s.ptr, s.ptr + s.len);
      };

      const std::string token = "mac-pair-token";
      const auto f1 = build_frame(1u, "Mac Paired V1");
      const auto f2 = build_frame(2u, "Mac Paired V2");

      int lfd = ::socket(AF_INET, SOCK_STREAM, 0);
      assert(lfd >= 0);
      int one = 1;
      (void)::setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
      sockaddr_in addr{};
      addr.sin_family = AF_INET;
      addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
      addr.sin_port = 0;
      assert(::bind(lfd, reinterpret_cast<sockaddr *>(&addr), sizeof(addr)) == 0);
      socklen_t alen = sizeof(addr);
      assert(::getsockname(lfd, reinterpret_cast<sockaddr *>(&addr), &alen) == 0);
      const uint16_t port = ntohs(addr.sin_port);
      assert(::listen(lfd, 2) == 0);

      auto read_exact = [](int fd, uint8_t *buf, size_t len) -> bool {
        size_t off = 0u;
        const auto dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
        while (off < len) {
          if (std::chrono::steady_clock::now() >= dl) return false;
          pollfd pfd{fd, POLLIN, 0};
          if (::poll(&pfd, 1u, 25) <= 0) continue;
          const ssize_t n = ::recv(fd, buf + off, len - off, 0);
          if (n <= 0) return false;
          off += static_cast<size_t>(n);
        }
        return true;
      };
      auto read_msg = [&](int fd, std::vector<uint8_t> &out) -> bool {
        uint8_t hdr[4]{};
        if (!read_exact(fd, hdr, 4u)) return false;
        const uint32_t len = static_cast<uint32_t>(hdr[0]) | (static_cast<uint32_t>(hdr[1]) << 8u) |
                             (static_cast<uint32_t>(hdr[2]) << 16u) |
                             (static_cast<uint32_t>(hdr[3]) << 24u);
        out.resize(len);
        return read_exact(fd, out.data(), len);
      };
      auto write_msg = [](int fd, std::span<const uint8_t> p) -> bool {
        uint8_t hdr[4]{
            static_cast<uint8_t>(p.size() & 0xFFu),
            static_cast<uint8_t>((p.size() >> 8u) & 0xFFu),
            static_cast<uint8_t>((p.size() >> 16u) & 0xFFu),
            static_cast<uint8_t>((p.size() >> 24u) & 0xFFu),
        };
        return ::send(fd, hdr, 4u, 0) == 4 &&
               ::send(fd, p.data(), p.size(), 0) == static_cast<ssize_t>(p.size());
      };

      std::atomic<bool> saw_press{false};
      std::thread srv([&]() {
        int cfd = ::accept(lfd, nullptr, nullptr);
        assert(cfd >= 0);
        std::vector<uint8_t> msg;
        assert(read_msg(cfd, msg));
        nve::TransportEnvelopeView env{};
        assert(nve::pair_wire::parse_envelope(msg, env) &&
               env.kind == nve::TransportMsgKind::Hello);
        nve::HelloPayload hp{};
        assert(nve::pair_wire::decode_hello_payload(env.payload, hp));
        const uint64_t sn = 0xAABBCCDDEEFF0011ULL;
        const uint32_t sid = nve::pair_wire::derive_session_id(token, 1u, hp.client_nonce, sn);
        nve::HelloAckPayload ack{};
        ack.schema_version = 1u;
        ack.server_max_window = 64u;
        ack.server_nonce = sn;
        const auto ack_b = nve::pair_wire::encode_hello_ack_payload(ack);
        assert(write_msg(
            cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::HelloAck, sid, ack_b)));

        const auto kf1 = nve::pair_wire::encode_nvd_packet(
            nve::DeltaPacketKind::Keyframe, 0u, 1u, 0u, 0u, f1);
        assert(write_msg(
            cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, kf1)));

        const auto d2 = nve::pair_wire::encode_command_op_delta(f1, f2);
        assert(d2.has_value());
        const auto p2 = nve::pair_wire::encode_nvd_packet(
            nve::DeltaPacketKind::CommandOpDelta, 0u, 2u, 1u, 0u, *d2);
        assert(write_msg(
            cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, p2)));

        while (read_msg(cfd, msg)) {
          nve::TransportEnvelopeView in{};
          if (nve::pair_wire::parse_envelope(msg, in) && in.session_id == sid &&
              in.kind == nve::TransportMsgKind::InputEvents) {
            std::vector<nve::DecodedEvent> dec;
            if (nve::decode_event_ring(
                    NveBufferSlice{in.payload.data(), in.payload.size()}, dec) &&
                dec.size() == 1u && dec[0].kind == nve::EventKind::Press && dec[0].node_id == 4u) {
              saw_press.store(true);
              break;
            }
          }
        }
        ::close(cfd);
        ::close(lfd);
      });

      RCTNveMacComponentView *pairedView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 320, 200)];
      NveAppleComponentHost *pairHost = [NveAppleComponentHost hostForRootView:pairedView];
      assert(pairHost != nil);

      NSString *ep = [NSString stringWithFormat:@"127.0.0.1:%u", port];
      NSError *pairErr = nil;
      assert([pairHost connectPairedEndpoint:ep
                                       token:@"mac-pair-token"
                                   transport:@"tcp"
                                   surfaceId:1u
                                       error:&pairErr]);
      assert(pairHost.isPaired == YES && pairHost.isPairedConnected == YES);

      const auto dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
      while (std::chrono::steady_clock::now() < dl) {
        [[NSRunLoop mainRunLoop]
            runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
        (void)[pairHost pollPairedFrames];
        NSView *lbl = [pairedView nativeViewForNodeId:3u];
        if ([lbl isKindOfClass:[NSTextField class]] &&
            [((NSTextField *)lbl).stringValue isEqualToString:@"Mac Paired V2"]) {
          break;
        }
      }

      NSView *lblView = [pairedView nativeViewForNodeId:3u];
      assert(lblView != nil && [lblView isKindOfClass:[NSTextField class]]);
      assert([((NSTextField *)lblView).stringValue isEqualToString:@"Mac Paired V2"]);

      [pairedView simulatePressOnNodeId:4u atPoint:NSMakePoint(12.0, 14.0)];
      const auto evDl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
      while (!saw_press.load() && std::chrono::steady_clock::now() < evDl) {
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
      }
      assert(saw_press.load());

      [pairHost disconnectPaired];
      assert(pairHost.isPaired == NO);
      srv.join();
    }

    // 13. NativeViewRegistry + real AppKit NSVisualEffectView (BlurSurface) & WKWebView (WebView)
    //     driven by zero-JSON NVNP props and NVNC DispatchCommand frames through FabricHost.
    {
      auto push_u16 = [](std::vector<uint8_t> &b, uint16_t v) {
        b.push_back(static_cast<uint8_t>(v & 0xFFu));
        b.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
      };
      auto push_u32 = [](std::vector<uint8_t> &b, uint32_t v) {
        for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>((v >> (i * 8)) & 0xFFu));
      };
      auto push_f32 = [](std::vector<uint8_t> &b, float v) {
        uint32_t bits = 0u;
        std::memcpy(&bits, &v, sizeof(bits));
        for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>((bits >> (i * 8)) & 0xFFu));
      };
      auto encode_blur_props = [&](nve::BlurMaterial mat, float intensity, std::optional<uint32_t> tint) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'P', 1u, 3u, 0u};
        push_u16(buf, 1u); buf.push_back(2u); push_u32(buf, static_cast<uint32_t>(mat));
        push_u16(buf, 2u); buf.push_back(5u); push_f32(buf, intensity);
        push_u16(buf, 3u);
        if (tint.has_value()) { buf.push_back(2u); push_u32(buf, *tint); } else { buf.push_back(0u); }
        return buf;
      };
      auto encode_webview_props = [&](std::string_view url, bool allows_bf) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'P', 1u, 2u, 0u};
        push_u16(buf, 1u); buf.push_back(7u); push_u32(buf, static_cast<uint32_t>(url.size()));
        buf.insert(buf.end(), url.begin(), url.end());
        push_u16(buf, 2u); buf.push_back(1u); buf.push_back(allows_bf ? 1u : 0u);
        return buf;
      };
      auto encode_cmd_no_args = [&](uint16_t op) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op);
        buf.push_back(0u);
        return buf;
      };
      auto encode_cmd_str = [&](uint16_t op, std::string_view arg) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op);
        buf.push_back(1u);
        buf.push_back(7u);
        push_u32(buf, static_cast<uint32_t>(arg.size()));
        buf.insert(buf.end(), arg.begin(), arg.end());
        return buf;
      };

      nve::NativeViewRegistry registry;
      nve::apple::register_apple_native_views(registry);
      assert(registry.supports(nve::kNativeKindBlurSurface));
      assert(registry.supports(nve::kNativeKindBlurAlias));
      assert(registry.supports(nve::kNativeKindWebView));
      assert(registry.supports(nve::kNativeKindWebViewAlias));
      assert(!nve::apple::is_safe_apple_webview_url("javascript:alert(1)"));
      assert(nve::apple::is_safe_apple_webview_url("https://nve.dev/docs"));

      NveSurfaceConfig cfg{.surface_id = 1u, .width = 480.0f, .height = 320.0f, .scale_factor = 2.0f};
      NveFabricHostHandle host = nve_fabric_host_create(cfg);
      assert(host != nullptr);
      nve::apple::bind_apple_native_views_to_fabric_host(host, registry);

      // Frame 1: mount BlurSurface (node 20) and WebView (node 21) via NVNP binary props.
      const auto blur1 = encode_blur_props(nve::BlurMaterial::Thick, 0.75f, 0x204060CCu);
      const auto wv1 = encode_webview_props("https://nve.dev/home", true);
      nve::CommandBufferEncoder enc1(1u, 1u);
      enc1.create_node(20u, NVE_COMP_CUSTOM, nve::kNativeKindBlurSurface);
      enc1.set_raw_prop_string(20u, nve::kNativeViewKindPropKey, nve::kNativeKindBlurSurface);
      enc1.set_raw_prop_bytes(20u, nve::kNativeViewPropsPropKey, blur1);
      enc1.create_node(21u, NVE_COMP_CUSTOM, nve::kNativeKindWebView);
      enc1.set_raw_prop_string(21u, nve::kNativeViewKindPropKey, nve::kNativeKindWebView);
      enc1.set_raw_prop_bytes(21u, nve::kNativeViewPropsPropKey, wv1);
      enc1.set_children(NVE_ROOT_NODE_ID, {20u, 21u});
      enc1.commit(true);
      assert(nve_fabric_host_submit_frame(host, enc1.finish(), nullptr) == 0);

      NSView *blurRaw = nve::apple::native_view_for_node(registry, 20u);
      assert(blurRaw != nil && [blurRaw isKindOfClass:[NSVisualEffectView class]]);
      assert([blurRaw isKindOfClass:[NveAppleBlurSurfaceView class]]);
      NveAppleBlurSurfaceView *blurView = (NveAppleBlurSurfaceView *)blurRaw;
      assert(blurView.nveBlurMaterial == nve::BlurMaterial::Thick);
      assert(blurView.material == NSVisualEffectMaterialHeaderView);
      assert(std::fabs(blurView.nveBlurIntensity - 0.75f) < 1e-4f);
      assert(blurView.nveHasTint == YES && blurView.nveTintRgba == 0x204060CCu);

      NSView *wvRaw = nve::apple::native_view_for_node(registry, 21u);
      assert(wvRaw != nil && [wvRaw isKindOfClass:[WKWebView class]]);
      assert([wvRaw isKindOfClass:[NveAppleWebView class]]);
      NveAppleWebView *webView = (NveAppleWebView *)wvRaw;
      assert([webView.nveLastRequestedUrl isEqualToString:@"https://nve.dev/home"]);
      assert(webView.nveAllowsBackForward == YES && webView.allowsBackForwardNavigationGestures == YES);
      assert(webView.nveNavigationCount == 1u);

      // Frame 2: update BlurSurface (clear tint, Chrome material) + dispatch NVNC WebView commands.
      const auto blur2 = encode_blur_props(nve::BlurMaterial::Chrome, 0.9f, std::nullopt);
      nve::CommandBufferEncoder enc2(1u, 2u);
      enc2.set_raw_prop_bytes(20u, nve::kNativeViewPropsPropKey, blur2);
      enc2.dispatch_command(21u, "navigate", encode_cmd_str(1u, "javascript:alert(1)"));
      enc2.dispatch_command(21u, "navigate", encode_cmd_str(1u, "https://nve.dev/api"));
      enc2.dispatch_command(21u, "reload", encode_cmd_no_args(2u));
      enc2.dispatch_command(21u, "go_back", encode_cmd_no_args(3u));
      enc2.dispatch_command(21u, "go_forward", encode_cmd_no_args(4u));
      enc2.commit(true);
      assert(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);

      assert(blurView.nveBlurMaterial == nve::BlurMaterial::Chrome);
      assert(blurView.material == NSVisualEffectMaterialTitlebar);
      assert(blurView.nveHasTint == NO);
      assert([webView.nveLastRequestedUrl isEqualToString:@"https://nve.dev/api"]);
      assert(webView.nveNavigationCount == 2u);
      assert(webView.nveReloadCount == 1u);
      assert(webView.nveGoBackCount == 1u);
      assert(webView.nveGoForwardCount == 1u);

      // Clean unmount via registry
      assert(registry.unmount(20u));
      assert(registry.unmount(21u));
      assert(registry.mounted_count() == 0u);
      nve_fabric_host_destroy(host);
    }

    // 14. Real Apple AVFoundation camera adapter ("nve.camera.preview") + NVNP/NVNC/NVCF
    //     byte-for-byte codec checks + CVPixelBuffer frame descriptor + VisionCamera v5 bridge.
    {
      auto push_u16 = [](std::vector<uint8_t> &b, uint16_t v) {
        b.push_back(static_cast<uint8_t>(v & 0xFFu));
        b.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
      };
      auto push_u32 = [](std::vector<uint8_t> &b, uint32_t v) {
        for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>((v >> (i * 8)) & 0xFFu));
      };
      auto push_f32 = [](std::vector<uint8_t> &b, float v) {
        uint32_t bits = 0u;
        std::memcpy(&bits, &v, sizeof(bits));
        for (int i = 0; i < 4; ++i) b.push_back(static_cast<uint8_t>((bits >> (i * 8)) & 0xFFu));
      };
      auto encode_camera_props = [&](std::optional<std::string_view> device_id,
                                     uint32_t facing,
                                     uint32_t torch,
                                     float zoom,
                                     uint32_t fps,
                                     bool hdr,
                                     uint32_t stabilization,
                                     bool enable_fp,
                                     bool audio,
                                     bool active) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'P', 1u, 10u, 0u};
        push_u16(buf, 1u);
        if (device_id.has_value()) {
          buf.push_back(7u);
          push_u32(buf, static_cast<uint32_t>(device_id->size()));
          buf.insert(buf.end(), device_id->begin(), device_id->end());
        } else {
          buf.push_back(0u);
        }
        push_u16(buf, 2u); buf.push_back(2u); push_u32(buf, facing);
        push_u16(buf, 3u); buf.push_back(2u); push_u32(buf, torch);
        push_u16(buf, 4u); buf.push_back(5u); push_f32(buf, zoom);
        push_u16(buf, 5u); buf.push_back(2u); push_u32(buf, fps);
        push_u16(buf, 6u); buf.push_back(1u); buf.push_back(hdr ? 1u : 0u);
        push_u16(buf, 7u); buf.push_back(2u); push_u32(buf, stabilization);
        push_u16(buf, 8u); buf.push_back(1u); buf.push_back(enable_fp ? 1u : 0u);
        push_u16(buf, 9u); buf.push_back(1u); buf.push_back(audio ? 1u : 0u);
        push_u16(buf, 10u); buf.push_back(1u); buf.push_back(active ? 1u : 0u);
        return buf;
      };
      auto encode_cmd_u32 = [&](uint16_t op, uint32_t a0) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op); buf.push_back(1u); buf.push_back(2u); push_u32(buf, a0);
        return buf;
      };
      auto encode_cmd_u32_u32 = [&](uint16_t op, uint32_t a0, uint32_t a1) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op); buf.push_back(2u);
        buf.push_back(2u); push_u32(buf, a0);
        buf.push_back(2u); push_u32(buf, a1);
        return buf;
      };
      auto encode_cmd_f32 = [&](uint16_t op, float a0) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op); buf.push_back(1u); buf.push_back(5u); push_f32(buf, a0);
        return buf;
      };
      auto encode_cmd_f32_f32 = [&](uint16_t op, float a0, float a1) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op); buf.push_back(2u);
        buf.push_back(5u); push_f32(buf, a0);
        buf.push_back(5u); push_f32(buf, a1);
        return buf;
      };
      auto encode_cmd_str = [&](uint16_t op, std::string_view s) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op); buf.push_back(1u); buf.push_back(7u);
        push_u32(buf, static_cast<uint32_t>(s.size()));
        buf.insert(buf.end(), s.begin(), s.end());
        return buf;
      };
      auto encode_cmd_empty = [&](uint16_t op) {
        std::vector<uint8_t> buf = {'N', 'V', 'N', 'C', 1u};
        push_u16(buf, op); buf.push_back(0u);
        return buf;
      };

      // 14a. NVCF 40-byte wire layout + validation edge cases.
      nve::apple::CameraFrameDescriptor fd{};
      fd.width = 1920u;
      fd.height = 1080u;
      fd.stride_bytes = 7680u;
      fd.pixel_format = nve::apple::CameraPixelFormat::Bgra8888;
      fd.rotation_deg = 90u;
      fd.timestamp_ns = 0x1122334455667788ULL;
      fd.native_handle_id = 0xCAFEBABE12345678ULL;
      std::array<uint8_t, nve::apple::kCameraFrameDescriptorSize> wire{};
      assert(nve::apple::encode_camera_frame_descriptor(fd, wire) == nve::NativeViewCodecError::Ok);
      assert(wire[0] == 'N' && wire[1] == 'V' && wire[2] == 'C' && wire[3] == 'F');
      assert(wire[4] == 1u && wire[5] == 1u && wire[6] == 90u && wire[7] == 0u);
      nve::apple::CameraFrameDescriptor parsedFd{};
      assert(nve::apple::decode_camera_frame_descriptor(wire, parsedFd) == nve::NativeViewCodecError::Ok);
      assert(parsedFd.width == 1920u && parsedFd.height == 1080u && parsedFd.stride_bytes == 7680u);
      assert(parsedFd.pixel_format == nve::apple::CameraPixelFormat::Bgra8888);
      assert(parsedFd.rotation_deg == 90u && parsedFd.timestamp_ns == fd.timestamp_ns);
      assert(parsedFd.native_handle_id == fd.native_handle_id);

      auto badMagic = wire; badMagic[0] = 'X';
      assert(nve::apple::decode_camera_frame_descriptor(badMagic, parsedFd) == nve::NativeViewCodecError::InvalidMagic);
      auto badReserved = wire; badReserved[20] = 1u;
      assert(nve::apple::decode_camera_frame_descriptor(badReserved, parsedFd) == nve::NativeViewCodecError::InvalidEnum);
      auto badRot = wire; badRot[6] = 45u;
      assert(nve::apple::decode_camera_frame_descriptor(badRot, parsedFd) == nve::NativeViewCodecError::InvalidEnum);
      assert(nve::apple::decode_camera_frame_descriptor(std::span<const uint8_t>(wire.data(), 39u), parsedFd) ==
             nve::NativeViewCodecError::Truncated);

      // Invalid NVNP props (facing=0, fps=241, zoom=0.0f) rejected cleanly.
      {
        nve::NvnpDecoder badDec{};
        nve::apple::CameraPreviewProps badProps{};
        const auto badFacing = encode_camera_props(std::nullopt, 0u, 0u, 1.0f, 30u, false, 0u, false, false, true);
        assert(nve::NvnpDecoder::parse(badFacing, badDec) == nve::NativeViewCodecError::Ok);
        assert(nve::apple::decode_camera_preview_props(badDec, badProps) == nve::NativeViewCodecError::InvalidEnum);
        const auto badFps = encode_camera_props(std::nullopt, 1u, 0u, 1.0f, 241u, false, 0u, false, false, true);
        assert(nve::NvnpDecoder::parse(badFps, badDec) == nve::NativeViewCodecError::Ok);
        assert(nve::apple::decode_camera_preview_props(badDec, badProps) == nve::NativeViewCodecError::InvalidEnum);
        const auto badZoom = encode_camera_props(std::nullopt, 1u, 0u, 0.0f, 30u, false, 0u, false, false, true);
        assert(nve::NvnpDecoder::parse(badZoom, badDec) == nve::NativeViewCodecError::Ok);
        assert(nve::apple::decode_camera_preview_props(badDec, badProps) == nve::NativeViewCodecError::TypeMismatch);
      }

      // 14b. Mount NveAppleCameraView via NativeViewRegistry + FabricHost + CVPixelBuffer -> NVCF.
      nve::apple::set_vision_camera_apple_bridge(nil);
      nve::NativeViewRegistry registry;
      nve::apple::register_apple_native_views(registry);
      assert(registry.supports(nve::apple::kNativeKindCameraPreview));
      assert(registry.supports(nve::apple::kNativeKindCameraAlias));

      NveSurfaceConfig cfg{.surface_id = 1u, .width = 480.0f, .height = 320.0f, .scale_factor = 2.0f};
      NveFabricHostHandle host = nve_fabric_host_create(cfg);
      assert(host != nullptr);
      nve::apple::bind_apple_native_views_to_fabric_host(host, registry);

      const auto camProps1 = encode_camera_props(
          "cam-back-0", 1u, 2u, 2.5f, 60u, true, 2u, true, false, true);
      nve::CommandBufferEncoder enc1(1u, 1u);
      enc1.create_node(30u, NVE_COMP_CUSTOM, nve::apple::kNativeKindCameraPreview);
      enc1.set_raw_prop_string(30u, nve::kNativeViewKindPropKey, nve::apple::kNativeKindCameraPreview);
      enc1.set_raw_prop_bytes(30u, nve::kNativeViewPropsPropKey, camProps1);
      enc1.set_children(NVE_ROOT_NODE_ID, {30u});
      enc1.commit(true);
      assert(nve_fabric_host_submit_frame(host, enc1.finish(), nullptr) == 0);

      NSView *camRaw = nve::apple::native_view_for_node(registry, 30u);
      assert(camRaw != nil && [camRaw isKindOfClass:[NveAppleCameraView class]]);
      NveAppleCameraView *camView = (NveAppleCameraView *)camRaw;
      assert(camView.nveCaptureSession != nil);
      assert(camView.nvePreviewLayer != nil);
      assert([camView.nveDeviceId isEqualToString:@"cam-back-0"]);
      assert(camView.nveFacing == nve::apple::CameraPreviewFacing::Back);
      assert(camView.nveTorchMode == nve::apple::TorchMode::Auto);
      assert(std::fabs(camView.nveZoom - 2.5f) < 1e-4f);
      assert(camView.nveFps == 60u && camView.nveHdr == YES);
      assert(camView.nveStabilization == nve::apple::StabilizationMode::Cinematic);
      assert(camView.nveFrameProcessorEnabled == YES && camView.nveAudioEnabled == NO && camView.nveActive == YES);

      // Real CoreVideo BGRA & NV12 CVPixelBuffers emitting 40-byte NVCF descriptors.
      __block std::vector<uint8_t> capturedFrame;
      camView.nveFrameCallback = ^(const uint8_t *bytes, size_t len) {
        capturedFrame.assign(bytes, bytes + len);
      };
      CVPixelBufferRef bgraPb = nullptr;
      assert(CVPixelBufferCreate(
                 kCFAllocatorDefault, 64u, 48u, kCVPixelFormatType_32BGRA, nullptr, &bgraPb) == kCVReturnSuccess);
      assert([camView emitFrameFromPixelBuffer:bgraPb timestampNs:1234567890ULL rotationDeg:90u]);
      assert(capturedFrame.size() == nve::apple::kCameraFrameDescriptorSize);
      assert(camView.nveFrameCount == 1u && camView.nveLastFrameDescriptor.length == 40u);
      nve::apple::CameraFrameDescriptor decodedPb{};
      assert(nve::apple::decode_camera_frame_descriptor(capturedFrame, decodedPb) == nve::NativeViewCodecError::Ok);
      assert(decodedPb.width == 64u && decodedPb.height == 48u && decodedPb.stride_bytes >= 256u);
      assert(decodedPb.pixel_format == nve::apple::CameraPixelFormat::Bgra8888);
      assert(decodedPb.rotation_deg == 90u && decodedPb.timestamp_ns == 1234567890ULL);
      assert(decodedPb.native_handle_id == static_cast<uint64_t>(reinterpret_cast<uintptr_t>(bgraPb)));
      CVPixelBufferRelease(bgraPb);

      CVPixelBufferRef nv12Pb = nullptr;
      assert(CVPixelBufferCreate(
                 kCFAllocatorDefault, 80u, 60u, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, nullptr, &nv12Pb) ==
             kCVReturnSuccess);
      assert([camView emitFrameFromPixelBuffer:nv12Pb timestampNs:9876543210ULL rotationDeg:180u]);
      assert(nve::apple::decode_camera_frame_descriptor(capturedFrame, decodedPb) == nve::NativeViewCodecError::Ok);
      assert(decodedPb.width == 80u && decodedPb.height == 60u && decodedPb.stride_bytes >= 80u);
      assert(decodedPb.pixel_format == nve::apple::CameraPixelFormat::Nv12 && decodedPb.rotation_deg == 180u);
      CVPixelBufferRelease(nv12Pb);

      // Dispatch all 7 NVNC commands + invalid zoom rejection through FabricHost.
      nve::CommandBufferEncoder enc2(1u, 2u);
      enc2.dispatch_command(30u, "switch_facing", encode_cmd_u32(1u, 2u));
      enc2.dispatch_command(30u, "set_torch", encode_cmd_u32(2u, 1u));
      enc2.dispatch_command(30u, "set_zoom", encode_cmd_f32(3u, 3.25f));
      enc2.dispatch_command(30u, "set_zoom", encode_cmd_f32(3u, -2.0f));
      enc2.dispatch_command(30u, "focus_point", encode_cmd_f32_f32(4u, 0.25f, 0.75f));
      enc2.dispatch_command(30u, "capture_photo", encode_cmd_u32_u32(5u, 85u, 2u));
      enc2.dispatch_command(30u, "start_recording", encode_cmd_str(6u, "/tmp/nve-mac-test.mov"));
      enc2.dispatch_command(30u, "stop_recording", encode_cmd_empty(7u));
      enc2.commit(true);
      assert(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);

      assert(camView.nveFacing == nve::apple::CameraPreviewFacing::Front);
      assert(camView.nveTorchMode == nve::apple::TorchMode::On);
      assert(std::fabs(camView.nveZoom - 3.25f) < 1e-4f);
      assert(std::fabs(camView.nveLastFocusPoint.x - 0.25) < 1e-4 &&
             std::fabs(camView.nveLastFocusPoint.y - 0.75) < 1e-4);
      assert(camView.nvePhotoCaptureCount == 1u);
      assert([camView.nveLastRecordingPath isEqualToString:@"/tmp/nve-mac-test.mov"]);
      assert(camView.nveIsRecording == NO);
      assert(registry.unmount(30u));

      // 14c. Pluggable VisionCamera v5 bridge override + fallback to AVFoundation.
      MockVisionCameraBridge *mockBridge = [[MockVisionCameraBridge alloc] init];
      nve::apple::set_vision_camera_apple_bridge(mockBridge);
      assert(nve::apple::get_vision_camera_apple_bridge() == mockBridge);

      const auto vcProps = encode_camera_props(
          std::nullopt, 2u, 1u, 1.75f, 30u, false, 1u, false, true, true);
      nve::CommandBufferEncoder enc3(1u, 3u);
      enc3.create_node(40u, NVE_COMP_CUSTOM, nve::apple::kNativeKindCameraPreview);
      enc3.set_raw_prop_string(40u, nve::kNativeViewKindPropKey, nve::apple::kNativeKindCameraPreview);
      enc3.set_raw_prop_bytes(40u, nve::kNativeViewPropsPropKey, vcProps);
      enc3.set_children(NVE_ROOT_NODE_ID, {40u});
      enc3.commit(true);
      assert(nve_fabric_host_submit_frame(host, enc3.finish(), nullptr) == 0);

      nve::CommandBufferEncoder enc4(1u, 4u);
      enc4.dispatch_command(40u, "switch_facing", encode_cmd_u32(1u, 3u));
      enc4.dispatch_command(40u, "start_recording", encode_cmd_str(6u, "/tmp/vc.mp4"));
      enc4.commit(true);
      assert(nve_fabric_host_submit_frame(host, enc4.finish(), nullptr) == 0);

      NSView *vcRaw = nve::apple::native_view_for_node(registry, 40u);
      assert(vcRaw != nil && ![vcRaw isKindOfClass:[NveAppleCameraView class]]);
      assert(mockBridge.createCount == 1u && mockBridge.updateCount == 1u && mockBridge.commandCount == 2u);
      assert(mockBridge.lastFacing == nve::apple::CameraPreviewFacing::External);
      assert(mockBridge.lastTorch == nve::apple::TorchMode::On);
      assert(std::fabs(mockBridge.lastZoom - 1.75f) < 1e-4f);
      assert([mockBridge.lastRecordingPath isEqualToString:@"/tmp/vc.mp4"]);
      assert(registry.unmount(40u));
      assert(mockBridge.unmountCount == 1u);

      // Clear bridge -> falls back cleanly to NveAppleCameraView (AVFoundation).
      nve::apple::set_vision_camera_apple_bridge(nil);
      assert(registry.mount_or_update(41u, nve::apple::kNativeKindCameraPreview, vcProps) != nullptr);
      NSView *fallbackView = nve::apple::native_view_for_node(registry, 41u);
      assert(fallbackView != nil && [fallbackView isKindOfClass:[NveAppleCameraView class]]);
      assert(registry.unmount(41u));
      assert(registry.mounted_count() == 0u);
      nve_fabric_host_destroy(host);
    }

    std::cout << "nve_macos_shell_test passed ("
              << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+AppKit" : "Standalone+AppKit")
              << ")\n";
  }
  return 0;
}
