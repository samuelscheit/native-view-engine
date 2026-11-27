#include "NveQtWidget.hpp"
#include "qt_test_helpers.hpp"

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
#include <cstdlib>
#include <iostream>
#include <string>
#include <vector>

namespace {

using nve::qt_test::approx_eq;
using nve::qt_test::BinaryCmdHelper;
using nve::qt_test::DirectOutboundQueue;

struct MockQtRustComponent {
  uint32_t surface_id{1};
  int count{0};
  std::string title{"Initial"};
  bool morph_text_to_wrapper{false};
  BinaryCmdHelper cmd;
  DirectOutboundQueue outbound;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockQtRustComponent();
    self->surface_id = sid;
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

    self->cmd.set_layout(2, 1, 2, 100.0f, 2, 100.0f, 0.0f, 20.0f, 10.0f);
    self->cmd.set_style(2, 0xFF202020u, 0xFFFFFFFFu, 0xFF404040u, 1.0f, 8.0f, 1.0f, 16.0f);

    self->cmd.set_layout(3, 0, 1, 120.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(3, self->title + ":" + std::to_string(self->count));
    self->cmd.set_style(3, 0x00000000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 18.0f);

    self->cmd.set_layout(4, 0, 0, 0.0f, 1, 40.0f, 1.0f, 0.0f, 0.0f);
    self->cmd.set_style(4, 0xFFFF5500u, 0xFFFFFFFFu, 0xFFFFFFFFu, 1.5f, 12.0f, 0.95f, 16.0f);

    self->cmd.set_layout(5, 0, 1, 80.0f, 0, 0.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_style(5, 0x00000000u, 0xFF38BDF8u, 0, 0.0f, 0.0f, 1.0f, 14.0f);

    self->cmd.set_layout(6, 0, 1, 80.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_layout(7, 0, 1, 70.0f, 1, 30.0f, 0.0f, 0.0f, 0.0f);
    self->cmd.set_text(7, "hello");
    self->cmd.set_style(7, 0xFFFFFFFFu, 0xFF000000u, 0, 0.0f, 0.0f, 1.0f, 14.0f);

    return self->cmd.finish();
  }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    if (batch.ptr == nullptr) return 0u;
    for (uint32_t i = 0u; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 4u) {
        ++self->count;
        self->outbound.push_custom(
            self->surface_id,
            4u,
            ev.timestamp_us,
            "onIncrement",
            "{\"count\":" + std::to_string(self->count) + "}");
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "title" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        self->title.assign(v.data(), v.size());
      } else if (k == "morph" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->morph_text_to_wrapper = entries[i].value.data.bool_val != 0u;
      }
    }
    self->outbound.push_custom(
        self->surface_id, NVE_ROOT_NODE_ID, 777u, "onPropsApplied", "{\"ok\":true}");
    return 1u;
  }

  static NveEventBatch drain_emitted_events(NveAppHandle h) {
    return static_cast<MockQtRustComponent*>(h)->outbound.drain();
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch) {
    auto* self = static_cast<MockQtRustComponent*>(h);
    self->title.assign(reinterpret_cast<const char*>(patch.ptr), patch.len);
    return 1u;
  }

  static NveBufferSlice snapshot_state(NveAppHandle) { return {nullptr, 0}; }
  static void restore_state(NveAppHandle, NveBufferSlice) {}

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

  NVE_CHECK(!widget.loadDylib(QString(), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!widget.loadDylib(QStringLiteral("/nonexistent/libnve_missing.dylib"), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 1 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!widget.loadDylib(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 2 && !err_msg.isEmpty());

  err_code = 0;
  err_msg.clear();
  NVE_CHECK(!widget.loadDylib(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), 1, &err_code, &err_msg));
  NVE_CHECK(err_code == 3 && !err_msg.isEmpty());
  NVE_CHECK(!widget.isAppLoaded());

  err_code = -1;
  err_msg = QStringLiteral("stale");
  NVE_CHECK(widget.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 7, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  NVE_CHECK(widget.isAppLoaded());
  NVE_CHECK(widget.surfaceId() == 7);
  NVE_CHECK(widget.nodeCount() == 3);

  auto* lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:0"));

  widget.simulatePressOnNodeId(2, 5.0f, 5.0f);
  widget.simulatePressOnNodeId(2, 5.0f, 5.0f);
  lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:2"));

  NVE_CHECK(!widget.reloadDylib(QStringLiteral("/nonexistent/fail.dylib"), &err_code, &err_msg));
  NVE_CHECK(err_code == 1);
  NVE_CHECK(qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3))->text() == QStringLiteral("Fixture:2"));

  NVE_CHECK(!widget.reloadDylib(QStringLiteral(NVE_TEST_NO_ENTRY_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 2);
  NVE_CHECK(qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3))->text() == QStringLiteral("Fixture:2"));

  NVE_CHECK(!widget.reloadDylib(QStringLiteral(NVE_TEST_NO_SURFACE_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 3);
  NVE_CHECK(qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3))->text() == QStringLiteral("Fixture:2"));

  NVE_CHECK(widget.reloadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:2"));

  NVE_CHECK(widget.loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 7, &err_code, &err_msg));
  lbl = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(lbl != nullptr && lbl->text() == QStringLiteral("Fixture:0"));
}

void test_attach_props_events_morph_and_measurer() {
  nve::qt::NveQtWidget widget;
  widget.resize(520, 180);

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
    NVE_CHECK(approx_eq(container->width(), 520.0));
    NVE_CHECK(approx_eq(label3->x(), 20.0) && approx_eq(label3->width(), 120.0));
    NVE_CHECK(approx_eq(button4->x(), 150.0) && approx_eq(button4->width(), 170.0));
  }

  NVE_CHECK(widget.applyProps(nve::PropMap{{"title", std::string("Updated")}}));
  NVE_CHECK(label3->text() == QStringLiteral("Updated:0"));
  NVE_CHECK(received_events.size() == 1);
  NVE_CHECK(received_events[0].name == "onPropsApplied");

  widget.simulatePressOnNodeId(4, 10.0f, 10.0f);
  NVE_CHECK(label3->text() == QStringLiteral("Updated:1"));
  NVE_CHECK(received_events.size() == 2);
  NVE_CHECK(received_events[1].name == "onIncrement");

  send_left_click(button4, QPointF(10.0, 10.0));
  NVE_CHECK(label3->text() == QStringLiteral("Updated:2"));
  NVE_CHECK(received_events.size() == 3);

  NVE_CHECK(widget.applyProps(nve::PropMap{{"morph", true}}));
  QWidget* morph5_wrapper = widget.nativeWidgetForNodeId(5);
  NVE_CHECK(morph5_wrapper != nullptr && morph5_wrapper != morph5);
  NVE_CHECK(qobject_cast<QLabel*>(morph5_wrapper) == nullptr);
  auto* child8 = qobject_cast<QLabel*>(widget.nativeWidgetForNodeId(8));
  NVE_CHECK(child8 != nullptr && child8->parentWidget() == morph5_wrapper);
  NVE_CHECK(child8->text() == QStringLiteral("WrappedChild"));

  NVE_CHECK(widget.applyProps(nve::PropMap{{"morph", false}}));
  QWidget* morph5_leaf = widget.nativeWidgetForNodeId(5);
  auto* morph5_label = qobject_cast<QLabel*>(morph5_leaf);
  NVE_CHECK(morph5_label != nullptr);
  NVE_CHECK(morph5_label->text() == QStringLiteral("LeafText"));

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
      if (lbl->text() == text) return lbl;
    }
    return nullptr;
  };

  NVE_CHECK(collect_labels().size() >= 4);
  QLabel* title_lbl = find_label(QStringLiteral("NVE Kitchen Sink"));
  QLabel* count_lbl = find_label(QStringLiteral("Count: 0"));
  QLabel* inc_lbl = find_label(QStringLiteral("Increment +"));
  QLabel* reset_lbl = find_label(QStringLiteral("Reset"));
  NVE_CHECK(title_lbl != nullptr && count_lbl != nullptr && inc_lbl != nullptr && reset_lbl != nullptr);

  NVE_CHECK(count_lbl->font().pixelSize() == 28);
  for (QLabel* lbl : {title_lbl, count_lbl, inc_lbl, reset_lbl}) {
    NVE_CHECK(lbl->width() > 20 && lbl->height() >= 14);
    NVE_CHECK(lbl->palette().color(QPalette::WindowText).alpha() == 255);
    NVE_CHECK(lbl->testAttribute(Qt::WA_TransparentForMouseEvents));
  }

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
        if (btn->isAncestorOf(inc_lbl)) inc_pressable_id = snap.node_id;
        if (btn->isAncestorOf(reset_lbl)) reset_pressable_id = snap.node_id;
      }
    }
  }

  NVE_CHECK(top_card != nullptr && top_card->width() > 200 && top_card->height() > 60);
  NVE_CHECK(inc_pressable_id != 0 && reset_pressable_id != 0);

  QWidget* inc_btn = widget.nativeWidgetForNodeId(inc_pressable_id);
  QWidget* reset_btn = widget.nativeWidgetForNodeId(reset_pressable_id);
  NVE_CHECK(inc_btn != nullptr && reset_btn != nullptr);

  const int title_y = title_lbl->mapTo(&widget, QPoint(0, 0)).y();
  const int count_y = count_lbl->mapTo(&widget, QPoint(0, 0)).y();
  const int inc_btn_y = inc_btn->mapTo(&widget, QPoint(0, 0)).y();
  NVE_CHECK(title_y < count_y && count_y + count_lbl->height() < inc_btn_y);

  auto assert_centered_in_button = [](QLabel* lbl, QWidget* btn) {
    const QPoint top_left = lbl->mapTo(btn, QPoint(0, 0));
    const double lbl_cx = static_cast<double>(top_left.x()) + static_cast<double>(lbl->width()) * 0.5;
    const double lbl_cy = static_cast<double>(top_left.y()) + static_cast<double>(lbl->height()) * 0.5;
    NVE_CHECK(approx_eq(lbl_cx, static_cast<double>(btn->width()) * 0.5, 3.0));
    NVE_CHECK(approx_eq(lbl_cy, static_cast<double>(btn->height()) * 0.5, 3.0));
  };
  assert_centered_in_button(inc_lbl, inc_btn);
  assert_centered_in_button(reset_lbl, reset_btn);

  widget.simulatePressOnNodeId(inc_pressable_id, 20.0f, 20.0f);
  widget.simulatePressOnNodeId(inc_pressable_id, 20.0f, 20.0f);
  QCoreApplication::processEvents();
  NVE_CHECK(find_label(QStringLiteral("Count: 2")) != nullptr);

  const char* shot_path = std::getenv("NVE_TEST_SCREENSHOT_PATH");
  if (shot_path != nullptr && shot_path[0] != '\0') {
    QPixmap pix = widget.grab();
    NVE_CHECK(!pix.isNull() && pix.width() == 640 && pix.height() == 420);
    NVE_CHECK(pix.save(QString::fromUtf8(shot_path), "PNG"));
  }

  err_code = -1;
  err_msg.clear();
  NVE_CHECK(widget.reloadDylib(counter_path, &err_code, &err_msg));
  NVE_CHECK(err_code == 0 && err_msg.isEmpty());
  QCoreApplication::processEvents();
  NVE_CHECK(find_label(QStringLiteral("Count: 2")) != nullptr);

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

  widget.simulatePressOnNodeId(reset_after_reload, 20.0f, 20.0f);
  QCoreApplication::processEvents();
  NVE_CHECK(find_label(QStringLiteral("Count: 0")) != nullptr);
  const NveMapEntry reset_initial[1] = {
      NveMapEntry{nve::make_str_slice("initialCount"), nve::make_i64_value(0)},
  };
  NVE_CHECK(widget.applyProps(reset_initial, 1u));
}

} // namespace

void nve_test_qt_a11y_and_media();
void nve_test_qt_canvas_and_pair();
void nve_test_qt_window_manager();

int main(int argc, char* argv[]) {
  QApplication app(argc, argv);

  test_error_codes_and_stateful_fixture();
  test_attach_props_events_morph_and_measurer();
  test_real_counter_dylib();
  nve_test_qt_a11y_and_media();
  nve_test_qt_canvas_and_pair();
  nve_test_qt_window_manager();

  std::cout << "nve_qt_shell_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+Qt6" : "Standalone+Qt6") << ")\n";
  return 0;
}
