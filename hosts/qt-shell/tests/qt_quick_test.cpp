#include "NveQtQuickItem.hpp"
#include "qt_test_helpers.hpp"

#include <QApplication>
#include <QCoreApplication>
#include <QImage>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQmlComponent>
#include <QQuickView>
#include <QQuickWindow>
#include <QSGClipNode>
#include <QSGGeometryNode>
#include <QSGNode>
#include <QSGOpacityNode>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QUrl>
#include <QWheelEvent>

#include <atomic>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <memory>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace {

using nve::qt_test::approx_eq;
using nve::qt_test::BinaryCmdHelper;
using nve::qt_test::DirectOutboundQueue;

std::span<const uint8_t> nve_test_patch(std::string_view text) {
  return {reinterpret_cast<const uint8_t*>(text.data()), text.size()};
}

struct RecordedEvent {
  uint32_t node_id{0};
  uint64_t table{0};
  uint32_t event_id{0};
  float x{0.0f};
  float y{0.0f};
  float width{0.0f};
  float height{0.0f};
  std::string text;
};

struct MockQuickComponent {
  static inline MockQuickComponent* last_instance = nullptr;

  uint32_t surface_id{1};
  int count{0};
  std::string title{"QuickGPU"};
  std::string input_text{"hi"};
  bool show_extra_text{true};
  std::vector<RecordedEvent> direct_events;
  int direct_press_calls{0};
  int direct_scroll_calls{0};
  int direct_text_calls{0};
  int direct_focus_calls{0};
  int direct_blur_calls{0};
  BinaryCmdHelper cmd;
  DirectOutboundQueue outbound;
  std::string state_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQuickComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }

  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockQuickComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQuickComponent*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_VIEW);
    self->cmd.create_node(3, NVE_COMP_TEXT);
    self->cmd.create_node(4, NVE_COMP_TEXT);
    self->cmd.create_node(5, NVE_COMP_PRESSABLE);
    self->cmd.create_node(6, NVE_COMP_TEXT);
    self->cmd.create_node(7, NVE_COMP_VIEW);
    self->cmd.create_node(8, NVE_COMP_TEXT);
    self->cmd.create_node(9, NVE_COMP_SCROLL_VIEW);
    self->cmd.create_node(10, NVE_COMP_VIEW);
    self->cmd.create_node(11, NVE_COMP_PRESSABLE);
    self->cmd.create_node(12, NVE_COMP_TEXT);
    self->cmd.create_node(13, NVE_COMP_TEXT_INPUT);
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

    self->cmd.set_layout(2, 0, 2, 100.0f, 2, 100.0f, 0.0f, 16.0f, 8.0f);
    self->cmd.set_style(2, 0xFF1E170Fu, 0xFFFFFFFFu, 0xFF334155u, 1.0f, 0.0f, 1.0f, 16.0f);

    self->cmd.set_layout(3, 0, 1, 240.0f, 1, 28.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(3, self->title);
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 18.0f);

    self->cmd.set_layout(4, 0, 1, 200.0f, 1, 26.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(4, "Clicks: " + std::to_string(self->count));
    self->cmd.set_style(4, 0x00000000u, 0xFF38BDF8u, 0, 0.0f, 0.0f, 1.0f, 16.0f);

    self->cmd.set_layout(5, 0, 1, 180.0f, 1, 40.0f, 0.0f, 8.0f, 0.0f);
    self->cmd.set_style(5, 0xFFEB6325u, 0xFFFFFFFFu, 0xFFFFFFFFu, 2.0f, 12.0f, 1.0f, 16.0f);
    self->cmd.set_event_mask(5, nve::kEventMaskPress);
    self->cmd.set_layout(6, 0, 1, 140.0f, 1, 22.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(6, "Increment Button");
    self->cmd.set_style(6, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 15.0f);

    self->cmd.set_layout(7, 0, 1, 140.0f, 1, 28.0f, 0.0f, 4.0f, 0.0f);
    self->cmd.set_style(7, 0xFF8B5CF6u, 0xFFFFFFFFu, 0, 0.0f, 8.0f, 0.5f, 14.0f);
    self->cmd.set_layout(8, 0, 1, 120.0f, 1, 20.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(8, "Translucent");
    self->cmd.set_style(8, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 13.0f);

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

    self->cmd.set_layout(13, 0, 1, 240.0f, 1, 32.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(13, self->input_text);
    self->cmd.set_style(13, 0xFF0F172Au, 0xFFFFFFFFu, 0xFF64748Bu, 1.0f, 6.0f, 1.0f, 14.0f);

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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickComponent*>(h);
    if (batch.events == nullptr) return 0u;
    uint32_t dirty = 0u;
    for (uint32_t i = 0u; i < batch.event_count; ++i) {
      const NveEvent& ev = batch.events[i];
      RecordedEvent rec{};
      rec.node_id = ev.node;
      rec.table = ev.table;
      rec.event_id = ev.event;
      rec.x = nve::event_payload_f32(ev, 0u);
      rec.y = nve::event_payload_f32(ev, 1u);
      rec.width = nve::event_payload_f32(ev, 2u);
      rec.height = nve::event_payload_f32(ev, 3u);
      if (ev.bytes.len > 0u && batch.bytes != nullptr && ev.bytes.offset <= batch.bytes_len &&
          ev.bytes.len <= batch.bytes_len - ev.bytes.offset) {
        rec.text.assign(
            reinterpret_cast<const char*>(batch.bytes + ev.bytes.offset), ev.bytes.len);
      }
      self->direct_events.push_back(rec);

      if (ev.table == NVE_TABLE_INTERACTION && ev.event == NVE_INTERACTION_EVENT_PRESS) {
        ++self->direct_press_calls;
        if (ev.node == 5u || ev.node == 11u) {
          ++self->count;
          self->outbound.push_custom(
              self->surface_id,
              ev.node,
              ev.timestamp,
              "onQuickPress",
              "{\"count\":" + std::to_string(self->count) + "}");
          dirty = 1u;
        }
      } else if (ev.table == NVE_TABLE_SCROLL && ev.event == NVE_SCROLL_EVENT_SCROLL) {
        ++self->direct_scroll_calls;
      } else if (ev.table == NVE_TABLE_TEXT_INPUT && ev.event == NVE_TEXT_INPUT_EVENT_CHANGE) {
        ++self->direct_text_calls;
        if (ev.node == 13u) {
          self->input_text = rec.text;
          dirty = 1u;
        }
      } else if (ev.table == NVE_TABLE_INTERACTION && ev.event == NVE_INTERACTION_EVENT_FOCUS) {
        ++self->direct_focus_calls;
      } else if (ev.table == NVE_TABLE_INTERACTION && ev.event == NVE_INTERACTION_EVENT_BLUR) {
        ++self->direct_blur_calls;
      }
    }
    return dirty;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockQuickComponent*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "extra" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->show_extra_text = entries[i].value.data.bool_val != 0u;
      } else if (k == "title" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        self->title.assign(v.data(), v.size());
      }
    }
    return 1u;
  }

  static NveEventBatch drain_emitted_events(NveAppHandle h) {
    return static_cast<MockQuickComponent*>(h)->outbound.drain();
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
    auto* self = static_cast<MockQuickComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch.ptr), patch.len);
    return 1u;
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
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
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
  if (node == nullptr) return;
  ++out.total;
  switch (node->type()) {
    case QSGNode::TransformNodeType:
      ++out.transform_nodes;
      break;
    case QSGNode::OpacityNodeType: {
      ++out.opacity_nodes;
      const auto* op = static_cast<const QSGOpacityNode*>(node);
      if (std::fabs(op->opacity() - 0.5) < 0.05) out.saw_half_opacity = true;
      break;
    }
    case QSGNode::ClipNodeType: {
      ++out.clip_nodes;
      const auto* clip = static_cast<const QSGClipNode*>(node);
      if (clip->isRectangular()) ++out.rect_clip_nodes;
      else ++out.rounded_clip_nodes;
      break;
    }
    case QSGNode::GeometryNodeType: {
      ++out.geometry_nodes;
      if (dynamic_cast<const QSGSimpleRectNode*>(node) != nullptr) ++out.simple_rect_nodes;
      if (const auto* tex = dynamic_cast<const QSGSimpleTextureNode*>(node)) {
        ++out.texture_nodes;
        NVE_CHECK(tex->texture() != nullptr && !tex->ownsTexture());
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
  QMouseEvent press(QEvent::MouseButtonPress, pos, pos, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
  QMouseEvent release(QEvent::MouseButtonRelease, pos, pos, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
  QCoreApplication::sendEvent(item, &press);
  QCoreApplication::sendEvent(item, &release);
}

void send_quick_wheel(QQuickItem* item, const QPointF& pos, const QPoint& pixel_delta) {
  QWheelEvent wheel(pos, pos, pixel_delta, QPoint(0, 0), Qt::NoButton, Qt::NoModifier, Qt::ScrollUpdate, false);
  QCoreApplication::sendEvent(item, &wheel);
}

void send_quick_key(QQuickItem* item, int key, const QString& text) {
  QKeyEvent ev(QEvent::KeyPress, key, Qt::NoModifier, text);
  QCoreApplication::sendEvent(item, &ev);
}

void test_qml_registration_and_quick_view() {
  NVE_CHECK(nve::NveQtQuickItem::registerQmlType() >= 0);

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
  NVE_CHECK(item->surfaceId() == 11u && !item->isAppLoaded());

  view.setContent(url, &comp, obj);
  NVE_CHECK(view.rootObject() == item);
  NVE_CHECK(item->attachInterface(&MockQuickComponent::get_interface, 11u));
  NVE_CHECK(item->isAppLoaded() && item->nodeCount() >= 15u && item->sceneGeneration() > 0u);
}

void test_dylib_errors_and_hot_reload() {
  nve::NveQtQuickItem item;
  item.setSize(QSizeF(420, 260));

  int err_code = 0;
  QString err_msg;

  NVE_CHECK(!item.loadDylib(QString(), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!item.loadDylib(QStringLiteral("/nonexistent/missing.dylib"), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!item.loadDylib(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!item.loadDylib(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && !err_msg.isEmpty() && !item.isAppLoaded());

  err_code = -1;
  err_msg = QStringLiteral("stale");
  NVE_CHECK(item.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 5, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty() && item.isAppLoaded());
  NVE_CHECK(item.surfaceId() == 5u && item.nodeCount() == 3u);
  NVE_CHECK(item.sceneView().schema_hash == NVE_SCENE_SCHEMA_HASH);
  NVE_CHECK(item.sceneNodeText(3) == "Fixture:0");

  item.dispatchPress(2, 6.0f, 6.0f);
  item.dispatchPress(2, 6.0f, 6.0f);
  NVE_CHECK(item.sceneNodeText(3) == "Fixture:2");

  NVE_CHECK(!item.hotReload(QStringLiteral("/nonexistent/fail.dylib"), &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && item.sceneNodeText(3) == "Fixture:2");
  NVE_CHECK(!item.hotReload(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && item.sceneNodeText(3) == "Fixture:2");
  NVE_CHECK(!item.hotReload(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && item.sceneNodeText(3) == "Fixture:2");

  NVE_CHECK(item.hotReload(QStringLiteral(NVE_TEST_VALID_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty() && item.sceneNodeText(3) == "Fixture:2");

  NVE_CHECK(item.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 5, &err_code, &err_msg));
  NVE_CHECK(item.sceneNodeText(3) == "Fixture:0");
}

void test_scene_hit_test_direct_events_and_scene_graph_caching() {
  QQuickWindow window;
  window.resize(520, 420);

  auto* item = new nve::NveQtQuickItem(window.contentItem());
  item->setSize(QSizeF(520, 420));

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

  const QRectF btn5_bounds = item->sceneNodeGlobalBounds(5);
  NVE_CHECK(btn5_bounds.isValid() && btn5_bounds.width() > 100.0 && btn5_bounds.height() > 20.0);
  const QPointF btn5_center = btn5_bounds.center();
  NVE_CHECK(item->hitTestNode(btn5_center, true) == 5u);
  NVE_CHECK(item->hitTestNode(QPointF(5000.0, 5000.0), false) == 0u);

  send_quick_click(item, btn5_center);
  NVE_CHECK(mock->direct_press_calls == 1 && item->sceneNodeText(4) == "Clicks: 1");
  NVE_CHECK(!mock->direct_events.empty());
  const auto& first_ev = mock->direct_events.back();
  NVE_CHECK(first_ev.node_id == 5u && first_ev.table == NVE_TABLE_INTERACTION &&
            first_ev.event_id == NVE_INTERACTION_EVENT_PRESS);
  NVE_CHECK(approx_eq(first_ev.x, btn5_bounds.width() * 0.5, 1.5));
  NVE_CHECK(approx_eq(first_ev.y, btn5_bounds.height() * 0.5, 1.5));
  NVE_CHECK(emitted_events.size() == 1u && emitted_events[0].name == "onQuickPress");

  {
    QMouseEvent press(QEvent::MouseButtonPress, btn5_center, btn5_center, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QMouseEvent move(QEvent::MouseMove, QPointF(480.0, 400.0), QPointF(480.0, 400.0), Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
    QMouseEvent release(QEvent::MouseButtonRelease, QPointF(480.0, 400.0), QPointF(480.0, 400.0), Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(item, &press);
    QCoreApplication::sendEvent(item, &move);
    QCoreApplication::sendEvent(item, &release);
  }
  NVE_CHECK(mock->direct_press_calls == 1 && item->sceneNodeText(4) == "Clicks: 1");

  const QRectF scroll9_bounds = item->sceneNodeGlobalBounds(9);
  const QRectF scroll_item11_before = item->sceneNodeGlobalBounds(11);
  NVE_CHECK(scroll9_bounds.isValid() && approx_eq(scroll9_bounds.height(), 60.0));
  NVE_CHECK(item->hitTestScrollableNode(scroll9_bounds.center()) == 9u);

  send_quick_wheel(item, scroll9_bounds.center(), QPoint(0, -40));
  NVE_CHECK(mock->direct_scroll_calls == 1);
  NVE_CHECK(approx_eq(item->scrollOffsetForNode(9).y(), 40.0, 0.5));
  NVE_CHECK(approx_eq(item->sceneNodeGlobalBounds(11).y(), scroll_item11_before.y() - 40.0, 0.5));

  send_quick_wheel(item, scroll9_bounds.center(), QPoint(0, -500));
  NVE_CHECK(approx_eq(item->scrollOffsetForNode(9).y(), 160.0, 0.5));
  NVE_CHECK(item->hitTestNode(scroll_item11_before.center(), true) != 11u);

  item->setScrollOffsetForNode(9, QPointF(0.0, 0.0));
  NVE_CHECK(item->hitTestNode(scroll_item11_before.center(), true) == 11u);
  send_quick_click(item, scroll_item11_before.center());
  NVE_CHECK(mock->direct_press_calls == 2 && item->sceneNodeText(4) == "Clicks: 2");

  const QRectF input13_bounds = item->sceneNodeGlobalBounds(13);
  NVE_CHECK(input13_bounds.isValid());
  send_quick_click(item, input13_bounds.center());
  NVE_CHECK(item->focusedNodeId() == 13u && mock->direct_focus_calls == 1);

  send_quick_key(item, Qt::Key_Exclam, QStringLiteral("!"));
  send_quick_key(item, Qt::Key_Q, QStringLiteral("Q"));
  NVE_CHECK(mock->direct_text_calls == 2 && item->sceneNodeText(13) == "hi!Q");

  send_quick_key(item, Qt::Key_Backspace, QString());
  NVE_CHECK(mock->direct_text_calls == 3 && item->sceneNodeText(13) == "hi!");

  send_quick_key(item, Qt::Key_Escape, QString());
  NVE_CHECK(item->focusedNodeId() == 0u && mock->direct_blur_calls == 1);

  if (nve::fabric_host_has_yoga_layout()) {
    const QRectF box15_before = item->sceneNodeGlobalBounds(15);
    item->setSize(QSizeF(620, 420));
    const QRectF box15_after = item->sceneNodeGlobalBounds(15);
    NVE_CHECK(approx_eq(box15_after.width() - box15_before.width(), 100.0, 2.5));
  }

  window.show();
  QCoreApplication::processEvents();
  const QImage grabbed = window.grabWindow();
  NVE_CHECK(!grabbed.isNull() && grabbed.width() > 0 && grabbed.height() > 0);
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

  std::unique_ptr<QSGNode> sg_root(item->syncSceneGraphNode(nullptr));
  NVE_CHECK(sg_root != nullptr);

  SGTreeAudit audit1{};
  audit_sg_tree(sg_root.get(), audit1);
  NVE_CHECK(audit1.transform_nodes >= 6);
  NVE_CHECK(audit1.opacity_nodes >= 1 && audit1.saw_half_opacity);
  NVE_CHECK(audit1.clip_nodes >= 3 && audit1.rect_clip_nodes >= 1 && audit1.rounded_clip_nodes >= 2);
  NVE_CHECK(audit1.geometry_nodes >= 6 && audit1.simple_rect_nodes >= 1 && audit1.texture_nodes == 7);

  const nve::SceneGraphStats stats1 = item->sceneGraphStats();
  NVE_CHECK(stats1.textTextureNodes == 7u && stats1.textRasterizations == 7u);
  NVE_CHECK(stats1.textCacheHits == 0u && stats1.cachedTextTextures == 7u);

  NVE_CHECK(item->syncSceneGraphNode(sg_root.get()) == sg_root.get());
  NVE_CHECK(item->sceneGraphStats().textRasterizations == 7u && item->sceneGraphStats().textCacheHits == 0u);

  const QSGNode* header3_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3);
  const QSGNode* button5_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 5);
  const QSGNode* badge7_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 7);
  const QSGNode* scroll9_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 9);
  NVE_CHECK(header3_sg_before && button5_sg_before && badge7_sg_before && scroll9_sg_before);

  item->dispatchPress(5, 10.0f, 10.0f);
  NVE_CHECK(item->sceneNodeText(4) == "Clicks: 3");
  NVE_CHECK(item->syncSceneGraphNode(sg_root.get()) == sg_root.get());
  const nve::SceneGraphStats stats3 = item->sceneGraphStats();
  NVE_CHECK(stats3.textRasterizations == 8u && stats3.textCacheHits == 6u && stats3.cachedTextTextures == 7u);
  NVE_CHECK(stats3.dirtyNodesUpdated == 1u && stats3.retainedNodesReused >= 14u);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3) == header3_sg_before);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 5) == button5_sg_before);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 7) == badge7_sg_before);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 9) == scroll9_sg_before);

  const NveMapEntry entries[1] = {
      NveMapEntry{nve::make_str_slice("extra"), nve::make_bool_value(false)},
  };
  NVE_CHECK(item->applyProps(entries, 1u));
  NVE_CHECK(!item->sceneNode(16).has_value());
  NVE_CHECK(item->syncSceneGraphNode(sg_root.get()) == sg_root.get());
  const nve::SceneGraphStats stats4 = item->sceneGraphStats();
  NVE_CHECK(stats4.textTextureNodes == 6u && stats4.cachedTextTextures == 6u);
  NVE_CHECK(stats4.textRasterizations == 8u && stats4.textCacheHits == 12u);

  NVE_CHECK(item->applyHotPatch(nve_test_patch("PatchedQuickTitle")));
  NVE_CHECK(item->sceneNodeText(3) == "PatchedQuickTitle");
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats stats5 = item->sceneGraphStats();
  NVE_CHECK(stats5.textRasterizations == 9u && stats5.textCacheHits == 17u);

  sg_root.reset();
}

} // namespace

void nve_test_qt_quick_extra();
void nve_test_qt_quick_media_canvas();

int main(int argc, char* argv[]) {
  QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);
  QApplication app(argc, argv);

  test_qml_registration_and_quick_view();
  test_dylib_errors_and_hot_reload();
  test_scene_hit_test_direct_events_and_scene_graph_caching();
  nve_test_qt_quick_extra();
  nve_test_qt_quick_media_canvas();

  std::cout << "nve_qt_quick_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+Qt6Quick" : "Standalone+Qt6Quick")
            << ")\n";
  return 0;
}
