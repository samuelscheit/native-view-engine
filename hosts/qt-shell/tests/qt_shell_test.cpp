#include "NveQtWidget.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_pair_client.hpp"
#ifdef NVE_QT_HAS_QUICK
#include "NveQtQuickItem.hpp"
#endif

#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <chrono>
#include <thread>

#include <QAccessible>
#include <QApplication>
#include <QBuffer>
#include <QColor>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFont>
#include <QImage>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QMouseEvent>
#include <QPalette>
#include <QPixmap>
#include <QScrollArea>
#include <QScrollBar>
#include <QUrl>

#include <atomic>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <iostream>
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
    push<uint8_t>(1); // Align FlexStart so auto-sized children invoke text measurement
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
      float font_size) {
    push<uint8_t>(NVE_OP_SET_STYLE);
    push<uint32_t>(id);
    push<uint32_t>(bg);
    push<uint32_t>(fg);
    push<uint32_t>(border_rgba);
    push<float>(border_w);
    push<float>(radius);
    push<float>(opacity);
    push<float>(font_size);
    push<int32_t>(0);
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

/// In-process mock component exercising layout, props, events, scroll/input widgets,
/// and leaf-text <-> wrapper-text tag recreation on the same node ID.
struct MockQtRustComponent {
  uint32_t surface_id{1};
  int count{0};
  std::string title{"Initial"};
  bool morph_text_to_wrapper{false};
  BinaryCmdHelper cmd;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQtRustComponent();
    self->surface_id = sid;
    self->outbound.clear();
    return self;
  }

  static void destroy(NveAppHandle h) { delete static_cast<MockQtRustComponent*>(h); }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_VIEW);
    self->cmd.create_node(3, NVE_COMP_TEXT);
    self->cmd.create_node(4, NVE_COMP_PRESSABLE);
    self->cmd.create_node(5, NVE_COMP_TEXT);
    self->cmd.create_node(6, NVE_COMP_SCROLL_VIEW);
    self->cmd.create_node(7, NVE_COMP_TEXT_INPUT);

    if (self->morph_text_to_wrapper) {
      self->cmd.create_node(8, NVE_COMP_TEXT);
      self->cmd.set_children(5, {8});
      self->cmd.set_text(8, "WrappedChild");
      self->cmd.set_style(8, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 14.0f);
    } else {
      self->cmd.set_children(5, {});
      self->cmd.set_text(5, "LeafText");
    }

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
    self->cmd.set_children(2, {3, 4, 5, 6});
    self->cmd.set_children(6, {7});

    // Row container: 100% x 100%, padding=20, gap=10
    self->cmd.set_layout(2, 1, 2, 100.0f, 2, 100.0f, 0.0f, 20.0f, 10.0f);
    self->cmd.set_style(2, 0xFF202020u, 0xFFFFFFFFu, 0xFF404040u, 1.0f, 8.0f, 1.0f, 16.0f);

    // Label 3: fixed 120x40
    self->cmd.set_layout(3, 0, 1, 120.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(3, self->title + ":" + std::to_string(self->count));
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 18.0f);

    // Pressable 4: flex_grow=1, height=40
    self->cmd.set_layout(4, 0, 0, 0.0f, 1, 40.0f, 1.0f, 0.0f, 0.0f);
    self->cmd.set_style(4, 0xFFFF5500u, 0xFFFFFFFFu, 0xFFFFFFFFu, 1.5f, 12.0f, 0.95f, 16.0f);

    // Morphable text node 5: width=80, height=auto (exercises Yoga QFontMetricsF measurer)
    self->cmd.set_layout(5, 0, 1, 80.0f, 0, 0.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(5, 0x00000000u, 0xFF38BDF8u, 0, 0.0f, 0.0f, 1.0f, 14.0f);

    // ScrollView 6 + TextInput 7
    self->cmd.set_layout(6, 0, 1, 80.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_layout(7, 0, 1, 70.0f, 1, 30.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(7, "hello");
    self->cmd.set_style(7, 0xFFFFFFFFu, 0xFF000000u, 0, 0.0f, 0.0f, 1.0f, 14.0f);

    return self->cmd.finish();
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) {
      return 0;
    }
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4) {
        ++self->count;
        const std::string payload = "{\"count\":" + std::to_string(self->count) + "}";
        self->outbound.push_custom(
            self->surface_id,
            4,
            ev.timestamp_us,
            "onIncrement",
            reinterpret_cast<const uint8_t*>(payload.data()),
            payload.size());
        return 1;
      }
    }
    return 0;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("Updated") != std::string::npos) {
      self->title = "Updated";
    }
    if (json.find("\"morph\":true") != std::string::npos) {
      self->morph_text_to_wrapper = true;
    } else if (json.find("\"morph\":false") != std::string::npos) {
      self->morph_text_to_wrapper = false;
    }
    const std::string payload = "{\"ok\":true}";
    self->outbound.push_custom(
        self->surface_id,
        NVE_ROOT_NODE_ID,
        777,
        "onPropsApplied",
        reinterpret_cast<const uint8_t*>(payload.data()),
        payload.size());
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch.ptr), patch.len);
    return 1;
  }

  static NveBufferSlice snapshot_state(NveAppHandle) { return {nullptr, 0}; }
  static void restore_state(NveAppHandle, NveBufferSlice) {}

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
        .dispatch_events_direct = nullptr,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
        .set_props_direct = nullptr,
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

void send_left_click(QWidget* target, const QPointF& local_pos) {
  const QPointF global_pos = target->mapToGlobal(local_pos);
  QMouseEvent press(
      QEvent::MouseButtonPress, local_pos, global_pos, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
  QMouseEvent release(
      QEvent::MouseButtonRelease, local_pos, global_pos, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
  QCoreApplication::sendEvent(target, &press);
  QCoreApplication::sendEvent(target, &release);
}

void test_error_codes_and_stateful_fixture() {
  nve::qt::NveQtWidget widget;
  widget.resize(400, 240);

  int err_code = 0;
  QString err_msg;

  // Error code 1: empty & nonexistent path
  NVE_CHECK(!widget.loadDylib(QString(), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!widget.loadDylib(QStringLiteral("/nonexistent/libnve_missing.dylib"), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  // Error code 2: valid dylib missing nve_get_interface
  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!widget.loadDylib(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && !err_msg.isEmpty());

  // Error code 3: exports nve_get_interface but missing NVE_IFACE_SURFACE
  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!widget.loadDylib(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && !err_msg.isEmpty());
  NVE_CHECK(!widget.isAppLoaded());

  // Load valid stateful fixture dylib
  err_code = -1;
  err_msg = QStringLiteral("stale");
  NVE_CHECK(widget.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 7, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  NVE_CHECK(widget.isAppLoaded());
  NVE_CHECK(widget.surfaceId() == 7);
  NVE_CHECK(widget.nodeCount() == 3);

  auto* lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:0"));

  // Press twice -> Fixture:2
  widget.simulatePressOnNodeId(2, 5.0f, 5.0f);
  widget.simulatePressOnNodeId(2, 5.0f, 5.0f);
  lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:2"));

  // Failed reload (codes 1/2/3) must leave the live app and Fixture:2 intact
  NVE_CHECK(!widget.reloadDylib(QStringLiteral("/nonexistent/fail.dylib"), &err_code, &err_msg));
  NVE_CHECK(err_code == 1);
  NVE_CHECK(qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3))->text() == QStringLiteral("Fixture:2"));

  NVE_CHECK(!widget.reloadDylib(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 2);
  NVE_CHECK(qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3))->text() == QStringLiteral("Fixture:2"));

  NVE_CHECK(!widget.reloadDylib(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 3);
  NVE_CHECK(qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3))->text() == QStringLiteral("Fixture:2"));

  // Successful warm reload preserves Fixture:2
  NVE_CHECK(widget.reloadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:2"));

  // Fresh load resets state back to Fixture:0
  NVE_CHECK(widget.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 7, &err_code, &err_msg));
  lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:0"));
}

void test_attach_props_events_morph_and_measurer() {
  nve::qt::NveQtWidget widget;
  widget.resize(520, 180);

  // Verify QFontMetricsF text measurer directly + wrap with spy to confirm invocation.
  auto base_measurer = nve::qt::NveQtWidget::makeQtTextMeasurer();
  const NveIntrinsicSize empty_sz = base_measurer("", 16.0f, 0.0f);
  NVE_CHECK(empty_sz.width == 0.0f && empty_sz.height == 0.0f);
  const NveIntrinsicSize s14 = base_measurer("NATIVE VIEW ENGINE", 14.0f, 0.0f);
  const NveIntrinsicSize s28 = base_measurer("Count: 0", 28.0f, 0.0f);
  NVE_CHECK(s14.width > 40.0f && s14.height >= 14.0f);
  NVE_CHECK(s28.width > 40.0f && s28.height > s14.height);
  const NveIntrinsicSize wrapped =
      base_measurer("This is a long sentence that wraps onto multiple lines", 16.0f, 90.0f);
  NVE_CHECK(wrapped.width <= 92.0f && wrapped.height > s14.height * 2.0f);

  std::atomic<int> measure_calls{0};
  widget.setTextMeasurer(
      [&](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
        measure_calls.fetch_add(1, std::memory_order_relaxed);
        return base_measurer(text, font_size, max_width);
      });

  std::vector<nve::DecodedEvent> received_events;
  widget.setOnNveEvent([&](const nve::DecodedEvent& ev) { received_events.push_back(ev); });

  NVE_CHECK(widget.attachInterface(&MockQtRustComponent::get_interface, 9));
  NVE_CHECK(widget.surfaceId() == 9);
  NVE_CHECK(widget.nodeCount() == 7);
  if (nve::fabric_host_has_yoga_layout()) {
    NVE_CHECK(measure_calls.load() > 0);
  }

  QWidget* container = widget.nativeWidgetForNodeId(2);
  auto* label3 = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  QWidget* button4 = widget.nativeWidgetForNodeId(4);
  QWidget* morph5 = widget.nativeWidgetForNodeId(5);
  auto* scroll6 = qobject_cast<QScrollArea*>(widget.nativeWidgetForNodeId(6));
  auto* edit7 = qobject_cast<QLineEdit*>(widget.nativeWidgetForNodeId(7));

  NVE_CHECK(container != nullptr && label3 != nullptr && button4 != nullptr);
  NVE_CHECK(qobject_cast<QLabel*>(morph5) != nullptr);
  NVE_CHECK(scroll6 != nullptr && edit7 != nullptr);
  NVE_CHECK(label3->parentWidget() == container);
  NVE_CHECK(button4->parentWidget() == container);
  NVE_CHECK(edit7->parentWidget() == scroll6->widget());
  NVE_CHECK(label3->text() == QStringLiteral("Initial:0"));
  NVE_CHECK(label3->testAttribute(Qt::WA_TransparentForMouseEvents));
  NVE_CHECK(edit7->text() == QStringLiteral("hello"));

  if (nve::fabric_host_has_yoga_layout()) {
    // 520 wide, padding=20 each side => inner=480; 3 gaps of 10 => 450;
    // label3=120, morph5=80, scroll6=80 => button4 flex_grow=1 gets 170.
    NVE_CHECK(approx_eq(container->width(), 520.0));
    NVE_CHECK(approx_eq(label3->x(), 20.0) && approx_eq(label3->width(), 120.0));
    NVE_CHECK(approx_eq(button4->x(), 150.0) && approx_eq(button4->width(), 170.0));
  }

  // Apply props JSON -> updates text + emits onPropsApplied
  NVE_CHECK(widget.applyPropsJson(R"({"title":"Updated"})"));
  NVE_CHECK(label3->text() == QStringLiteral("Updated:0"));
  NVE_CHECK(received_events.size() == 1);
  NVE_CHECK(received_events[0].name == "onPropsApplied");

  // Simulate press via API and via real Qt mouse event
  widget.simulatePressOnNodeId(4, 10.0f, 10.0f);
  NVE_CHECK(label3->text() == QStringLiteral("Updated:1"));
  NVE_CHECK(received_events.size() == 2);
  NVE_CHECK(received_events[1].name == "onIncrement");

  send_left_click(button4, QPointF(10.0, 10.0));
  NVE_CHECK(label3->text() == QStringLiteral("Updated:2"));
  NVE_CHECK(received_events.size() == 3);

  // Leaf NVE_COMP_TEXT -> wrapper NVE_COMP_TEXT tag recreation (QLabel* -> container widget)
  NVE_CHECK(widget.applyPropsJson(R"({"morph":true})"));
  QWidget* morph5_wrapper = widget.nativeWidgetForNodeId(5);
  NVE_CHECK(morph5_wrapper != nullptr);
  NVE_CHECK(morph5_wrapper != morph5);
  NVE_CHECK(qobject_cast<QLabel*>(morph5_wrapper) == nullptr);
  auto* child8 = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(8));
  NVE_CHECK(child8 != nullptr && child8->parentWidget() == morph5_wrapper);
  NVE_CHECK(child8->text() == QStringLiteral("WrappedChild"));

  // Wrapper NVE_COMP_TEXT -> leaf NVE_COMP_TEXT recreation (container -> QLabel*)
  NVE_CHECK(widget.applyPropsJson(R"({"morph":false})"));
  QWidget* morph5_leaf = widget.nativeWidgetForNodeId(5);
  auto* morph5_label = qobject_cast<QLabel*>(morph5_leaf);
  NVE_CHECK(morph5_label != nullptr);
  NVE_CHECK(morph5_label->text() == QStringLiteral("LeafText"));

  // Live resize recalculates Yoga Flexbox widths dynamically
  if (nve::fabric_host_has_yoga_layout()) {
    widget.show();
    widget.resize(620, 180);
    QCoreApplication::processEvents();
    NVE_CHECK(approx_eq(button4->width(), 270.0));
  }
}

void test_real_counter_dylib() {
  const char* dylib_env = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (dylib_env == nullptr || dylib_env[0] == '\0') {
    return;
  }

  nve::qt::NveQtWidget widget;
  widget.resize(1280, 800);

  auto base_measurer = nve::qt::NveQtWidget::makeQtTextMeasurer();
  std::atomic<int> counter_measure_calls{0};
  widget.setTextMeasurer(
      [&](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
        counter_measure_calls.fetch_add(1, std::memory_order_relaxed);
        return base_measurer(text, font_size, max_width);
      });

  int err_code = -1;
  QString err_msg;
  const QString counter_path = QString::fromUtf8(dylib_env);
  NVE_CHECK(widget.loadDylib(counter_path, 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  if (nve::fabric_host_has_yoga_layout()) {
    NVE_CHECK(counter_measure_calls.load() >= 4);
  }

  widget.show();
  QCoreApplication::processEvents();

  auto collect_labels = [&]() { return widget.findChildren<QLabel*>(); };
  const auto find_label = [&](const QString& text) -> QLabel* {
    for (QLabel* lbl : collect_labels()) {
      if (lbl->text() == text) {
        return lbl;
      }
    }
    return nullptr;
  };

  const QList<QLabel*> initial_labels = collect_labels();
  NVE_CHECK(initial_labels.size() >= 4);

  QLabel* title_lbl = find_label(QStringLiteral("NVE Kitchen Sink"));
  QLabel* count_lbl = find_label(QStringLiteral("Count: 0"));
  QLabel* inc_lbl = find_label(QStringLiteral("Increment +"));
  QLabel* reset_lbl = find_label(QStringLiteral("Reset"));
  NVE_CHECK(title_lbl != nullptr && count_lbl != nullptr && inc_lbl != nullptr &&
            reset_lbl != nullptr);

  // Verify font size on Count: 0 is 28px and all 4 labels have non-zero geometry + opaque text color.
  NVE_CHECK(count_lbl->font().pixelSize() == 28);
  for (QLabel* lbl : {title_lbl, count_lbl, inc_lbl, reset_lbl}) {
    NVE_CHECK(lbl->width() > 20);
    NVE_CHECK(lbl->height() >= 14);
    const QColor fg = lbl->palette().color(QPalette::WindowText);
    NVE_CHECK(fg.alpha() == 255);
    NVE_CHECK(lbl->testAttribute(Qt::WA_TransparentForMouseEvents));
  }

  // Locate card ancestor and button pressable ancestors from snapshots.
  const auto snapshots = widget.session().snapshot_nodes();
  uint32_t inc_pressable_id = 0;
  uint32_t reset_pressable_id = 0;
  QWidget* top_card = nullptr;
  for (const auto& snap : snapshots) {
    if (snap.kind == NVE_COMP_VIEW && snap.border_radius >= 8.0f && snap.layout.width > 200.0f &&
        snap.layout.height > 60.0f) {
      top_card = widget.nativeWidgetForNodeId(snap.node_id);
    }
    if (snap.kind == NVE_COMP_PRESSABLE) {
      QWidget* btn = widget.nativeWidgetForNodeId(snap.node_id);
      if (btn != nullptr) {
        if (btn->isAncestorOf(inc_lbl)) {
          inc_pressable_id = snap.node_id;
        }
        if (btn->isAncestorOf(reset_lbl)) {
          reset_pressable_id = snap.node_id;
        }
      }
    }
  }

  NVE_CHECK(top_card != nullptr);
  NVE_CHECK(top_card->width() > 200 && top_card->height() > 60);
  NVE_CHECK(inc_pressable_id != 0 && reset_pressable_id != 0);

  QWidget* inc_btn = widget.nativeWidgetForNodeId(inc_pressable_id);
  QWidget* reset_btn = widget.nativeWidgetForNodeId(reset_pressable_id);
  NVE_CHECK(inc_btn != nullptr && reset_btn != nullptr);

  // Top card labels must sit above the buttons in widget coordinates.
  const int title_y = title_lbl->mapTo(&widget, QPoint(0, 0)).y();
  const int count_y = count_lbl->mapTo(&widget, QPoint(0, 0)).y();
  const int inc_btn_y = inc_btn->mapTo(&widget, QPoint(0, 0)).y();
  NVE_CHECK(title_y < count_y);
  NVE_CHECK(count_y + count_lbl->height() < inc_btn_y);

  // Verify button labels are centered inside their button bounds!
  auto assert_centered_in_button = [](QLabel* lbl, QWidget* btn) {
    const QPoint top_left = lbl->mapTo(btn, QPoint(0, 0));
    const double lbl_cx = static_cast<double>(top_left.x()) + static_cast<double>(lbl->width()) * 0.5;
    const double lbl_cy = static_cast<double>(top_left.y()) + static_cast<double>(lbl->height()) * 0.5;
    const double btn_cx = static_cast<double>(btn->width()) * 0.5;
    const double btn_cy = static_cast<double>(btn->height()) * 0.5;
    NVE_CHECK(approx_eq(lbl_cx, btn_cx, 3.0));
    NVE_CHECK(approx_eq(lbl_cy, btn_cy, 3.0));
  };
  assert_centered_in_button(inc_lbl, inc_btn);
  assert_centered_in_button(reset_lbl, reset_btn);

  // Click Increment + twice -> Count: 2
  widget.simulatePressOnNodeId(inc_pressable_id, 20.0f, 20.0f);
  widget.simulatePressOnNodeId(inc_pressable_id, 20.0f, 20.0f);
  QCoreApplication::processEvents();
  NVE_CHECK(find_label(QStringLiteral("Count: 2")) != nullptr);

  // Export visual verification PNG if requested
  const char* shot_path = std::getenv("NVE_TEST_SCREENSHOT_PATH");
  if (shot_path != nullptr && shot_path[0] != '\0') {
    QPixmap pix = widget.grab();
    NVE_CHECK(!pix.isNull() && pix.width() == 640 && pix.height() == 420);
    NVE_CHECK(pix.save(QString::fromUtf8(shot_path), "PNG"));
  }

  // State-preserving reloadDylib -> still Count: 2!
  err_code = -1;
  err_msg.clear();
  NVE_CHECK(widget.reloadDylib(counter_path, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  QCoreApplication::processEvents();
  NVE_CHECK(find_label(QStringLiteral("Count: 2")) != nullptr);

  // Re-locate Reset pressable after reload and click Reset -> Count: 0!
  uint32_t reset_after_reload = 0;
  QLabel* reset_lbl_after = find_label(QStringLiteral("Reset"));
  NVE_CHECK(reset_lbl_after != nullptr);
  for (const auto& snap : widget.session().snapshot_nodes()) {
    if (snap.kind == NVE_COMP_PRESSABLE) {
      QWidget* btn = widget.nativeWidgetForNodeId(snap.node_id);
      if (btn != nullptr && btn->isAncestorOf(reset_lbl_after)) {
        reset_after_reload = snap.node_id;
        break;
      }
    }
  }
  NVE_CHECK(reset_after_reload != 0);

  // Instant-boot Frame-0 snapshot save at Count: 2 -> reset static -> cold replay BEFORE loadDylib!
  const QString counter_snap =
      QDir::temp().filePath(QStringLiteral("nve_qt_counter_%1.nvesnap").arg(QCoreApplication::applicationPid()));
  QFile::remove(counter_snap);
  NVE_CHECK(widget.saveSnapshot(counter_snap, 1));

  widget.simulatePressOnNodeId(reset_after_reload, 20.0f, 20.0f);
  QCoreApplication::processEvents();
  NVE_CHECK(find_label(QStringLiteral("Count: 0")) != nullptr);
  // Reset KitchenSinkModule's counter to 0 so cold load proves restore_state works!
  const NveMapEntry reset_initial[1] = {
      NveMapEntry{nve::make_str_slice("initialCount"), nve::make_i64_value(0)},
  };
  NVE_CHECK(widget.applyPropsDirect(reset_initial, 1u));

  nve::qt::NveQtWidget cold_counter;
  cold_counter.resize(640, 420);
  uint64_t counter_restore_us = 0;
  NVE_CHECK(cold_counter.replayFrame0Snapshot(counter_snap, 1, 1, &counter_restore_us));
  std::cout << "[qt_shell_test] Cold mmap Frame-0 restore (counter dylib): "
            << counter_restore_us << " us\n";

  // Frame-0 QLabel MUST immediately display Count: 2 before loadDylib even runs!
  bool saw_f0_count2 = false;
  for (QLabel* lbl : cold_counter.findChildren<QLabel*>()) {
    if (lbl->text() == QStringLiteral("Count: 2")) {
      saw_f0_count2 = true;
    }
  }
  NVE_CHECK(saw_f0_count2);
  NVE_CHECK(!cold_counter.isAppLoaded());

  err_code = -1;
  err_msg.clear();
  NVE_CHECK(cold_counter.loadDylib(counter_path, 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 0);
  bool saw_live_count2 = false;
  for (QLabel* lbl : cold_counter.findChildren<QLabel*>()) {
    if (lbl->text() == QStringLiteral("Count: 2")) {
      saw_live_count2 = true;
    }
  }
  NVE_CHECK(saw_live_count2);
  QFile::remove(counter_snap);
}

void test_frame0_mmap_snapshot_roundtrip() {
  const QString fixture_snap = QDir::temp().filePath(
      QStringLiteral("nve_qt_fixture_%1.nvesnap").arg(QCoreApplication::applicationPid()));
  QFile::remove(fixture_snap);

  {
    nve::qt::NveQtWidget writer;
    writer.resize(400, 240);
    NVE_CHECK(writer.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 1));
    writer.simulatePressOnNodeId(2, 5.0f, 5.0f);
    writer.simulatePressOnNodeId(2, 5.0f, 5.0f);
    writer.simulatePressOnNodeId(2, 5.0f, 5.0f);
    auto* lbl = qobject_cast<QLabel*>(writer.nativeWidgetForNodeId(3));
    NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:3"));
    NVE_CHECK(writer.saveSnapshot(fixture_snap, 2));
  }

  nve::qt::NveQtWidget cold;
  cold.resize(400, 240);
  uint64_t restore_us = 0;
  NVE_CHECK(!cold.replayFrame0Snapshot(fixture_snap, 99, 1, &restore_us));
  NVE_CHECK(cold.replayFrame0Snapshot(fixture_snap, 2, 1, &restore_us));
  std::cout << "[qt_shell_test] Cold mmap Frame-0 restore (fixture dylib): "
            << restore_us << " us\n";

  // Frame 0 mounts exact mutated QLabel BEFORE loadDylib runs!
  NVE_CHECK(!cold.isAppLoaded());
  NVE_CHECK(cold.nodeCount() == 3);
  auto* f0_lbl = qobject_cast<QLabel*>(cold.nativeWidgetForNodeId(3));
  NVE_CHECK(f0_lbl != nullptr && f0_lbl->text() == QStringLiteral("Fixture:3"));

  // Loading dylib seamlessly restores Fixture:3 via restore_state!
  NVE_CHECK(cold.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 1));
  NVE_CHECK(cold.isAppLoaded());
  auto* live_lbl = qobject_cast<QLabel*>(cold.nativeWidgetForNodeId(3));
  NVE_CHECK(live_lbl != nullptr && live_lbl->text() == QStringLiteral("Fixture:3"));
  QFile::remove(fixture_snap);

  const char* studio_env = std::getenv("NVE_TEST_STUDIO_DYLIB");
  if (studio_env != nullptr && studio_env[0] != '\0') {
    const QString studio_path = QString::fromUtf8(studio_env);
    const QString studio_snap = QDir::temp().filePath(
        QStringLiteral("nve_qt_studio_%1.nvesnap").arg(QCoreApplication::applicationPid()));
    QFile::remove(studio_snap);

    auto has_text = [](nve::qt::NveQtWidget& w, const QString& needle) {
      for (QLabel* l : w.findChildren<QLabel*>()) {
        if (l->text().contains(needle)) {
          return true;
        }
      }
      return false;
    };

    {
      nve::qt::NveQtWidget studio_writer;
      studio_writer.resize(960, 640);
      NVE_CHECK(studio_writer.loadDylib(studio_path, 1));
      const NveMapEntry studio_props[2] = {
          NveMapEntry{nve::make_str_slice("accent"), nve::make_str_value("violet")},
          NveMapEntry{nve::make_str_slice("view"), nve::make_str_value("metrics")},
      };
      NVE_CHECK(studio_writer.applyPropsDirect(studio_props, 2u));
      NVE_CHECK(has_text(studio_writer, QStringLiteral("Live Metrics")));
      NVE_CHECK(studio_writer.saveSnapshot(studio_snap, 1));
    }

    nve::qt::NveQtWidget studio_cold;
    studio_cold.resize(960, 640);
    uint64_t studio_restore_us = 0;
    NVE_CHECK(studio_cold.replayFrame0Snapshot(studio_snap, 1, 1, &studio_restore_us));
    std::cout << "[qt_shell_test] Cold mmap Frame-0 restore (studio-workspace): "
              << studio_restore_us << " us\n";
    NVE_CHECK(!studio_cold.isAppLoaded());
    NVE_CHECK(has_text(studio_cold, QStringLiteral("Live Metrics")));

    NVE_CHECK(studio_cold.loadDylib(studio_path, 1));
    NVE_CHECK(has_text(studio_cold, QStringLiteral("Live Metrics")));
    QFile::remove(studio_snap);
  }
}

struct MockA11yImeQtApp {
  static inline MockA11yImeQtApp* last_instance = nullptr;
  uint32_t surface_id{1};
  bool disabled_btn{false};
  bool secure_input{false};
  std::string input_text;
  int press_calls{0};
  int text_change_calls{0};
  int submit_calls{0};
  std::string last_text;
  std::string last_submit;
  BinaryCmdHelper cmd;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockA11yImeQtApp();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockA11yImeQtApp*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockA11yImeQtApp*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_PRESSABLE);
    self->cmd.create_node(3, NVE_COMP_TEXT);
    self->cmd.create_node(4, NVE_COMP_VIEW);
    self->cmd.create_node(5, NVE_COMP_TEXT_INPUT);

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2, 3, 4, 5});

    self->cmd.set_layout(2, 0, 1, 140.0f, 1, 36.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_raw_prop_string(2, "accessibilityLabel", "Submit Action");
    self->cmd.set_raw_prop_string(2, "accessibilityHint", "Activates submission");
    self->cmd.set_raw_prop_string(2, "testID", "qt-submit-btn");
    self->cmd.set_raw_prop_bool(2, "disabled", self->disabled_btn);

    self->cmd.set_layout(3, 0, 1, 180.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(3, "Section Title");
    self->cmd.set_raw_prop_string(3, "role", "header");
    self->cmd.set_raw_prop_string(3, "aria-label", "Main Section");

    self->cmd.set_layout(4, 0, 1, 120.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_raw_prop_string(4, "accessibilityRole", "checkbox");
    self->cmd.set_raw_prop_string(4, "accessibilityLabel", "Auto Save");
    self->cmd.set_raw_prop_string(4, "accessibilityValue", "on");

    self->cmd.set_layout(5, 0, 1, 220.0f, 1, 32.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(5, self->input_text);
    self->cmd.set_raw_prop_string(5, "placeholder", "Type CJK or accented text...");
    self->cmd.set_raw_prop_string(5, "accessibilityLabel", "Search Query");
    self->cmd.set_raw_prop_string(5, "data-testid", "qt-search-input");
    self->cmd.set_raw_prop_bool(5, "secureTextEntry", self->secure_input);

    return self->cmd.finish();
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockA11yImeQtApp*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 2) {
        ++self->press_calls;
      } else if (ev.kind == nve::EventKind::TextChange && ev.node_id == 5) {
        ++self->text_change_calls;
        self->last_text = ev.text;
      } else if (ev.kind == nve::EventKind::Custom && ev.node_id == 5 && ev.name == "onSubmit") {
        ++self->submit_calls;
        if (const auto v = nve::BinValueView::parse(ev.payload_bytes.data(), ev.payload_bytes.size());
            v && v->as_str()) {
          self->last_submit = std::string(*v->as_str());
        }
      }
    }
    return 0;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockA11yImeQtApp*>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 2) {
        ++self->press_calls;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::TextChange) && ev.node_id == 5) {
        ++self->text_change_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_text.assign(sv.data(), sv.size());
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Custom) && ev.node_id == 5 &&
                 nve::str_slice_view(ev.name) == "onSubmit") {
        ++self->submit_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_submit.assign(sv.data(), sv.size());
      }
    }
    return 0;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockA11yImeQtApp*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("\"disabled\":true") != std::string::npos) self->disabled_btn = true;
    if (json.find("\"secure\":true") != std::string::npos) self->secure_input = true;
    if (json.find("\"serverText\":\"stale\"") != std::string::npos) self->input_text = "stale";
    return 1;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveComponentApi kComp{&set_props, nullptr, nullptr};
    if (iface == NVE_IFACE_SURFACE) return &kSurface;
    if (iface == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

void test_accessibility_and_ime_text_input() {
  nve::qt::NveQtWidget widget;
  widget.resize(480, 320);
  NVE_CHECK(widget.attachInterface(&MockA11yImeQtApp::get_interface, 19));
  NVE_CHECK(MockA11yImeQtApp::last_instance != nullptr);
  auto* mock = MockA11yImeQtApp::last_instance;

  QWidget* btn = widget.nativeWidgetForNodeId(2);
  QWidget* hdr = widget.nativeWidgetForNodeId(3);
  QWidget* chk = widget.nativeWidgetForNodeId(4);
  auto* edit = qobject_cast<QLineEdit*>(widget.nativeWidgetForNodeId(5));
  NVE_CHECK(btn != nullptr && hdr != nullptr && chk != nullptr && edit != nullptr);

  // Verify accessibleName, accessibleDescription, objectName, and QAccessible interface
  NVE_CHECK(btn->accessibleName() == QStringLiteral("Submit Action"));
  NVE_CHECK(btn->accessibleDescription() == QStringLiteral("Activates submission"));
  NVE_CHECK(btn->objectName() == QStringLiteral("qt-submit-btn"));
  NVE_CHECK(widget.accessibleRoleForNodeId(2) == QAccessible::Button);
  if (QAccessibleInterface* btn_iface = QAccessible::queryAccessibleInterface(btn)) {
    NVE_CHECK(btn_iface->role() == QAccessible::Button);
    NVE_CHECK(btn_iface->text(QAccessible::Name) == QStringLiteral("Submit Action"));
    NVE_CHECK(btn_iface->text(QAccessible::Description) == QStringLiteral("Activates submission"));
    NVE_CHECK(!btn_iface->state().disabled);
  }

  NVE_CHECK(hdr->accessibleName() == QStringLiteral("Main Section"));
  NVE_CHECK(widget.accessibleRoleForNodeId(3) == QAccessible::Heading);

  NVE_CHECK(chk->accessibleName() == QStringLiteral("Auto Save"));
  NVE_CHECK(widget.accessibleRoleForNodeId(4) == QAccessible::CheckBox);
  const auto chk_sem = widget.accessibilityInfoForNodeId(4);
  NVE_CHECK(chk_sem.has_value() && chk_sem->value == QStringLiteral("on"));
  if (QAccessibleInterface* chk_iface = QAccessible::queryAccessibleInterface(chk)) {
    NVE_CHECK(chk_iface->role() == QAccessible::CheckBox);
    NVE_CHECK(chk_iface->text(QAccessible::Value) == QStringLiteral("on"));
  }

  // Verify QLineEdit IME attributes, placeholder, echo mode
  NVE_CHECK(edit->testAttribute(Qt::WA_InputMethodEnabled));
  NVE_CHECK(edit->placeholderText() == QStringLiteral("Type CJK or accented text..."));
  NVE_CHECK(edit->accessibleName() == QStringLiteral("Search Query"));
  NVE_CHECK(edit->objectName() == QStringLiteral("qt-search-input"));
  NVE_CHECK(edit->echoMode() == QLineEdit::Normal);
  NVE_CHECK(widget.accessibleRoleForNodeId(5) == QAccessible::EditableText);

  // 1. IME preedit event must NOT dispatch premature TextChange or be clobbered on re-render
  {
    QList<QInputMethodEvent::Attribute> attrs;
    QInputMethodEvent preedit_ev(QStringLiteral("にほん"), attrs);
    QCoreApplication::sendEvent(edit, &preedit_ev);
    NVE_CHECK(mock->text_change_calls == 0);

    // Server re-render mid-preedit must NOT overwrite composition!
    NVE_CHECK(widget.applyPropsJson(R"({"serverText":"stale"})"));
    NVE_CHECK(edit->text().isEmpty());
  }

  // 2. IME commit event with multi-byte CJK + accented + emoji characters dispatches clean UTF-8!
  {
    QInputMethodEvent commit_ev;
    commit_ev.setCommitString(QStringLiteral("日本語 café 🚀"));
    QCoreApplication::sendEvent(edit, &commit_ev);
    NVE_CHECK(edit->text() == QStringLiteral("日本語 café 🚀"));
    NVE_CHECK(mock->text_change_calls == 1);
    NVE_CHECK(mock->last_text == "日本語 café 🚀");
  }

  // 3. Return key dispatches onSubmit with the exact UTF-8 text
  {
    QKeyEvent ret_ev(QEvent::KeyPress, Qt::Key_Return, Qt::NoModifier, QStringLiteral("\r"));
    QCoreApplication::sendEvent(edit, &ret_ev);
    NVE_CHECK(mock->submit_calls == 1);
    NVE_CHECK(mock->last_submit == "日本語 café 🚀");
  }

  // 4. Toggle disabled=true and secureTextEntry=true
  NVE_CHECK(widget.applyPropsJson(R"({"disabled":true,"secure":true})"));
  NVE_CHECK(!btn->isEnabled());
  NVE_CHECK(edit->echoMode() == QLineEdit::Password);
  send_left_click(btn, QPointF(10.0, 10.0));
  NVE_CHECK(mock->press_calls == 0);
}

std::string make_png_data_uri(int width, int height, const QColor& color) {
  QImage img(width, height, QImage::Format_RGBA8888);
  img.fill(color);
  QByteArray png_bytes;
  QBuffer buffer(&png_bytes);
  buffer.open(QIODevice::WriteOnly);
  NVE_CHECK(img.save(&buffer, "PNG"));
  const QByteArray b64 = png_bytes.toBase64();
  return "data:image/png;base64," + std::string(b64.constData(), static_cast<size_t>(b64.size()));
}

struct MockQtImageScrollApp {
  static inline MockQtImageScrollApp* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string data_uri;
  std::string file_uri;
  std::string resize_mode{"cover"};
  int counter{0};
  int scroll_events{0};
  uint32_t last_scroll_node{0};
  float last_scroll_y{0.0f};
  float last_scroll_content_h{0.0f};
  BinaryCmdHelper cmd;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQtImageScrollApp();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }

  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockQtImageScrollApp*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQtImageScrollApp*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_VIEW);
    self->cmd.create_node(3, NVE_COMP_IMAGE);
    self->cmd.create_node(4, NVE_COMP_IMAGE);
    self->cmd.create_node(5, NVE_COMP_SCROLL_VIEW);
    self->cmd.create_node(6, NVE_COMP_VIEW);
    self->cmd.create_node(7, NVE_COMP_TEXT);
    self->cmd.create_node(8, NVE_COMP_TEXT);

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
    self->cmd.set_children(2, {3, 4, 5, 7, 8});
    self->cmd.set_children(5, {6});

    self->cmd.set_layout(2, 0, 1, 320.0f, 1, 260.0f, 0.0f, 8.0f, 6.0f);
    self->cmd.set_style(2, 0xFF101010u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    self->cmd.set_layout(3, 0, 1, 48.0f, 1, 48.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0xFFFFFFFFu, 1.0f, 8.0f, 1.0f, 14.0f);
    self->cmd.set_raw_prop_string(3, "src", self->data_uri);
    self->cmd.set_raw_prop_string(3, "resizeMode", self->resize_mode);

    self->cmd.set_layout(4, 0, 1, 32.0f, 1, 32.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(4, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 4.0f, 1.0f, 14.0f);
    self->cmd.set_raw_prop_string(4, "src", self->file_uri);
    self->cmd.set_raw_prop_string(4, "resizeMode", "contain");

    self->cmd.set_layout(5, 0, 1, 140.0f, 1, 60.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(5, 0xFF1E293Bu, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);
    self->cmd.set_layout(6, 0, 1, 140.0f, 1, 240.0f, 0.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(6, 0xFF334155u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    self->cmd.set_layout(7, 0, 1, 120.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(7, "Counter:" + std::to_string(self->counter));
    self->cmd.set_style(7, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    self->cmd.set_layout(8, 0, 1, 120.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(8, "StaticSibling");
    self->cmd.set_style(8, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    return self->cmd.finish();
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQtImageScrollApp*>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Scroll)) {
        ++self->scroll_events;
        self->last_scroll_node = ev.node_id;
        self->last_scroll_y = ev.y;
        self->last_scroll_content_h = ev.height;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 7) {
        ++self->counter;
        return 1;
      }
    }
    return 0;
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockQtImageScrollApp*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 7) {
        ++self->counter;
        return 1;
      }
    }
    return 0;
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockQtImageScrollApp*>(h);
    const std::string json(reinterpret_cast<const char*>(props.ptr), props.len);
    if (json.find("\"contain\"") != std::string::npos) self->resize_mode = "contain";
    if (json.find("\"stretch\"") != std::string::npos) self->resize_mode = "stretch";
    if (json.find("\"center\"") != std::string::npos) self->resize_mode = "center";
    if (json.find("\"bump\":true") != std::string::npos) ++self->counter;
    return 1;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, &dispatch_events_direct};
    static const NveComponentApi kComp{&set_props, nullptr, nullptr};
    if (iface == NVE_IFACE_SURFACE) return &kSurface;
    if (iface == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

void test_image_scroll_and_incremental_reconciliation() {
  const std::string red_uri = make_png_data_uri(8, 8, QColor(235, 30, 45, 255));
  const QString tmp_png = QDir::temp().filePath(
      QStringLiteral("nve_qt_img_test_%1.png").arg(QCoreApplication::applicationPid()));
  {
    QImage blue(8, 8, QImage::Format_RGBA8888);
    blue.fill(QColor(25, 110, 240, 255));
    NVE_CHECK(blue.save(tmp_png, "PNG"));
  }
  const QByteArray file_url_utf8 = QUrl::fromLocalFile(tmp_png).toString().toUtf8();
  const std::string file_uri(file_url_utf8.constData(), static_cast<size_t>(file_url_utf8.size()));

  // Verify bounded LRU NveQtImageCache decoding & hit tracking directly.
  nve::qt::NveQtImageCache cache(2u);
  const QImage img1 = cache.getImage(red_uri);
  NVE_CHECK(!img1.isNull() && img1.width() == 8 && img1.height() == 8);
  NVE_CHECK(cache.misses() == 1u && cache.hits() == 0u);
  const QImage img1_hit = cache.getImage(red_uri);
  NVE_CHECK(!img1_hit.isNull() && cache.hits() == 1u);
  const QImage img2 = cache.getImage(file_uri);
  NVE_CHECK(!img2.isNull() && img2.width() == 8 && img2.height() == 8);
  NVE_CHECK(cache.size() == 2u);

  nve::qt::NveQtWidget widget;
  widget.resize(320, 260);
  NVE_CHECK(widget.attachInterface(&MockQtImageScrollApp::get_interface, 31));
  NVE_CHECK(MockQtImageScrollApp::last_instance != nullptr);
  auto* mock = MockQtImageScrollApp::last_instance;
  mock->data_uri = red_uri;
  mock->file_uri = file_uri;
  NVE_CHECK(widget.renderFrame() == 0);

  QWidget* img3 = widget.nativeWidgetForNodeId(3);
  QWidget* img4 = widget.nativeWidgetForNodeId(4);
  auto* scroll5 = qobject_cast<QScrollArea*>(widget.nativeWidgetForNodeId(5));
  auto* counter7 = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(7));
  auto* static8 = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(8));
  NVE_CHECK(img3 != nullptr && img4 != nullptr && scroll5 != nullptr);
  NVE_CHECK(counter7 != nullptr && static8 != nullptr);
  NVE_CHECK(img3->property("nveImageDecoded").toBool());
  NVE_CHECK(img3->property("nveImageSize").toSize() == QSize(8, 8));
  NVE_CHECK(img4->property("nveImageDecoded").toBool());
  NVE_CHECK(img4->property("nveImageSize").toSize() == QSize(8, 8));
  NVE_CHECK(img3->property("nveImageResizeMode").toInt() == static_cast<int>(nve::qt::ImageResizeMode::Cover));

  // Grab rendered image widget and confirm red pixels actually painted!
  widget.show();
  QCoreApplication::processEvents();
  const QImage rendered = img3->grab().toImage();
  NVE_CHECK(!rendered.isNull() && rendered.width() == 48 && rendered.height() == 48);
  const QColor center_px = rendered.pixelColor(24, 24);
  NVE_CHECK(center_px.red() > 200 && center_px.green() < 60 && center_px.blue() < 70);

  // Switch through contain, stretch, center resize modes via raw_props!
  NVE_CHECK(widget.applyPropsJson(R"({"resizeMode":"contain"})"));
  NVE_CHECK(img3->property("nveImageResizeMode").toInt() == static_cast<int>(nve::qt::ImageResizeMode::Contain));
  NVE_CHECK(widget.applyPropsJson(R"({"resizeMode":"stretch"})"));
  NVE_CHECK(img3->property("nveImageResizeMode").toInt() == static_cast<int>(nve::qt::ImageResizeMode::Stretch));
  NVE_CHECK(widget.applyPropsJson(R"({"resizeMode":"center"})"));
  NVE_CHECK(img3->property("nveImageResizeMode").toInt() == static_cast<int>(nve::qt::ImageResizeMode::Center));

  // Verify QScrollArea vertical QScrollBar::valueChanged dispatches zero-copy EventKind::Scroll!
  NVE_CHECK(scroll5->widget() != nullptr && scroll5->widget()->height() >= 200);
  NVE_CHECK(scroll5->verticalScrollBar() != nullptr);
  scroll5->verticalScrollBar()->setValue(45);
  QCoreApplication::processEvents();
  NVE_CHECK(mock->scroll_events >= 1);
  NVE_CHECK(mock->last_scroll_node == 5u);
  NVE_CHECK(approx_eq(mock->last_scroll_y, 45.0, 2.0));
  NVE_CHECK(mock->last_scroll_content_h >= 200.0f);

  // Verify O(changed) IncrementalSceneReconciler updates ONLY node 7 (not all 7 nodes)!
  const uint64_t updates_before = widget.reconcileUpdateCount();
  widget.simulatePressOnNodeId(7, 4.0f, 4.0f);
  NVE_CHECK(counter7->text() == QStringLiteral("Counter:1"));
  const uint64_t updates_delta = widget.reconcileUpdateCount() - updates_before;
  NVE_CHECK(updates_delta == 1u);

  QFile::remove(tmp_png);
}

struct MockQtCanvasApp {
  static inline MockQtCanvasApp* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string draw_list_attr;
  bool emit_raw_corrupt_text{false};
  std::string raw_corrupt_bytes;
  int sibling_tick{0};
  BinaryCmdHelper cmd;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQtCanvasApp();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }

  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockQtCanvasApp*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockQtCanvasApp*>(h);
    self->cmd.begin(self->surface_id, frame_id);

    self->cmd.create_node(2, NVE_COMP_VIEW);
    self->cmd.create_node(3, NVE_COMP_SKIA_CANVAS);
    self->cmd.create_node(4, NVE_COMP_TEXT);

    self->cmd.set_children(NVE_ROOT_NODE_ID, {2});
    self->cmd.set_children(2, {3, 4});

    self->cmd.set_layout(2, 0, 1, 240.0f, 1, 140.0f, 0.0f, 8.0f, 6.0f);
    self->cmd.set_style(2, 0xFF0F172Au, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    self->cmd.set_layout(3, 0, 1, 200.0f, 1, 80.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);
    if (self->emit_raw_corrupt_text) {
      self->cmd.set_raw_prop_string(3, "drawList", "");
      self->cmd.set_text(3, self->raw_corrupt_bytes);
    } else {
      self->cmd.set_raw_prop_string(3, "drawList", self->draw_list_attr);
    }

    self->cmd.set_layout(4, 0, 1, 120.0f, 1, 24.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(4, "Tick:" + std::to_string(self->sibling_tick));
    self->cmd.set_style(4, 0x00000000u, 0xFFFFFFFFu, 0u, 0.0f, 0.0f, 1.0f, 14.0f);

    return self->cmd.finish();
  }

  static uint32_t dispatch_events(NveAppHandle, NveBufferSlice) { return 0; }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events, nullptr};
    return (iface == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
  }
};

void test_draw_list_canvas_widget_and_corrupt_survival() {
  // 1. Direct renderDrawListToPainter verification: all 8 opcodes + Studio sparkline + clip balance.
  const std::vector<float> samples{12.0f, 24.0f, 18.0f, 42.0f, 35.0f, 64.0f};
  const nve::DrawColorRgba bg_col = nve::DrawColorRgba::rgb(15u, 23u, 42u);
  const nve::DrawColorRgba stroke_col = nve::DrawColorRgba::rgb(56u, 189u, 248u);
  nve::DrawListEncoder spark_enc;
  spark_enc.sparkline(samples, 200.0f, 80.0f, stroke_col, bg_col);
  const std::vector<uint8_t> spark_bytes = spark_enc.finish();

  {
    QImage img(200, 80, QImage::Format_RGBA8888_Premultiplied);
    img.fill(Qt::transparent);
    QPainter p(&img);
    const auto res = nve::qt::renderDrawListToPainter(p, QRectF(0.0, 0.0, 200.0, 80.0), spark_bytes);
    NVE_CHECK(res.ok());
    NVE_CHECK(res.ops_decoded >= 8u);
    p.end();

    // Verify background rounded-rect interior color and cyan sparkline stroke/endpoint circle pixels.
    int bg_pixels = 0;
    int cyan_pixels = 0;
    for (int y = 0; y < img.height(); ++y) {
      for (int x = 0; x < img.width(); ++x) {
        const QColor c = img.pixelColor(x, y);
        if (std::abs(c.red() - 15) <= 2 && std::abs(c.green() - 23) <= 2 &&
            std::abs(c.blue() - 42) <= 2 && c.alpha() == 255) {
          ++bg_pixels;
        }
        if (c.blue() > 210 && c.green() > 150 && c.red() < 90 && c.alpha() == 255) {
          ++cyan_pixels;
        }
      }
    }
    NVE_CHECK(bg_pixels > 5000);
    NVE_CHECK(cyan_pixels > 40);
  }

  // 2. Unbalanced clip stream (2 ClipRects, 0 RestoreClips) + extra RestoreClip must leave QPainter balanced.
  {
    nve::DrawListEncoder unbal;
    unbal.clear(nve::DrawColorRgba::rgb(20u, 20u, 20u))
        .clip_rect(10.0f, 10.0f, 30.0f, 30.0f)
        .clip_rect(15.0f, 15.0f, 20.0f, 20.0f)
        .fill_rect(0.0f, 0.0f, 64.0f, 64.0f, nve::DrawColorRgba::rgb(240u, 30u, 50u));
    const std::vector<uint8_t> unbal_bytes = unbal.finish();

    QImage img(64, 64, QImage::Format_RGBA8888_Premultiplied);
    img.fill(Qt::transparent);
    QPainter p(&img);
    const auto res = nve::qt::renderDrawListToPainter(p, QRectF(0.0, 0.0, 64.0, 64.0), unbal_bytes);
    NVE_CHECK(res.ok());
    // Outer QPainter must NOT remain clipped after unbalanced stream returns!
    NVE_CHECK(!p.hasClipping());
    // Inside nested clip (20, 20) -> red; outside clip (4, 4) -> clear color (20, 20, 20).
    const QColor inside = img.pixelColor(20, 20);
    const QColor outside = img.pixelColor(4, 4);
    NVE_CHECK(inside.red() > 220 && inside.green() < 50);
    NVE_CHECK(std::abs(outside.red() - 20) <= 2 && std::abs(outside.green() - 20) <= 2);

    // Extra RestoreClip with empty clip stack must also survive cleanly.
    nve::DrawListEncoder extra_pop;
    extra_pop.restore_clip().restore_clip().fill_circle(32.0f, 32.0f, 8.0f, nve::DrawColorRgba::rgb(10u, 220u, 90u));
    const auto pop_res = nve::qt::renderDrawListToPainter(p, QRectF(0.0, 0.0, 64.0, 64.0), extra_pop.finish());
    NVE_CHECK(pop_res.ok());
    NVE_CHECK(!p.hasClipping());
    p.end();
    const QColor center = img.pixelColor(32, 32);
    NVE_CHECK(center.green() > 200 && center.red() < 40);
  }

  // 3. Corrupt buffers (bad magic, truncated, unknown opcode, trailing bytes) must fail cleanly without painting.
  {
    QImage img(64, 64, QImage::Format_RGBA8888_Premultiplied);
    img.fill(QColor(11, 22, 33, 255));
    QPainter p(&img);

    const std::vector<uint8_t> bad_magic{'B', 'A', 'D', '_', 'M', 'A', 'G', 'I', 'C', 1, 0, 0, 0, 0, 0};
    NVE_CHECK(nve::qt::renderDrawListToPainter(p, QRectF(0, 0, 64, 64), bad_magic).error ==
              nve::DrawListError::BadMagic);

    std::vector<uint8_t> truncated = spark_bytes;
    truncated.resize(spark_bytes.size() / 2u);
    NVE_CHECK(!nve::qt::renderDrawListToPainter(p, QRectF(0, 0, 64, 64), truncated).ok());

    std::vector<uint8_t> trailing = spark_bytes;
    trailing.push_back(0xAAu);
    NVE_CHECK(nve::qt::renderDrawListToPainter(p, QRectF(0, 0, 64, 64), trailing).error ==
              nve::DrawListError::TrailingBytes);
    p.end();

    // Image must remain untouched!
    const QColor untouched = img.pixelColor(32, 32);
    NVE_CHECK(untouched == QColor(11, 22, 33, 255));
  }

  // 4. End-to-end NveQtWidget -> NveQtCanvasWidget caching, byte-equality skip, grab() pixels & corrupt survival.
  nve::qt::NveQtWidget widget;
  widget.resize(240, 140);
  NVE_CHECK(widget.attachInterface(&MockQtCanvasApp::get_interface, 41));
  NVE_CHECK(MockQtCanvasApp::last_instance != nullptr);
  auto* mock = MockQtCanvasApp::last_instance;

  auto* canvas = qobject_cast<nve::qt::NveQtCanvasWidget*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(canvas != nullptr);
  NVE_CHECK(canvas->drawListBytes().empty());

  mock->draw_list_attr = nve::encode_draw_list_attr_base64(spark_bytes);
  NVE_CHECK(widget.renderFrame() == 0);
  NVE_CHECK(canvas->drawListBytes().size() == spark_bytes.size());
  NVE_CHECK(std::memcmp(canvas->drawListBytes().data(), spark_bytes.data(), spark_bytes.size()) == 0);
  const uint64_t redraws_after_first = canvas->redrawRequestCount();
  NVE_CHECK(redraws_after_first == 1u);

  // Re-rendering with identical bytes + sibling text mutation must NOT trigger canvas redraw!
  mock->sibling_tick = 1;
  NVE_CHECK(widget.renderFrame() == 0);
  NVE_CHECK(canvas->redrawRequestCount() == redraws_after_first);

  widget.show();
  QCoreApplication::processEvents();
  const QImage grabbed = canvas->grab().toImage();
  NVE_CHECK(!grabbed.isNull() && grabbed.width() == 200 && grabbed.height() == 80);
  int grabbed_bg = 0;
  int grabbed_cyan = 0;
  for (int y = 0; y < grabbed.height(); ++y) {
    for (int x = 0; x < grabbed.width(); ++x) {
      const QColor c = grabbed.pixelColor(x, y);
      if (std::abs(c.red() - 15) <= 2 && std::abs(c.green() - 23) <= 2 &&
          std::abs(c.blue() - 42) <= 2) {
        ++grabbed_bg;
      }
      if (c.blue() > 210 && c.green() > 150 && c.red() < 90) {
        ++grabbed_cyan;
      }
    }
  }
  NVE_CHECK(grabbed_bg > 5000);
  NVE_CHECK(grabbed_cyan > 40);

  // Updating drawList bytes triggers exactly 1 new redraw and paints the new accent color!
  nve::DrawListEncoder violet_enc;
  violet_enc.sparkline(samples, 200.0f, 80.0f, nve::DrawColorRgba::rgb(236u, 72u, 153u), bg_col);
  mock->draw_list_attr = violet_enc.to_attr_string();
  NVE_CHECK(widget.renderFrame() == 0);
  NVE_CHECK(canvas->redrawRequestCount() == redraws_after_first + 1u);

  // Feeding raw corrupt bytes via text_slab must survive grab() cleanly without crash.
  mock->emit_raw_corrupt_text = true;
  mock->raw_corrupt_bytes.assign(reinterpret_cast<const char*>(spark_bytes.data()), 19u);
  NVE_CHECK(widget.renderFrame() == 0);
  const QImage corrupt_grab = canvas->grab().toImage();
  NVE_CHECK(!corrupt_grab.isNull() && corrupt_grab.width() == 200 && corrupt_grab.height() == 80);
}

void test_paired_thin_client_widgets_and_quick() {
  auto build_frame = [](uint64_t fid, const std::string& label_text) {
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
    enc.set_text(3u, label_text);
    enc.set_event_mask(4u, nve::kEventMaskPress);
    enc.commit(true);
    const NveBufferSlice s = enc.finish();
    return std::vector<uint8_t>(s.ptr, s.ptr + s.len);
  };

  auto run_server_once = [&](const std::string& token,
                             const std::vector<uint8_t>& f1,
                             const std::vector<uint8_t>& f2,
                             std::atomic<bool>& saw_press,
                             uint16_t& out_port) -> std::thread {
    int lfd = ::socket(AF_INET, SOCK_STREAM, 0);
    NVE_CHECK(lfd >= 0);
    int one = 1;
    (void)::setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = 0;
    NVE_CHECK(::bind(lfd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
    socklen_t alen = sizeof(addr);
    NVE_CHECK(::getsockname(lfd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
    out_port = ntohs(addr.sin_port);
    NVE_CHECK(::listen(lfd, 2) == 0);

    return std::thread([lfd, token, f1, f2, &saw_press]() {
      auto read_exact = [](int fd, uint8_t* buf, size_t len) -> bool {
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
      auto read_msg = [&](int fd, std::vector<uint8_t>& out) -> bool {
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

      int cfd = ::accept(lfd, nullptr, nullptr);
      NVE_CHECK(cfd >= 0);
      std::vector<uint8_t> msg;
      NVE_CHECK(read_msg(cfd, msg));
      nve::TransportEnvelopeView env{};
      NVE_CHECK(nve::pair_wire::parse_envelope(msg, env) &&
                env.kind == nve::TransportMsgKind::Hello);
      nve::HelloPayload hp{};
      NVE_CHECK(nve::pair_wire::decode_hello_payload(env.payload, hp));
      const uint64_t sn = 0x1020304050607080ULL;
      const uint32_t sid = nve::pair_wire::derive_session_id(token, 1u, hp.client_nonce, sn);
      nve::HelloAckPayload ack{};
      ack.schema_version = 1u;
      ack.server_max_window = 64u;
      ack.server_nonce = sn;
      const auto ack_b = nve::pair_wire::encode_hello_ack_payload(ack);
      NVE_CHECK(write_msg(
          cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::HelloAck, sid, ack_b)));

      const auto kf1 = nve::pair_wire::encode_nvd_packet(
          nve::DeltaPacketKind::Keyframe, 0u, 1u, 0u, 0u, f1);
      NVE_CHECK(write_msg(
          cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, kf1)));

      const auto d2 = nve::pair_wire::encode_command_op_delta(f1, f2);
      NVE_CHECK(d2.has_value());
      const auto p2 = nve::pair_wire::encode_nvd_packet(
          nve::DeltaPacketKind::CommandOpDelta, 0u, 2u, 1u, 0u, *d2);
      NVE_CHECK(write_msg(
          cfd, nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, p2)));

      while (read_msg(cfd, msg)) {
        nve::TransportEnvelopeView in{};
        if (nve::pair_wire::parse_envelope(msg, in) && in.session_id == sid &&
            in.kind == nve::TransportMsgKind::InputEvents) {
          std::vector<nve::DecodedEvent> dec;
          if (nve::decode_event_ring(NveBufferSlice{in.payload.data(), in.payload.size()}, dec) &&
              dec.size() == 1u && dec[0].kind == nve::EventKind::Press && dec[0].node_id == 4u) {
            saw_press.store(true);
            break;
          }
        }
      }
      ::close(cfd);
      ::close(lfd);
    });
  };

  // 1. NveQtWidget paired thin-client roundtrip (zero dylib, QMetaObject::invokeMethod wake).
  {
    const auto f1 = build_frame(1u, "Qt Paired V1");
    const auto f2 = build_frame(2u, "Qt Paired V2");
    std::atomic<bool> saw_press{false};
    uint16_t port = 0u;
    std::thread srv = run_server_once("qt-pair-token", f1, f2, saw_press, port);

    nve::qt::NveQtWidget widget;
    widget.resize(320, 200);
    QString err;
    NVE_CHECK(widget.connectPaired(
        QStringLiteral("127.0.0.1:%1").arg(port),
        QStringLiteral("qt-pair-token"),
        QStringLiteral("tcp"),
        1u,
        &err));
    NVE_CHECK(widget.isPaired() && widget.isPairedConnected() && widget.isAppLoaded());

    const auto dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    while (std::chrono::steady_clock::now() < dl) {
      QCoreApplication::processEvents();
      auto* lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3u));
      if (lbl != nullptr && lbl->text() == QStringLiteral("Qt Paired V2")) {
        break;
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
    auto* lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3u));
    NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Qt Paired V2"));

    NVE_CHECK(widget.simulatePressOnNodeId(4u, 9.0f, 11.0f) == 1u);
    const auto ev_dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    while (!saw_press.load() && std::chrono::steady_clock::now() < ev_dl) {
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
    NVE_CHECK(saw_press.load());
    widget.disconnectPaired();
    NVE_CHECK(!widget.isPaired());
    srv.join();
  }

#ifdef NVE_QT_HAS_QUICK
  // 2. NveQtQuickItem paired thin-client roundtrip.
  {
    const auto f1 = build_frame(1u, "Quick Paired V1");
    const auto f2 = build_frame(2u, "Quick Paired V2");
    std::atomic<bool> saw_press{false};
    uint16_t port = 0u;
    std::thread srv = run_server_once("quick-pair-token", f1, f2, saw_press, port);

    nve::qt::NveQtQuickItem item;
    item.setSize(QSizeF(320.0, 200.0));
    QString err;
    NVE_CHECK(item.connectPaired(
        QStringLiteral("tcp://127.0.0.1:%1").arg(port),
        QStringLiteral("quick-pair-token"),
        QString(),
        1u,
        &err));
    NVE_CHECK(item.isPaired() && item.isPairedConnected() && item.isAppLoaded());

    const auto dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    while (std::chrono::steady_clock::now() < dl) {
      QCoreApplication::processEvents();
      if (item.sceneNodeText(3u) == "Quick Paired V2") {
        break;
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
    NVE_CHECK(item.sceneNodeText(3u) == "Quick Paired V2");

    NVE_CHECK(item.dispatchPress(4u, 7.0f, 8.0f) == 1u);
    const auto ev_dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    while (!saw_press.load() && std::chrono::steady_clock::now() < ev_dl) {
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
    NVE_CHECK(saw_press.load());
    item.disconnectPaired();
    NVE_CHECK(!item.isPaired());
    srv.join();
  }
#endif
}

} // namespace

void nve_test_qt_window_manager();

int main(int argc, char* argv[]) {
  QApplication app(argc, argv);

  test_error_codes_and_stateful_fixture();
  test_attach_props_events_morph_and_measurer();
  test_real_counter_dylib();
  test_frame0_mmap_snapshot_roundtrip();
  test_accessibility_and_ime_text_input();
  test_image_scroll_and_incremental_reconciliation();
  test_draw_list_canvas_widget_and_corrupt_survival();
  test_paired_thin_client_widgets_and_quick();
  nve_test_qt_window_manager();

  std::cout << "nve_qt_shell_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+Qt6" : "Standalone+Qt6") << ")\n";
  return 0;
}
