#include "NveQtQuickItem.hpp"

#include "mock_jsi_recorder.hpp"

#include <QAccessible>
#include <QApplication>
#include <QBuffer>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QImage>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPointingDevice>
#include <QQmlComponent>
#include <QQuickView>
#include <QQuickWindow>
#include <QSGClipNode>
#include <QSGGeometryNode>
#include <QSGNode>
#include <QSGOpacityNode>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QSGTransformNode>
#include <QUrl>
#include <QWheelEvent>

#include <atomic>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <iostream>
#include <memory>
#include <string>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace {

bool approx_eq(double a, double b, double eps = 2.0) {
  return std::fabs(a - b) <= eps;
}

class BinaryCmdHelper {
 public:
  std::vector<uint8_t> buf;
  uint32_t ops{0};

  void begin(uint32_t surface_id, uint64_t frame_id) {
    buf.clear();
    ops = 0;
    push<uint32_t>(NVE_ABI_MAGIC);
    push<uint16_t>(NVE_ABI_VERSION);
    push<uint16_t>(0);
    push<uint32_t>(surface_id);
    push<uint64_t>(frame_id);
    push<uint32_t>(0);
  }

  void create_node(uint32_t id, uint16_t kind) {
    push<uint8_t>(NVE_OP_CREATE_NODE);
    push<uint32_t>(id);
    push<uint16_t>(kind);
    push<uint16_t>(0);
    ++ops;
  }

  void remove_node(uint32_t id) {
    push<uint8_t>(NVE_OP_REMOVE_NODE);
    push<uint32_t>(id);
    ++ops;
  }

  void set_children(uint32_t parent, std::initializer_list<uint32_t> kids) {
    push<uint8_t>(NVE_OP_SET_CHILDREN);
    push<uint32_t>(parent);
    push<uint32_t>(static_cast<uint32_t>(kids.size()));
    for (uint32_t k : kids) {
      push<uint32_t>(k);
    }
    ++ops;
  }

  void set_text(uint32_t id, const std::string& text) {
    push<uint8_t>(NVE_OP_SET_TEXT);
    push<uint32_t>(id);
    push<uint32_t>(static_cast<uint32_t>(text.size()));
    buf.insert(buf.end(), text.begin(), text.end());
    ++ops;
  }

  void set_event_mask(uint32_t id, uint64_t mask) {
    push<uint8_t>(NVE_OP_SET_EVENT_MASK);
    push<uint32_t>(id);
    push<uint64_t>(mask);
    ++ops;
  }

  void set_layout(
      uint32_t id,
      uint8_t dir,
      uint8_t wtag,
      float w,
      uint8_t htag,
      float h,
      float grow,
      float pad,
      float gap,
      float shrink = 1.0f) {
    push<uint8_t>(NVE_OP_SET_LAYOUT);
    push<uint32_t>(id);
    push_dim(wtag, w);
    push_dim(htag, h);
    push_dim(0, 0.0f);
    push_dim(0, 0.0f);
    push_dim(0, 0.0f);
    push_dim(0, 0.0f);
    push<uint8_t>(dir);
    push<uint8_t>(0); // FlexStart
    push<uint8_t>(1); // Align FlexStart
    push<float>(grow);
    push<float>(shrink);
    push<float>(gap);
    push_insets(pad, pad, pad, pad);
    push_insets(0.0f, 0.0f, 0.0f, 0.0f);
    ++ops;
  }

  void set_style(
      uint32_t id,
      uint32_t bg,
      uint32_t fg,
      uint32_t border_rgba,
      float border_w,
      float radius,
      float opacity,
      float font_size,
      int32_t z_index = 0) {
    push<uint8_t>(NVE_OP_SET_STYLE);
    push<uint32_t>(id);
    push<uint32_t>(bg);
    push<uint32_t>(fg);
    push<uint32_t>(border_rgba);
    push<float>(border_w);
    push<float>(radius);
    push<float>(opacity);
    push<float>(font_size);
    push<int32_t>(z_index);
    ++ops;
  }

  void set_raw_prop_string(uint32_t id, std::string_view key, std::string_view val) {
    push<uint8_t>(NVE_OP_SET_RAW_PROP);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(key.size()));
    buf.insert(buf.end(), key.begin(), key.end());
    push<uint8_t>(4u);
    push<uint32_t>(static_cast<uint32_t>(val.size()));
    buf.insert(buf.end(), val.begin(), val.end());
    ++ops;
  }

  void set_raw_prop_bool(uint32_t id, std::string_view key, bool val) {
    push<uint8_t>(NVE_OP_SET_RAW_PROP);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(key.size()));
    buf.insert(buf.end(), key.begin(), key.end());
    push<uint8_t>(1u);
    push<uint8_t>(val ? 1u : 0u);
    ++ops;
  }

  NveBufferSlice finish() {
    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(1);
    ++ops;
    std::memcpy(buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{buf.data(), buf.size()};
  }

 private:
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf.insert(buf.end(), p, p + sizeof(T));
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
};

struct RecordedDirectEvent {
  uint32_t node_id{0};
  uint8_t kind{0};
  uint8_t payload_tag{0};
  float x{0.0f};
  float y{0.0f};
  float width{0.0f};
  float height{0.0f};
  std::string text;
};

/// Rich in-process mock component exercising zero-copy `dispatch_events_direct`,
/// `set_props_direct`, rounded-rect clipping, `ScrollView` clipping & scroll offsets,
/// `QSGOpacityNode` translucent subtrees, `TextInput` focus/blur/typing, z-index ordering,
/// and dynamic node addition/removal (for GPU text texture cache hit & eviction tests).
struct MockQuickComponent {
  static inline MockQuickComponent* last_instance = nullptr;

  uint32_t surface_id{1};
  int count{0};
  std::string title{"QuickGPU"};
  std::string input_text{"hi"};
  bool show_extra_text{true};
  std::vector<RecordedDirectEvent> direct_events;
  int direct_press_calls{0};
  int direct_scroll_calls{0};
  int direct_text_calls{0};
  int direct_focus_calls{0};
  int direct_blur_calls{0};
  BinaryCmdHelper cmd;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;
  std::string state_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQuickComponent();
    self->surface_id = sid;
    self->outbound.clear();
    last_instance = self;
    return self;
  }

  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockQuickComponent*>(h);
    if (last_instance == self) {
      last_instance = nullptr;
    }
    delete self;
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQuickComponent*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    // 2: Root column container (100% x 100%, padding 16, gap 10)
    self->cmd.create_node(2, NVE_COMP_VIEW);
    // 3: Static header text (should NEVER re-rasterize on counter increment!)
    self->cmd.create_node(3, NVE_COMP_TEXT);
    // 4: Dynamic counter text (ONLY text node that changes on press!)
    self->cmd.create_node(4, NVE_COMP_TEXT);
    // 5: Rounded button pressable (with child label 6) -> rounded clip + fill + border
    self->cmd.create_node(5, NVE_COMP_PRESSABLE);
    self->cmd.create_node(6, NVE_COMP_TEXT);
    // 7: Translucent badge (opacity 0.5) -> QSGOpacityNode + rounded fill
    self->cmd.create_node(7, NVE_COMP_VIEW);
    self->cmd.create_node(8, NVE_COMP_TEXT);
    // 9: ScrollView (height 60, tall child 10 of height 220 with nested pressable 11)
    self->cmd.create_node(9, NVE_COMP_SCROLL_VIEW);
    self->cmd.create_node(10, NVE_COMP_VIEW);
    self->cmd.create_node(11, NVE_COMP_PRESSABLE);
    self->cmd.create_node(12, NVE_COMP_TEXT);
    // 13: TextInput
    self->cmd.create_node(13, NVE_COMP_TEXT_INPUT);
    // 14: Row container holding flex-grow resizable box 15 (exercises geometryChange relayout)
    self->cmd.create_node(14, NVE_COMP_VIEW);
    self->cmd.create_node(15, NVE_COMP_VIEW);

    if (self->show_extra_text) {
      self->cmd.create_node(16, NVE_COMP_TEXT);
      self->cmd.set_children(2, {3, 4, 5, 7, 9, 13, 14, 16});
    } else {
      self->cmd.set_children(2, {3, 4, 5, 7, 9, 13, 14});
      self->cmd.remove_node(16);
    }

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
    self->cmd.set_children(5, {6});
    self->cmd.set_children(7, {8});
    self->cmd.set_children(9, {10});
    self->cmd.set_children(10, {11});
    self->cmd.set_children(11, {12});
    self->cmd.set_children(14, {15});

    // Container 2: column 100% x 100%, pad 16, gap 8
    self->cmd.set_layout(2, 0, 2, 100.0f, 2, 100.0f, 0.0f, 16.0f, 8.0f);
    self->cmd.set_style(2, 0xFF1E170Fu, 0xFFFFFFFFu, 0xFF334155u, 1.0f, 0.0f, 1.0f, 16.0f);

    // Header 3: 240 x 28
    self->cmd.set_layout(3, 0, 1, 240.0f, 1, 28.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(3, self->title);
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 18.0f);

    // Counter 4: 200 x 26
    self->cmd.set_layout(4, 0, 1, 200.0f, 1, 26.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(4, "Clicks: " + std::to_string(self->count));
    self->cmd.set_style(4, 0x00000000u, 0xFF38BDF8u, 0, 0.0f, 0.0f, 1.0f, 16.0f);

    // Button 5: 180 x 40, border_radius 12, border 2px -> rounded fill + rounded border + rounded clip
    self->cmd.set_layout(5, 0, 1, 180.0f, 1, 40.0f, 0.0f, 8.0f, 0.0f);
    self->cmd.set_style(5, 0xFFEB6325u, 0xFFFFFFFFu, 0xFFFFFFFFu, 2.0f, 12.0f, 1.0f, 16.0f);
    self->cmd.set_event_mask(5, nve::kEventMaskPress);
    self->cmd.set_layout(6, 0, 1, 140.0f, 1, 22.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(6, "Increment Button");
    self->cmd.set_style(6, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 15.0f);

    // Translucent badge 7: opacity 0.5 -> QSGOpacityNode
    self->cmd.set_layout(7, 0, 1, 140.0f, 1, 28.0f, 0.0f, 4.0f, 0.0f);
    self->cmd.set_style(7, 0xFF8B5CF6u, 0xFFFFFFFFu, 0, 0.0f, 8.0f, 0.5f, 14.0f);
    self->cmd.set_layout(8, 0, 1, 120.0f, 1, 20.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(8, "Translucent");
    self->cmd.set_style(8, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 13.0f);

    // ScrollView 9: 260 x 60 (rectangular clip), child 10: 260 x 220 (shrink=0 so content overflows)
    self->cmd.set_layout(9, 0, 1, 260.0f, 1, 60.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(9, 0xFF1F2937u, 0xFFFFFFFFu, 0xFF475569u, 1.0f, 0.0f, 1.0f, 14.0f);
    self->cmd.set_layout(10, 0, 1, 260.0f, 1, 220.0f, 0.0f, 8.0f, 0.0f, 0.0f);
    self->cmd.set_style(10, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 14.0f);
    self->cmd.set_layout(11, 0, 1, 220.0f, 1, 36.0f, 0.0f, 4.0f, 0.0f);
    self->cmd.set_style(11, 0xFF10B981u, 0xFFFFFFFFu, 0, 0.0f, 6.0f, 1.0f, 14.0f);
    self->cmd.set_event_mask(11, nve::kEventMaskPress);
    self->cmd.set_layout(12, 0, 1, 180.0f, 1, 22.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(12, "Scroll Child Item");
    self->cmd.set_style(12, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 14.0f);

    // TextInput 13: 240 x 32
    self->cmd.set_layout(13, 0, 1, 240.0f, 1, 32.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(13, self->input_text);
    self->cmd.set_style(13, 0xFF0F172Au, 0xFFFFFFFFu, 0xFF64748Bu, 1.0f, 6.0f, 1.0f, 14.0f);

    // Resizable row 14 (100% x 24) + flex_grow=1 box 15
    self->cmd.set_layout(14, 1, 2, 100.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_layout(15, 0, 0, 0.0f, 1, 24.0f, 1.0f, 0.0f, 0.0f);
    self->cmd.set_style(15, 0xFF0EA5E9u, 0xFFFFFFFFu, 0, 0.0f, 4.0f, 1.0f, 14.0f);

    if (self->show_extra_text) {
      self->cmd.set_layout(16, 0, 1, 180.0f, 0, 0.0f, 0.0f, 0.0f, 0.0f);
      self->cmd.set_text(16, "Evictable Cache Text");
      self->cmd.set_style(16, 0x00000000u, 0xFFFDE047u, 0, 0.0f, 0.0f, 1.0f, 14.0f);
    }

    return self->cmd.finish();
  }

  static uint32_t dispatch_events(NveAppHandle /*h*/, NveBufferSlice /*events*/) {
    // Should never be called when dispatch_events_direct is present!
    return 0;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickComponent*>(h);
    if (batch.ptr == nullptr) {
      return 0;
    }
    uint32_t dirty = 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      RecordedDirectEvent rec{};
      rec.node_id = ev.node_id;
      rec.kind = ev.kind;
      rec.payload_tag = ev.payload_tag;
      rec.x = ev.x;
      rec.y = ev.y;
      rec.width = ev.width;
      rec.height = ev.height;
      const auto sv = nve::str_slice_view(ev.text);
      rec.text.assign(sv.data(), sv.size());
      self->direct_events.push_back(rec);

      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press)) {
        ++self->direct_press_calls;
        if (ev.node_id == 5 || ev.node_id == 11) {
          ++self->count;
          const std::string payload = "{\"count\":" + std::to_string(self->count) + "}";
          self->outbound.push_custom(
              self->surface_id,
              ev.node_id,
              ev.timestamp_us,
              "onQuickPress",
              reinterpret_cast<const uint8_t*>(payload.data()),
              payload.size());
          dirty = 1;
        }
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Scroll)) {
        ++self->direct_scroll_calls;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::TextChange)) {
        ++self->direct_text_calls;
        if (ev.node_id == 13) {
          self->input_text = rec.text;
          dirty = 1;
        }
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Focus)) {
        ++self->direct_focus_calls;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Blur)) {
        ++self->direct_blur_calls;
      }
    }
    return dirty;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockQuickComponent*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("\"extra\":false") != std::string::npos) {
      self->show_extra_text = false;
    } else if (json.find("\"extra\":true") != std::string::npos) {
      self->show_extra_text = true;
    }
    if (json.find("PatchedTitle") != std::string::npos) {
      self->title = "PatchedTitle";
    }
    return 1;
  }

  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockQuickComponent*>(h);
    for (uint32_t i = 0; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "extra" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->show_extra_text = entries[i].value.data.bool_val != 0;
      } else if (k == "title" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        self->title.assign(v.data(), v.size());
      }
    }
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<MockQuickComponent*>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch_json) {
    auto* self = static_cast<MockQuickComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch_json.ptr), patch_json.len);
    return 1;
  }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    auto* self = static_cast<MockQuickComponent*>(h);
    self->state_buf = std::to_string(self->count);
    return NveBufferSlice{
        reinterpret_cast<const uint8_t*>(self->state_buf.data()),
        self->state_buf.size(),
    };
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    auto* self = static_cast<MockQuickComponent*>(h);
    if (state.ptr && state.len > 0) {
      const std::string raw(reinterpret_cast<const char*>(state.ptr), state.len);
      self->count = std::atoi(raw.c_str());
    }
  }

  static const void* get_interface(uint32_t interface_id) {
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

struct SGTreeAudit {
  int total{0};
  int transform_nodes{0};
  int opacity_nodes{0};
  int clip_nodes{0};
  int rect_clip_nodes{0};
  int rounded_clip_nodes{0};
  int geometry_nodes{0};
  int simple_rect_nodes{0};
  int texture_nodes{0};
  bool saw_half_opacity{false};
};

void audit_sg_tree(const QSGNode* node, SGTreeAudit& out) {
  if (node == nullptr) {
    return;
  }
  ++out.total;
  switch (node->type()) {
    case QSGNode::TransformNodeType:
      ++out.transform_nodes;
      break;
    case QSGNode::OpacityNodeType: {
      ++out.opacity_nodes;
      const auto* op = static_cast<const QSGOpacityNode*>(node);
      if (std::fabs(op->opacity() - 0.5) < 0.05) {
        out.saw_half_opacity = true;
      }
      break;
    }
    case QSGNode::ClipNodeType: {
      ++out.clip_nodes;
      const auto* clip = static_cast<const QSGClipNode*>(node);
      if (clip->isRectangular()) {
        ++out.rect_clip_nodes;
      } else {
        ++out.rounded_clip_nodes;
      }
      break;
    }
    case QSGNode::GeometryNodeType: {
      ++out.geometry_nodes;
      if (dynamic_cast<const QSGSimpleRectNode*>(node) != nullptr) {
        ++out.simple_rect_nodes;
      }
      if (const auto* tex = dynamic_cast<const QSGSimpleTextureNode*>(node)) {
        ++out.texture_nodes;
        NVE_CHECK(tex->texture() != nullptr);
        NVE_CHECK(!tex->ownsTexture());
        NVE_CHECK(tex->rect().width() > 0.0 && tex->rect().height() > 0.0);
      }
      break;
    }
    default:
      break;
  }
  for (const QSGNode* child = node->firstChild(); child != nullptr; child = child->nextSibling()) {
    audit_sg_tree(child, out);
  }
}

void send_quick_click(QQuickItem* item, const QPointF& pos) {
  QMouseEvent press(
      QEvent::MouseButtonPress, pos, pos, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
  QMouseEvent release(
      QEvent::MouseButtonRelease, pos, pos, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
  QCoreApplication::sendEvent(item, &press);
  QCoreApplication::sendEvent(item, &release);
}

void send_quick_wheel(QQuickItem* item, const QPointF& pos, const QPoint& pixel_delta) {
  QWheelEvent wheel(
      pos,
      pos,
      pixel_delta,
      QPoint(0, 0),
      Qt::NoButton,
      Qt::NoModifier,
      Qt::ScrollUpdate,
      false);
  QCoreApplication::sendEvent(item, &wheel);
}

void send_quick_key(QQuickItem* item, int key, const QString& text) {
  QKeyEvent ev(QEvent::KeyPress, key, Qt::NoModifier, text);
  QCoreApplication::sendEvent(item, &ev);
}

void test_qml_registration_and_quick_view() {
  const int type_id = nve::NveQtQuickItem::registerQmlType();
  NVE_CHECK(type_id >= 0);

  QQuickView view;
  view.setResizeMode(QQuickView::SizeRootObjectToView);
  view.resize(480, 320);

  const QUrl url;
  QQmlComponent comp(view.engine());
  comp.setData(
      QByteArrayLiteral("import Nve 1.0\nNveSurface { objectName: \"qmlNveSurface\"; surfaceId: 11 }\n"),
      url);
  if (comp.isError()) {
    std::cerr << "QQmlComponent errors: " << comp.errorString().toStdString() << "\n";
  }
  QObject* obj = comp.create(view.rootContext());
  NVE_CHECK(obj != nullptr);

  auto* item = qobject_cast<nve::NveQtQuickItem*>(obj);
  NVE_CHECK(item != nullptr);
  NVE_CHECK(item->objectName() == QStringLiteral("qmlNveSurface"));
  NVE_CHECK(item->surfaceId() == 11u);
  NVE_CHECK(!item->isAppLoaded());

  view.setContent(url, &comp, obj);
  NVE_CHECK(view.rootObject() == item);
  NVE_CHECK(item->attachInterface(&MockQuickComponent::get_interface, 11u));
  NVE_CHECK(item->isAppLoaded());
  NVE_CHECK(item->nodeCount() >= 15u);
  NVE_CHECK(item->sceneGeneration() > 0u);
}

void test_dylib_errors_and_hot_reload() {
  nve::NveQtQuickItem item;
  item.setSize(QSizeF(420, 260));

  int err_code = 0;
  QString err_msg;

  // Code 1: empty & nonexistent
  NVE_CHECK(!item.loadDylib(QString(), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!item.loadDylib(QStringLiteral("/nonexistent/missing.dylib"), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  // Code 2: missing nve_get_interface
  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!item.loadDylib(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && !err_msg.isEmpty());

  // Code 3: missing NVE_IFACE_SURFACE
  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!item.loadDylib(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && !err_msg.isEmpty());
  NVE_CHECK(!item.isAppLoaded());

  // Valid stateful fixture dylib
  err_code = -1;
  err_msg = QStringLiteral("stale");
  NVE_CHECK(item.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 5, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  NVE_CHECK(item.isAppLoaded());
  NVE_CHECK(item.surfaceId() == 5u);
  NVE_CHECK(item.nodeCount() == 3u);
  NVE_CHECK(item.sceneView().schema_hash == NVE_SCENE_SCHEMA_HASH);
  NVE_CHECK(item.sceneNodeText(3) == "Fixture:0");

  // Dispatch direct press twice (fixture falls back to EventRing seamlessly) -> Fixture:2
  item.dispatchPress(2, 6.0f, 6.0f);
  item.dispatchPress(2, 6.0f, 6.0f);
  NVE_CHECK(item.sceneNodeText(3) == "Fixture:2");

  // Failed hotReloads must leave live Fixture:2 intact
  NVE_CHECK(!item.hotReload(QStringLiteral("/nonexistent/fail.dylib"), &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && item.sceneNodeText(3) == "Fixture:2");
  NVE_CHECK(!item.hotReload(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && item.sceneNodeText(3) == "Fixture:2");
  NVE_CHECK(!item.hotReload(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && item.sceneNodeText(3) == "Fixture:2");

  // Warm hotReload preserves Fixture:2
  NVE_CHECK(item.hotReload(QStringLiteral(NVE_TEST_VALID_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  NVE_CHECK(item.sceneNodeText(3) == "Fixture:2");

  // Fresh load resets to Fixture:0
  NVE_CHECK(item.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 5, &err_code, &err_msg));
  NVE_CHECK(item.sceneNodeText(3) == "Fixture:0");
}

void test_scene_hit_test_direct_events_and_scene_graph_caching() {
  QQuickWindow window;
  window.resize(520, 420);

  auto* item = new nve::NveQtQuickItem(window.contentItem());
  item->setSize(QSizeF(520, 420));

  // Verify QFontMetricsF measurer spy invocation
  auto base_measurer = nve::NveQtQuickItem::makeQtTextMeasurer();
  std::atomic<int> measure_calls{0};
  item->setTextMeasurer(
      [&](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
        measure_calls.fetch_add(1, std::memory_order_relaxed);
        return base_measurer(text, font_size, max_width);
      });

  std::vector<nve::DecodedEvent> emitted_events;
  item->setOnNveEvent([&](const nve::DecodedEvent& ev) { emitted_events.push_back(ev); });

  NVE_CHECK(item->attachInterface(&MockQuickComponent::get_interface, 3u));
  NVE_CHECK(MockQuickComponent::last_instance != nullptr);
  auto* mock = MockQuickComponent::last_instance;
  NVE_CHECK(item->sceneView().schema_hash == NVE_SCENE_SCHEMA_HASH);
  NVE_CHECK(item->sceneView().node_count == 16u);
  if (nve::fabric_host_has_yoga_layout()) {
    NVE_CHECK(measure_calls.load() > 0);
  }

  // Verify layout & hit-testing on pressable button 5
  const QRectF btn5_bounds = item->sceneNodeGlobalBounds(5);
  NVE_CHECK(btn5_bounds.isValid() && btn5_bounds.width() > 100.0 && btn5_bounds.height() > 20.0);
  const QPointF btn5_center = btn5_bounds.center();
  NVE_CHECK(item->hitTestNode(btn5_center, /*interactiveOnly=*/true) == 5u);
  NVE_CHECK(item->hitTestNode(QPointF(5000.0, 5000.0), /*interactiveOnly=*/false) == 0u);

  // Real QMouseEvent click on button 5 -> zero-copy dispatch_events_direct with exact local coords!
  send_quick_click(item, btn5_center);
  NVE_CHECK(mock->direct_press_calls == 1);
  NVE_CHECK(item->sceneNodeText(4) == "Clicks: 1");
  NVE_CHECK(!mock->direct_events.empty());
  const auto& first_ev = mock->direct_events.back();
  NVE_CHECK(first_ev.node_id == 5u);
  NVE_CHECK(first_ev.kind == static_cast<uint8_t>(nve::EventKind::Press));
  NVE_CHECK(first_ev.payload_tag == 1u);
  NVE_CHECK(approx_eq(first_ev.x, btn5_bounds.width() * 0.5, 1.5));
  NVE_CHECK(approx_eq(first_ev.y, btn5_bounds.height() * 0.5, 1.5));
  NVE_CHECK(emitted_events.size() == 1u && emitted_events[0].name == "onQuickPress");

  // Press + drag outside button 5 + release must cancel the press (0 new press events)
  {
    QMouseEvent press(
        QEvent::MouseButtonPress, btn5_center, btn5_center, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QMouseEvent move(
        QEvent::MouseMove, QPointF(480.0, 400.0), QPointF(480.0, 400.0), Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
    QMouseEvent release(
        QEvent::MouseButtonRelease, QPointF(480.0, 400.0), QPointF(480.0, 400.0), Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(item, &press);
    QCoreApplication::sendEvent(item, &move);
    QCoreApplication::sendEvent(item, &release);
  }
  NVE_CHECK(mock->direct_press_calls == 1);
  NVE_CHECK(item->sceneNodeText(4) == "Clicks: 1");

  // ScrollView 9 wheel scrolling + clipping + scroll offset clamping + direct Scroll event
  const QRectF scroll9_bounds = item->sceneNodeGlobalBounds(9);
  const QRectF scroll_item11_before = item->sceneNodeGlobalBounds(11);
  NVE_CHECK(scroll9_bounds.isValid() && approx_eq(scroll9_bounds.height(), 60.0));
  NVE_CHECK(item->hitTestScrollableNode(scroll9_bounds.center()) == 9u);

  // Scroll down by 40px (negative wheel pixelDelta.y scrolls content down / increases scroll offset)
  send_quick_wheel(item, scroll9_bounds.center(), QPoint(0, -40));
  NVE_CHECK(mock->direct_scroll_calls == 1);
  const QPointF off1 = item->scrollOffsetForNode(9);
  NVE_CHECK(approx_eq(off1.y(), 40.0, 0.5));
  const QRectF scroll_item11_after = item->sceneNodeGlobalBounds(11);
  NVE_CHECK(approx_eq(scroll_item11_after.y(), scroll_item11_before.y() - 40.0, 0.5));

  // Overscroll clamps at max_y = content_h (220) - view_h (60) = 160
  send_quick_wheel(item, scroll9_bounds.center(), QPoint(0, -500));
  NVE_CHECK(approx_eq(item->scrollOffsetForNode(9).y(), 160.0, 0.5));
  // Now pressable 11 (originally at local y=8..44) is scrolled completely above ScrollView 9's clip!
  NVE_CHECK(item->hitTestNode(scroll_item11_before.center(), /*interactiveOnly=*/true) != 11u);

  // Reset scroll offset back to 0 so item 11 is visible & clickable inside ScrollView 9
  item->setScrollOffsetForNode(9, QPointF(0.0, 0.0));
  NVE_CHECK(item->hitTestNode(scroll_item11_before.center(), /*interactiveOnly=*/true) == 11u);
  send_quick_click(item, scroll_item11_before.center());
  NVE_CHECK(mock->direct_press_calls == 2);
  NVE_CHECK(item->sceneNodeText(4) == "Clicks: 2");

  // TextInput 13: click to focus -> type chars + backspace -> direct TextChange -> Escape blur
  const QRectF input13_bounds = item->sceneNodeGlobalBounds(13);
  NVE_CHECK(input13_bounds.isValid());
  send_quick_click(item, input13_bounds.center());
  NVE_CHECK(item->focusedNodeId() == 13u);
  NVE_CHECK(mock->direct_focus_calls == 1);

  send_quick_key(item, Qt::Key_Exclam, QStringLiteral("!"));
  send_quick_key(item, Qt::Key_Q, QStringLiteral("Q"));
  NVE_CHECK(mock->direct_text_calls == 2);
  NVE_CHECK(item->sceneNodeText(13) == "hi!Q");

  send_quick_key(item, Qt::Key_Backspace, QString());
  NVE_CHECK(mock->direct_text_calls == 3);
  NVE_CHECK(item->sceneNodeText(13) == "hi!");

  send_quick_key(item, Qt::Key_Escape, QString());
  NVE_CHECK(item->focusedNodeId() == 0u);
  NVE_CHECK(mock->direct_blur_calls == 1);

  // Resize via geometryChange -> Yoga flex_grow box 15 expands cleanly
  if (nve::fabric_host_has_yoga_layout()) {
    const QRectF box15_before = item->sceneNodeGlobalBounds(15);
    item->setSize(QSizeF(620, 420));
    const QRectF box15_after = item->sceneNodeGlobalBounds(15);
    NVE_CHECK(approx_eq(box15_after.width() - box15_before.width(), 100.0, 2.5));
  }

  // Real QQuickWindow offscreen rendering + Scene Graph QSGNode tree & text texture cache audit!
  window.show();
  QCoreApplication::processEvents();
  const QImage grabbed = window.grabWindow();
  NVE_CHECK(!grabbed.isNull() && grabbed.width() > 0 && grabbed.height() > 0);
  // Verify rendered image is not blank/uniform (contains dark background + colorful UI nodes)
  const QRgb first_px = grabbed.pixel(2, 2);
  bool saw_distinct_pixel = false;
  for (int y = 0; y < grabbed.height() && !saw_distinct_pixel; y += 4) {
    for (int x = 0; x < grabbed.width(); x += 4) {
      if (grabbed.pixel(x, y) != first_px) {
        saw_distinct_pixel = true;
        break;
      }
    }
  }
  NVE_CHECK(saw_distinct_pixel);

  // Now inspect the exact QSGNode tree and text texture cache generation tracking deterministically
  std::unique_ptr<QSGNode> sg_root(item->syncSceneGraphNode(nullptr));
  NVE_CHECK(sg_root != nullptr);

  SGTreeAudit audit1{};
  audit_sg_tree(sg_root.get(), audit1);
  NVE_CHECK(audit1.transform_nodes >= 6);
  NVE_CHECK(audit1.opacity_nodes >= 1 && audit1.saw_half_opacity);
  NVE_CHECK(audit1.clip_nodes >= 3);
  NVE_CHECK(audit1.rect_clip_nodes >= 1);    // ScrollView 9 rectangular scissor clip
  NVE_CHECK(audit1.rounded_clip_nodes >= 2); // Button 5 & Badge 7 / TextInput 13 rounded clip
  NVE_CHECK(audit1.geometry_nodes >= 6);
  NVE_CHECK(audit1.simple_rect_nodes >= 1);  // Unrounded background fill
  NVE_CHECK(audit1.texture_nodes == 7);      // Nodes 3, 4, 6, 8, 12, 13, 16

  const nve::SceneGraphStats stats1 = item->sceneGraphStats();
  NVE_CHECK(stats1.textTextureNodes == 7u);
  NVE_CHECK(stats1.textRasterizations == 7u);
  NVE_CHECK(stats1.textCacheHits == 0u);
  NVE_CHECK(stats1.cachedTextTextures == 7u);

  // Sync 2 with NO scene changes -> O(1) early out, 0 new rasterizations!
  QSGNode* sg_same = item->syncSceneGraphNode(sg_root.get());
  NVE_CHECK(sg_same == sg_root.get());
  const nve::SceneGraphStats stats2 = item->sceneGraphStats();
  NVE_CHECK(stats2.textRasterizations == 7u);
  NVE_CHECK(stats2.textCacheHits == 0u);

  // Sync 3: Press button 5 so ONLY counter text node 4 changes ("Clicks: 2" -> "Clicks: 3").
  // All 6 other text nodes MUST hit the GPU texture cache without re-rasterizing, AND unchanged
  // sibling QSGNode* pointers MUST survive without recreation!
  const QSGNode* header3_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3);
  const QSGNode* button5_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 5);
  const QSGNode* badge7_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 7);
  const QSGNode* scroll9_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 9);
  NVE_CHECK(header3_sg_before != nullptr && button5_sg_before != nullptr);
  NVE_CHECK(badge7_sg_before != nullptr && scroll9_sg_before != nullptr);

  item->dispatchPress(5, 10.0f, 10.0f);
  NVE_CHECK(item->sceneNodeText(4) == "Clicks: 3");
  QSGNode* sg_after_press = item->syncSceneGraphNode(sg_root.get());
  NVE_CHECK(sg_after_press == sg_root.get());
  const nve::SceneGraphStats stats3 = item->sceneGraphStats();
  NVE_CHECK(stats3.textRasterizations == 8u); // ONLY node 4 re-rasterized!
  NVE_CHECK(stats3.textCacheHits == 6u);      // 6 unchanged text nodes hit cache!
  NVE_CHECK(stats3.cachedTextTextures == 7u);
  NVE_CHECK(stats3.dirtyNodesUpdated == 1u);
  NVE_CHECK(stats3.retainedNodesReused >= 14u);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3) == header3_sg_before);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 5) == button5_sg_before);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 7) == badge7_sg_before);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 9) == scroll9_sg_before);

  // Sync 4: Remove extra text node 16 via zero-copy applyPropsDirect -> texture cache evicts 16!
  const NveMapEntry entries[1] = {
      NveMapEntry{nve::make_str_slice("extra"), nve::make_bool_value(false)},
  };
  NVE_CHECK(item->applyPropsDirect(entries, 1u));
  NVE_CHECK(!item->sceneNode(16).has_value());
  QSGNode* sg_after_evict = item->syncSceneGraphNode(sg_root.get());
  NVE_CHECK(sg_after_evict == sg_root.get());
  const nve::SceneGraphStats stats4 = item->sceneGraphStats();
  NVE_CHECK(stats4.textTextureNodes == 6u);
  NVE_CHECK(stats4.cachedTextTextures == 6u);
  NVE_CHECK(stats4.textRasterizations == 8u); // Zero new rasterizations on removal!
  NVE_CHECK(stats4.textCacheHits == 12u);     // All 6 remaining nodes hit cache!

  // Hot-patch updates title text node 3 (only 1 rasterization, 5 cache hits)
  NVE_CHECK(item->applyHotPatchJson("PatchedQuickTitle"));
  NVE_CHECK(item->sceneNodeText(3) == "PatchedQuickTitle");
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats stats5 = item->sceneGraphStats();
  NVE_CHECK(stats5.textRasterizations == 9u);
  NVE_CHECK(stats5.textCacheHits == 17u);

  // Destroy sg_root explicitly while window is still alive
  sg_root.reset();
}

void test_mmap_snapshot_roundtrip() {
  const QString snap_path = QDir::temp().filePath(
      QStringLiteral("nve_qt_quick_fixture_%1.nvesnap").arg(QCoreApplication::applicationPid()));
  QFile::remove(snap_path);

  {
    nve::NveQtQuickItem writer;
    writer.setSize(QSizeF(400, 240));
    NVE_CHECK(writer.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 1));
    writer.dispatchPress(2, 5.0f, 5.0f);
    writer.dispatchPress(2, 5.0f, 5.0f);
    writer.dispatchPress(2, 5.0f, 5.0f);
    NVE_CHECK(writer.sceneNodeText(3) == "Fixture:3");
    NVE_CHECK(writer.saveSnapshot(snap_path, 4));
  }

  QQuickWindow win;
  win.resize(400, 240);
  auto* cold = new nve::NveQtQuickItem(win.contentItem());
  cold->setSize(QSizeF(400, 240));

  uint64_t restore_us = 0;
  // Schema mismatch must fail cleanly
  NVE_CHECK(!cold->replaySnapshot(snap_path, 99, 1, &restore_us));
  // Matching schema replays Frame-0 BEFORE loadDylib!
  NVE_CHECK(cold->replaySnapshot(snap_path, 4, 1, &restore_us));
  std::cout << "[qt_quick_test] Cold mmap Frame-0 restore (fixture dylib): "
            << restore_us << " us\n";

  NVE_CHECK(!cold->isAppLoaded());
  NVE_CHECK(cold->nodeCount() == 3u);
  NVE_CHECK(cold->sceneNodeText(3) == "Fixture:3");

  // Scene Graph renders directly from the Frame-0 mmap replay before dylib loads!
  win.show();
  QCoreApplication::processEvents();
  std::unique_ptr<QSGNode> f0_sg(cold->syncSceneGraphNode(nullptr));
  NVE_CHECK(f0_sg != nullptr);
  NVE_CHECK(cold->sceneGraphStats().textTextureNodes >= 1u);

  // Loading dylib seamlessly restores Fixture:3 via restore_state!
  NVE_CHECK(cold->loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 1));
  NVE_CHECK(cold->isAppLoaded());
  NVE_CHECK(cold->sceneNodeText(3) == "Fixture:3");

  f0_sg.reset();
  QFile::remove(snap_path);
}

void test_z_index_hit_test_and_paint_order() {
  struct ZIndexComponent {
    BinaryCmdHelper cmd;
    static NveAppHandle create(uint32_t) { return new ZIndexComponent(); }
    static void destroy(NveAppHandle h) { delete static_cast<ZIndexComponent*>(h); }
    static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
      auto* self = static_cast<ZIndexComponent*>(h);
      self->cmd.begin(1, frame_id);
      self->cmd.create_node(2, NVE_COMP_VIEW);
      self->cmd.create_node(3, NVE_COMP_PRESSABLE);
      self->cmd.create_node(4, NVE_COMP_PRESSABLE);
      self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
      self->cmd.set_children(2, {3, 4});
      // Row container 200x100; node 3 (z=10) at x=0..100, node 4 (z=1) at x=0..100 (via margin? no,
      // give node 3 width=0 or negative margin—simplest: nest node 3 & 4 inside zero-gap row where
      // both span 100x80 with node 3 having z=10 and node 4 having z=1, verify z-order in SG tree,
      // and give node 3 & 4 overlapping bounds via parent layout!).
      self->cmd.set_layout(2, 1, 1, 200.0f, 1, 100.0f, 0.0f, 0.0f, 0.0f);
      self->cmd.set_layout(3, 0, 1, 100.0f, 1, 80.0f, 0.0f, 0.0f, 0.0f);
      self->cmd.set_style(3, 0xFFFF0000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 14.0f, /*z=*/10);
      self->cmd.set_layout(4, 0, 1, 100.0f, 1, 80.0f, 0.0f, 0.0f, 0.0f);
      self->cmd.set_style(4, 0xFF00FF00u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 14.0f, /*z=*/1);
      return self->cmd.finish();
    }
    static uint32_t dispatch_events(NveAppHandle, NveBufferSlice) { return 0; }
    static const void* get_interface(uint32_t id) {
      static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, nullptr};
      return (id == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
    }
  };

  nve::NveQtQuickItem item;
  item.setSize(QSizeF(200, 100));
  NVE_CHECK(item.attachInterface(&ZIndexComponent::get_interface, 1));

  // Verify Scene Graph child order: node 4 (z=1, x=100) must be painted BEFORE node 3 (z=10, x=0)
  std::unique_ptr<QSGNode> sg(item.syncSceneGraphNode(nullptr));
  NVE_CHECK(sg != nullptr);
  // Traverse down root -> NVE_ROOT_NODE_ID wrapper -> container 2 wrapper -> [child_z1, child_z10]
  QSGNode* root_wrap = sg->firstChild();
  NVE_CHECK(root_wrap != nullptr);
  QSGNode* c2_wrap = root_wrap->firstChild();
  NVE_CHECK(c2_wrap != nullptr);
  NVE_CHECK(c2_wrap->childCount() == 2);
  // First painted child is node 4 (x=100 -> QSGTransformNode), second is node 3 (x=0 -> QSGNode)
  NVE_CHECK(c2_wrap->childAtIndex(0)->type() == QSGNode::TransformNodeType);
  NVE_CHECK(c2_wrap->childAtIndex(1)->type() == QSGNode::BasicNodeType);
  // Hit-testing each pressable returns the exact node
  NVE_CHECK(item.hitTestNode(QPointF(50.0, 40.0), /*interactiveOnly=*/true) == 3u);
  NVE_CHECK(item.hitTestNode(QPointF(150.0, 40.0), /*interactiveOnly=*/true) == 4u);
}

void test_live_counter_and_studio_dylibs() {
  const char* counter_env = std::getenv("NVE_TEST_COUNTER_DYLIB");
  if (counter_env != nullptr && counter_env[0] != '\0') {
    QQuickWindow win;
    win.resize(640, 420);
    auto* item = new nve::NveQtQuickItem(win.contentItem());
    item->setSize(QSizeF(640, 420));

    const QString counter_path = QString::fromUtf8(counter_env);
    int err_code = -1;
    QString err_msg;
    NVE_CHECK(item->loadDylib(counter_path, 1, &err_code, &err_msg));
    NVE_CHECK(err_code == 0 && err_msg.isEmpty());

    auto find_node_by_text = [&](std::string_view needle) -> uint32_t {
      const NveSceneView view = item->sceneView();
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        if (nve::scene_node_text(view, view.nodes[i]) == needle) {
          return view.nodes[i].node_id;
        }
      }
      return 0u;
    };

    const uint32_t title_id = find_node_by_text("NATIVE VIEW ENGINE");
    const uint32_t count_id = find_node_by_text("Count: 0");
    const uint32_t inc_text_id = find_node_by_text("Increment +");
    const uint32_t reset_text_id = find_node_by_text("Reset");
    NVE_CHECK(title_id != 0u && count_id != 0u && inc_text_id != 0u && reset_text_id != 0u);

    // Hit-test Increment + center to locate its pressable button!
    const QRectF inc_bounds = item->sceneNodeGlobalBounds(inc_text_id);
    const uint32_t inc_btn_id = item->hitTestNode(inc_bounds.center(), /*interactiveOnly=*/true);
    NVE_CHECK(inc_btn_id != 0u);

    win.show();
    QCoreApplication::processEvents();
    std::unique_ptr<QSGNode> sg(item->syncSceneGraphNode(nullptr));
    const uint64_t initial_rasters = item->sceneGraphStats().textRasterizations;
    NVE_CHECK(initial_rasters >= 4u);

    // Click Increment + twice via real QMouseEvent -> Count: 2, unchanged labels hit cache!
    send_quick_click(item, inc_bounds.center());
    (void)item->syncSceneGraphNode(sg.get());
    send_quick_click(item, inc_bounds.center());
    (void)item->syncSceneGraphNode(sg.get());
    NVE_CHECK(find_node_by_text("Count: 2") != 0u);
    NVE_CHECK(item->sceneGraphStats().textCacheHits >= 6u);

    // Warm hotReload preserves Count: 2
    NVE_CHECK(item->hotReload(counter_path, &err_code, &err_msg));
    NVE_CHECK(err_code == 0 && find_node_by_text("Count: 2") != 0u);

    // Snapshot save -> reset -> cold replaySnapshot -> loadDylib restores Count: 2!
    const QString counter_snap = QDir::temp().filePath(
        QStringLiteral("nve_qt_quick_counter_%1.nvesnap").arg(QCoreApplication::applicationPid()));
    QFile::remove(counter_snap);
    NVE_CHECK(item->saveSnapshot(counter_snap, 1));
    NVE_CHECK(item->applyPropsJson(R"({"initialCount":0})"));

    nve::NveQtQuickItem cold_counter;
    cold_counter.setSize(QSizeF(640, 420));
    uint64_t restore_us = 0;
    NVE_CHECK(cold_counter.replaySnapshot(counter_snap, 1, 1, &restore_us));
    std::cout << "[qt_quick_test] Cold mmap Frame-0 restore (counter dylib): "
              << restore_us << " us\n";

    auto cold_has_text = [&](std::string_view needle) {
      const NveSceneView view = cold_counter.sceneView();
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        if (nve::scene_node_text(view, view.nodes[i]) == needle) {
          return true;
        }
      }
      return false;
    };
    NVE_CHECK(!cold_counter.isAppLoaded() && cold_has_text("Count: 2"));
    NVE_CHECK(cold_counter.loadDylib(counter_path, 1));
    NVE_CHECK(cold_counter.isAppLoaded() && cold_has_text("Count: 2"));

    sg.reset();
    QFile::remove(counter_snap);
  }

  const char* studio_env = std::getenv("NVE_TEST_STUDIO_DYLIB");
  if (studio_env != nullptr && studio_env[0] != '\0') {
    QQuickWindow win;
    win.resize(960, 640);
    auto* studio = new nve::NveQtQuickItem(win.contentItem());
    studio->setSize(QSizeF(960, 640));

    const QString studio_path = QString::fromUtf8(studio_env);
    NVE_CHECK(studio->loadDylib(studio_path, 1));
    NVE_CHECK(studio->nodeCount() > 10u);
    NVE_CHECK(studio->applyPropsJson(R"({"accent":"violet","view":"metrics"})"));

    auto has_substr = [](const nve::NveQtQuickItem& it, std::string_view needle) {
      const NveSceneView view = it.sceneView();
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        if (nve::scene_node_text(view, view.nodes[i]).find(needle) != std::string_view::npos) {
          return true;
        }
      }
      return false;
    };
    NVE_CHECK(has_substr(*studio, "Live Metrics"));

    win.show();
    QCoreApplication::processEvents();
    std::unique_ptr<QSGNode> sg(studio->syncSceneGraphNode(nullptr));
    NVE_CHECK(sg != nullptr && studio->sceneGraphStats().geometryNodes > 5u);
    NVE_CHECK(studio->sceneGraphStats().textTextureNodes > 5u);

    const QString studio_snap = QDir::temp().filePath(
        QStringLiteral("nve_qt_quick_studio_%1.nvesnap").arg(QCoreApplication::applicationPid()));
    QFile::remove(studio_snap);
    NVE_CHECK(studio->saveSnapshot(studio_snap, 1));

    nve::NveQtQuickItem cold_studio;
    cold_studio.setSize(QSizeF(960, 640));
    uint64_t studio_us = 0;
    NVE_CHECK(cold_studio.replaySnapshot(studio_snap, 1, 1, &studio_us));
    std::cout << "[qt_quick_test] Cold mmap Frame-0 restore (studio-workspace): "
              << studio_us << " us\n";
    NVE_CHECK(!cold_studio.isAppLoaded() && has_substr(cold_studio, "Live Metrics"));
    NVE_CHECK(cold_studio.loadDylib(studio_path, 1));
    NVE_CHECK(cold_studio.isAppLoaded() && has_substr(cold_studio, "Live Metrics"));

    sg.reset();
    QFile::remove(studio_snap);
  }
}

struct MockQuickA11yImeComponent {
  static inline MockQuickA11yImeComponent* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string input_text;
  int direct_text_calls{0};
  int direct_submit_calls{0};
  std::string last_text;
  std::string last_submit;
  BinaryCmdHelper cmd;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQuickA11yImeComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockQuickA11yImeComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQuickA11yImeComponent*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_VIEW);
    self->cmd.create_node(3, NVE_COMP_PRESSABLE);
    self->cmd.create_node(4, NVE_COMP_TEXT);
    self->cmd.create_node(5, NVE_COMP_TEXT_INPUT);

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
    self->cmd.set_children(2, {3, 4, 5});

    self->cmd.set_layout(2, 0, 2, 100.0f, 2, 100.0f, 0.0f, 12.0f, 8.0f);
    self->cmd.set_layout(3, 0, 1, 140.0f, 1, 36.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_raw_prop_string(3, "accessibilityLabel", "Disabled Action");
    self->cmd.set_raw_prop_string(3, "accessibilityHint", "Currently unavailable");
    self->cmd.set_raw_prop_bool(3, "accessibilityDisabled", true);

    self->cmd.set_layout(4, 0, 1, 180.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(4, "Quick Heading");
    self->cmd.set_raw_prop_string(4, "accessibilityRole", "header");
    self->cmd.set_raw_prop_string(4, "testID", "quick-hdr");

    self->cmd.set_layout(5, 0, 1, 240.0f, 1, 34.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(5, self->input_text);
    self->cmd.set_style(5, 0xFF0F172Au, 0xFFFFFFFFu, 0xFF64748Bu, 1.0f, 6.0f, 1.0f, 14.0f);
    self->cmd.set_raw_prop_string(5, "accessibilityLabel", "Quick Search");
    self->cmd.set_raw_prop_string(5, "placeholder", "Search IME...");

    return self->cmd.finish();
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickA11yImeComponent*>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.node_id == 5 && ev.kind == static_cast<uint8_t>(nve::EventKind::TextChange)) {
        ++self->direct_text_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_text.assign(sv.data(), sv.size());
        self->input_text = self->last_text;
        return 1;
      }
      if (ev.node_id == 5 && ev.kind == static_cast<uint8_t>(nve::EventKind::Custom) &&
          nve::str_slice_view(ev.name) == "onSubmit") {
        ++self->direct_submit_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_submit.assign(sv.data(), sv.size());
      }
    }
    return 0;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, nullptr, &dispatch_events_direct};
    if (iface == NVE_IFACE_SURFACE) return &kSurface;
    return nullptr;
  }
};

void test_accessibility_and_ime_input_method() {
  QQuickWindow win;
  win.resize(480, 320);
  auto* item = new nve::NveQtQuickItem(win.contentItem());
  item->setSize(QSizeF(480, 320));
  NVE_CHECK((item->flags() & QQuickItem::ItemAcceptsInputMethod) != 0);

  NVE_CHECK(item->attachInterface(&MockQuickA11yImeComponent::get_interface, 23u));
  NVE_CHECK(MockQuickA11yImeComponent::last_instance != nullptr);
  auto* mock = MockQuickA11yImeComponent::last_instance;

  // Verify accessible metadata & role extraction
  NVE_CHECK(item->accessibleRoleForNodeId(3) == QAccessible::Button);
  NVE_CHECK(item->accessibleNameForNodeId(3) == QStringLiteral("Disabled Action"));
  NVE_CHECK(item->accessibleDescriptionForNodeId(3) == QStringLiteral("Currently unavailable"));
  NVE_CHECK(item->accessibleRoleForNodeId(4) == QAccessible::Heading);
  NVE_CHECK(item->accessibleNameForNodeId(4) == QStringLiteral("Quick Heading"));
  NVE_CHECK(item->accessibleRoleForNodeId(5) == QAccessible::EditableText);
  NVE_CHECK(item->accessibleNameForNodeId(5) == QStringLiteral("Quick Search"));
  const auto input_sem = item->accessibilityInfoForNodeId(5);
  NVE_CHECK(input_sem.has_value() && input_sem->placeholder == QStringLiteral("Search IME..."));

  // Disabled button must NOT be hit-testable as interactive
  const QRectF disabled_btn_bounds = item->sceneNodeGlobalBounds(3);
  NVE_CHECK(disabled_btn_bounds.isValid());
  NVE_CHECK(item->hitTestNode(disabled_btn_bounds.center(), /*interactiveOnly=*/true) == 0u);

  // Focus TextInput 5 via mouse click
  const QRectF input_bounds = item->sceneNodeGlobalBounds(5);
  NVE_CHECK(input_bounds.isValid());
  send_quick_click(item, input_bounds.center());
  NVE_CHECK(item->focusedNodeId() == 5u);
  NVE_CHECK(item->inputMethodQuery(Qt::ImEnabled).toBool());

  // 1. IME preedit displays live without dispatching premature TextChange or Submit
  {
    QList<QInputMethodEvent::Attribute> attrs;
    QInputMethodEvent preedit_ev(QStringLiteral("かんじ"), attrs);
    QCoreApplication::sendEvent(item, &preedit_ev);
    NVE_CHECK(item->imePreeditText(5) == QStringLiteral("かんじ"));
    NVE_CHECK(mock->direct_text_calls == 0);

    // Return key during preedit must NOT submit prematurely
    send_quick_key(item, Qt::Key_Return, QStringLiteral("\r"));
    NVE_CHECK(mock->direct_submit_calls == 0);
  }

  // 2. IME commit with CJK + accented characters dispatches clean UTF-8 TextChange
  {
    QInputMethodEvent commit_ev;
    commit_ev.setCommitString(QStringLiteral("漢字 naïve"));
    QCoreApplication::sendEvent(item, &commit_ev);
    NVE_CHECK(item->imePreeditText(5).isEmpty());
    NVE_CHECK(mock->direct_text_calls == 1);
    NVE_CHECK(mock->last_text == "漢字 naïve");
    NVE_CHECK(item->sceneNodeText(5) == "漢字 naïve");
  }

  // 3. IME replacement replaces "naïve" (5 UTF-16 units) with "café" without corrupting CJK prefix
  {
    QInputMethodEvent replace_ev;
    replace_ev.setCommitString(QStringLiteral("café"), -5, 5);
    QCoreApplication::sendEvent(item, &replace_ev);
    NVE_CHECK(mock->direct_text_calls == 2);
    NVE_CHECK(mock->last_text == "漢字 café");
    NVE_CHECK(item->sceneNodeText(5) == "漢字 café");
  }

  // 4. Return key dispatches direct onSubmit carrying exact UTF-8 text
  send_quick_key(item, Qt::Key_Return, QStringLiteral("\r"));
  NVE_CHECK(mock->direct_submit_calls == 1);
  NVE_CHECK(mock->last_submit == "漢字 café");
}

struct MockQuickImageComponent {
  static inline MockQuickImageComponent* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string data_uri;
  std::string file_uri;
  int ticks{0};
  BinaryCmdHelper cmd;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQuickImageComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockQuickImageComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }
  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQuickImageComponent*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_VIEW);
    self->cmd.create_node(3, NVE_COMP_IMAGE);
    self->cmd.create_node(4, NVE_COMP_IMAGE);
    self->cmd.create_node(5, NVE_COMP_TEXT);

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
    self->cmd.set_children(2, {3, 4, 5});

    self->cmd.set_layout(2, 0, 1, 240.0f, 1, 200.0f, 0.0f, 10.0f, 8.0f);
    self->cmd.set_style(2, 0xFF0F172Au, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    self->cmd.set_layout(3, 0, 1, 64.0f, 1, 64.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 6.0f, 1.0f, 14.0f);
    self->cmd.set_raw_prop_string(3, "src", self->data_uri);
    self->cmd.set_raw_prop_string(3, "resizeMode", "cover");

    self->cmd.set_layout(4, 0, 1, 48.0f, 1, 48.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(4, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);
    self->cmd.set_raw_prop_string(4, "src", self->file_uri);
    self->cmd.set_raw_prop_string(4, "resizeMode", "contain");

    self->cmd.set_layout(5, 0, 1, 140.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(5, "Tick:" + std::to_string(self->ticks));
    self->cmd.set_style(5, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    return self->cmd.finish();
  }
  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickImageComponent*>(h);
    if (batch.ptr != nullptr && batch.len > 0u) {
      ++self->ticks;
      return 1;
    }
    return 0;
  }
  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, nullptr, &dispatch_events_direct};
    return (iface == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
  }
};

void test_quick_image_rendering_and_retained_sg_survival() {
  QImage green_img(8, 8, QImage::Format_RGBA8888);
  green_img.fill(QColor(24, 210, 96, 255));
  QByteArray png_bytes;
  QBuffer png_buf(&png_bytes);
  png_buf.open(QIODevice::WriteOnly);
  NVE_CHECK(green_img.save(&png_buf, "PNG"));
  const QByteArray b64 = png_bytes.toBase64();
  const std::string data_uri =
      "data:image/png;base64," + std::string(b64.constData(), static_cast<size_t>(b64.size()));

  const QString tmp_png = QDir::temp().filePath(
      QStringLiteral("nve_qt_quick_img_%1.png").arg(QCoreApplication::applicationPid()));
  {
    QImage amber_img(8, 8, QImage::Format_RGBA8888);
    amber_img.fill(QColor(245, 170, 20, 255));
    NVE_CHECK(amber_img.save(tmp_png, "PNG"));
  }
  const QByteArray file_url_utf8 = QUrl::fromLocalFile(tmp_png).toString().toUtf8();
  const std::string file_uri(file_url_utf8.constData(), static_cast<size_t>(file_url_utf8.size()));

  QQuickWindow win;
  win.resize(240, 200);
  auto* item = new nve::NveQtQuickItem(win.contentItem());
  item->setSize(QSizeF(240, 200));

  NVE_CHECK(item->attachInterface(&MockQuickImageComponent::get_interface, 29u));
  NVE_CHECK(MockQuickImageComponent::last_instance != nullptr);
  MockQuickImageComponent::last_instance->data_uri = data_uri;
  MockQuickImageComponent::last_instance->file_uri = file_uri;
  NVE_CHECK(item->renderFrame() == 0);

  win.show();
  QCoreApplication::processEvents();
  const QImage grabbed = win.grabWindow();
  NVE_CHECK(!grabbed.isNull() && grabbed.width() > 0 && grabbed.height() > 0);
  // Confirm green pixels from image node 3 rendered into the QuickWindow frame!
  bool saw_green_pixel = false;
  for (int y = 0; y < grabbed.height() && !saw_green_pixel; y += 2) {
    for (int x = 0; x < grabbed.width(); x += 2) {
      const QColor c = grabbed.pixelColor(x, y);
      if (c.green() > 180 && c.red() < 60 && c.blue() < 130) {
        saw_green_pixel = true;
        break;
      }
    }
  }
  NVE_CHECK(saw_green_pixel);

  std::unique_ptr<QSGNode> sg_root(item->syncSceneGraphNode(nullptr));
  NVE_CHECK(sg_root != nullptr);
  const nve::SceneGraphStats s1 = item->sceneGraphStats();
  NVE_CHECK(s1.imageTextureNodes == 2u);
  NVE_CHECK(s1.cachedImageTextures == 2u);
  NVE_CHECK(s1.textTextureNodes == 1u);

  // Record retained QSGNode* pointers for both image nodes before mutating sibling text node 5!
  const QSGNode* img3_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3);
  const QSGNode* img4_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 4);
  NVE_CHECK(img3_before != nullptr && img4_before != nullptr);

  item->dispatchPress(5, 2.0f, 2.0f);
  NVE_CHECK(item->sceneNodeText(5) == "Tick:1");
  QSGNode* sg_after = item->syncSceneGraphNode(sg_root.get());
  NVE_CHECK(sg_after == sg_root.get());
  const nve::SceneGraphStats s2 = item->sceneGraphStats();
  NVE_CHECK(s2.dirtyNodesUpdated == 1u);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3) == img3_before);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 4) == img4_before);

  sg_root.reset();
  QFile::remove(tmp_png);
}

struct MockQuickCanvasComponent {
  static inline MockQuickCanvasComponent* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string draw_list_attr;
  bool emit_corrupt_raw{false};
  std::string corrupt_raw;
  int ticks{0};
  BinaryCmdHelper cmd;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQuickCanvasComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockQuickCanvasComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }
  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQuickCanvasComponent*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_VIEW);
    self->cmd.create_node(3, NVE_COMP_SKIA_CANVAS);
    self->cmd.create_node(4, NVE_COMP_TEXT);

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
    self->cmd.set_children(2, {3, 4});

    self->cmd.set_layout(2, 0, 1, 240.0f, 1, 140.0f, 0.0f, 10.0f, 8.0f);
    self->cmd.set_style(2, 0xFF0B1020u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    self->cmd.set_layout(3, 0, 1, 200.0f, 1, 80.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 6.0f, 1.0f, 14.0f);
    if (self->emit_corrupt_raw) {
      self->cmd.set_raw_prop_string(3, "drawList", "");
      self->cmd.set_text(3, self->corrupt_raw);
    } else {
      self->cmd.set_raw_prop_string(3, "drawList", self->draw_list_attr);
    }

    self->cmd.set_layout(4, 0, 1, 140.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(4, "Frame:" + std::to_string(self->ticks));
    self->cmd.set_style(4, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    return self->cmd.finish();
  }
  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickCanvasComponent*>(h);
    if (batch.ptr != nullptr && batch.len > 0u) {
      ++self->ticks;
      return 1;
    }
    return 0;
  }
  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, nullptr, &dispatch_events_direct};
    return (iface == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
  }
};

void test_quick_draw_list_canvas_gpu_cache_and_corrupt_survival() {
  const std::vector<float> samples{10.0f, 32.0f, 22.0f, 55.0f, 44.0f, 78.0f};
  const nve::DrawColorRgba bg_col = nve::DrawColorRgba::rgb(18u, 28u, 52u);
  const nve::DrawColorRgba cyan_col = nve::DrawColorRgba::rgb(56u, 189u, 248u);
  const nve::DrawColorRgba pink_col = nve::DrawColorRgba::rgb(244u, 63u, 94u);

  nve::DrawListEncoder cyan_enc;
  cyan_enc.sparkline(samples, 200.0f, 80.0f, cyan_col, bg_col);
  const std::vector<uint8_t> cyan_bytes = cyan_enc.finish();

  nve::DrawListEncoder pink_enc;
  pink_enc.sparkline(samples, 200.0f, 80.0f, pink_col, bg_col);
  const std::vector<uint8_t> pink_bytes = pink_enc.finish();

  QQuickWindow win;
  win.resize(240, 140);
  auto* item = new nve::NveQtQuickItem(win.contentItem());
  item->setSize(QSizeF(240, 140));

  NVE_CHECK(item->attachInterface(&MockQuickCanvasComponent::get_interface, 37u));
  NVE_CHECK(MockQuickCanvasComponent::last_instance != nullptr);
  auto* mock = MockQuickCanvasComponent::last_instance;
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(cyan_bytes);
  NVE_CHECK(item->renderFrame() == 0);

  // 1. Live QQuickWindow rendering: cyan sparkline -> pink sparkline -> corrupt buffer survival.
  win.show();
  QCoreApplication::processEvents();
  const QImage grabbed1 = win.grabWindow();
  NVE_CHECK(!grabbed1.isNull() && grabbed1.width() > 0 && grabbed1.height() > 0);
  int bg_px = 0;
  int cyan_px = 0;
  for (int y = 0; y < grabbed1.height(); ++y) {
    for (int x = 0; x < grabbed1.width(); ++x) {
      const QColor c = grabbed1.pixelColor(x, y);
      if (std::abs(c.red() - 18) <= 3 && std::abs(c.green() - 28) <= 3 &&
          std::abs(c.blue() - 52) <= 3) {
        ++bg_px;
      }
      if (c.blue() > 210 && c.green() > 150 && c.red() < 90) {
        ++cyan_px;
      }
    }
  }
  NVE_CHECK(bg_px > 4000);
  NVE_CHECK(cyan_px > 35);

  // Update drawList bytes to pink on the live QQuickWindow and confirm pink pixels render!
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(pink_bytes);
  NVE_CHECK(item->renderFrame() == 0);
  QCoreApplication::processEvents();
  const QImage grabbed2 = win.grabWindow();
  int pink_px = 0;
  for (int y = 0; y < grabbed2.height(); ++y) {
    for (int x = 0; x < grabbed2.width(); ++x) {
      const QColor c = grabbed2.pixelColor(x, y);
      if (c.red() > 210 && c.green() < 100 && c.blue() > 70 && c.blue() < 130) {
        ++pink_px;
      }
    }
  }
  NVE_CHECK(pink_px > 35);

  // Corrupt draw list bytes survive live QQuickWindow rendering cleanly without crashing!
  mock->emit_corrupt_raw = true;
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(cyan_bytes.data()), 18u);
  NVE_CHECK(item->renderFrame() == 0);
  QCoreApplication::processEvents();
  const QImage grabbed_corrupt = win.grabWindow();
  NVE_CHECK(!grabbed_corrupt.isNull());

  // 2. Deterministic retained Scene Graph cache & O(dirty) survival audit on standalone sg_root.
  mock->emit_corrupt_raw = false;
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(cyan_bytes);
  NVE_CHECK(item->renderFrame() == 0);

  std::unique_ptr<QSGNode> sg_root(item->syncSceneGraphNode(nullptr));
  NVE_CHECK(sg_root != nullptr);
  const nve::SceneGraphStats s1 = item->sceneGraphStats();
  NVE_CHECK(s1.canvasTextureNodes == 1u);
  NVE_CHECK(s1.cachedCanvasTextures == 1u);
  NVE_CHECK(s1.canvasRasterizations == 1u);

  // Mutate ONLY sibling text node 4 -> canvas QSGNode* survives and 0 new canvas rasterizations!
  const QSGNode* canvas_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3);
  NVE_CHECK(canvas_sg_before != nullptr);
  item->dispatchPress(4, 2.0f, 2.0f);
  NVE_CHECK(item->sceneNodeText(4) == "Frame:1");
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats s2 = item->sceneGraphStats();
  NVE_CHECK(s2.canvasRasterizations == 1u);
  NVE_CHECK(s2.canvasTextureNodes == 1u);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3) == canvas_sg_before);

  // Update canvas drawList bytes to pink -> re-rasterizes once (2 total)!
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(pink_bytes);
  NVE_CHECK(item->renderFrame() == 0);
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats s3 = item->sceneGraphStats();
  NVE_CHECK(s3.canvasRasterizations == 2u);
  NVE_CHECK(s3.canvasTextureNodes == 1u);

  // Corrupt draw list bytes evict canvas texture cleanly without crashing!
  mock->emit_corrupt_raw = true;
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(cyan_bytes.data()), 18u);
  NVE_CHECK(item->renderFrame() == 0);
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats s4 = item->sceneGraphStats();
  NVE_CHECK(s4.canvasTextureNodes == 0u);
  NVE_CHECK(s4.cachedCanvasTextures == 0u);

  sg_root.reset();
}

} // namespace

int main(int argc, char* argv[]) {
  // Deterministic software Scene Graph renderer for headless CI under QT_QPA_PLATFORM=offscreen.
  QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);
  QApplication app(argc, argv);

  test_qml_registration_and_quick_view();
  test_dylib_errors_and_hot_reload();
  test_scene_hit_test_direct_events_and_scene_graph_caching();
  test_mmap_snapshot_roundtrip();
  test_z_index_hit_test_and_paint_order();
  test_live_counter_and_studio_dylibs();
  test_accessibility_and_ime_input_method();
  test_quick_image_rendering_and_retained_sg_survival();
  test_quick_draw_list_canvas_gpu_cache_and_corrupt_survival();

  std::cout << "nve_qt_quick_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+Qt6Quick" : "Standalone+Qt6Quick")
            << ")\n";
  return 0;
}
