#include "NveQtWidget.hpp"

#include "mock_jsi_recorder.hpp"

#include <QApplication>
#include <QColor>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFont>
#include <QLabel>
#include <QLineEdit>
#include <QMouseEvent>
#include <QPalette>
#include <QPixmap>
#include <QScrollArea>

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
      float gap) {
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
    push<float>(1.0f);
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

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch_json) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch_json.ptr), patch_json.len);
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
  const char* dylib_env = std::getenv("NVE_TEST_COUNTER_DYLIB");
  if (dylib_env == nullptr || dylib_env[0] == '\0') {
    return;
  }

  nve::qt::NveQtWidget widget;
  widget.resize(640, 420);

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

  QLabel* title_lbl = find_label(QStringLiteral("NATIVE VIEW ENGINE"));
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

  // Locate top card ancestor and button pressable ancestors from snapshots.
  const auto snapshots = widget.session().snapshot_nodes();
  uint32_t inc_pressable_id = 0;
  uint32_t reset_pressable_id = 0;
  QWidget* top_card = nullptr;
  for (const auto& snap : snapshots) {
    if (snap.kind == NVE_COMP_VIEW && snap.border_radius >= 15.0f && snap.layout.width > 200.0f &&
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
  // Reset CounterModule's process-global INITIAL_COUNT to 0 so cold load proves restore_state works!
  NVE_CHECK(widget.applyPropsJson(R"({"initialCount":0})"));

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
      NVE_CHECK(studio_writer.applyPropsJson(R"({"accent":"violet","view":"metrics"})"));
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

} // namespace

int main(int argc, char* argv[]) {
  QApplication app(argc, argv);

  test_error_codes_and_stateful_fixture();
  test_attach_props_events_morph_and_measurer();
  test_real_counter_dylib();
  test_frame0_mmap_snapshot_roundtrip();

  std::cout << "nve_qt_shell_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+Qt6" : "Standalone+Qt6") << ")\n";
  return 0;
}
