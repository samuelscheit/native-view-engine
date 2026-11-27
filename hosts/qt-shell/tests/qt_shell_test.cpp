// Core Qt Widgets host suite: the built-in counter app mirrored into retained widgets,
// `QFontMetrics` / `QTextLayout` measurement feeding layout (and a custom measurer), O(changed)
// reconciliation, mouse / keyboard / simulated press dispatch, outbound events, props, live
// resize, `cdylib` loading errors, state- and focus-preserving hot reload, app-state files, app
// switches, and queued wakes that never outlive the host.
#include "NveQtWidgetInternal.hpp"
#include "qt_test_helpers.hpp"

#include <nve/host/demo_counter_app.hpp>

#include <QKeyEvent>

#include <filesystem>
#include <memory>
#include <vector>

namespace {

using namespace nve::qt;
using namespace nve::qt::test;
using nve::host::demo::CounterApp;

NveQtNodeWidget* frame(NveQtWidget& w, uint32_t id) { return dynamic_cast<NveQtNodeWidget*>(w.nativeWidgetForNodeId(id)); }

std::string text_of(NveQtWidget& w, uint32_t id) {
  const NveQtNodeWidget* f = frame(w, id);
  return f != nullptr ? f->text().toStdString() : std::string("<none>");
}

void test_counter_widgets_measurement_and_events() {
  NveQtWidget widget(QtHostConfig{.surface_id = 7u, .width = 480.0f, .height = 360.0f});
  QtHost& host = widget.host();
  NVE_CHECK(!host.is_loaded() && widget.viewCount() == 0u && host.surface_id() == 7u);
  NVE_CHECK(widget.width() == 480 && widget.height() == 360);

  // QTextLayout measurement: monotonic in size and weight, wraps under a width constraint.
  const NveIntrinsicSize small = measure_text("Hello NVE", 16.0f, 400u, 0.0f);
  const NveIntrinsicSize big = measure_text("Hello NVE", 32.0f, 400u, 0.0f);
  const NveIntrinsicSize bold = measure_text("Hello NVE", 16.0f, 800u, 0.0f);
  const NveIntrinsicSize wrapped = measure_text("Hello NVE wrapping text", 16.0f, 400u, 40.0f);
  NVE_CHECK(small.width > 0.0f && small.height > 0.0f);
  NVE_CHECK(big.width > small.width * 1.5f && big.height > small.height);
  NVE_CHECK(bold.width >= small.width);
  NVE_CHECK(wrapped.width <= 41.0f && wrapped.height > small.height * 1.5f);
  NVE_CHECK(measure_text("", 16.0f, 400u, 0.0f).width == 0.0f);
  NVE_CHECK(measure_text("a\nb", 16.0f, 400u, 0.0f).height > small.height * 1.5f);  // hard breaks
  NVE_CHECK(measure_text("Hello", 16.0f, 400u, std::nanf("")).width == measure_text("Hello", 16.0f, 400u, 0.0f).width);

  std::vector<int64_t> counts;
  host.set_event_callback([&counts](std::span<const NveDirectEvent> evs) {
    for (const NveDirectEvent& ev : evs) {
      if (ev.kind == NVE_EVENT_CUSTOM && ev.value.tag == NVE_VAL_I64) counts.push_back(ev.value.data.i64_val);
    }
  });

  NVE_CHECK(host.attach_interface(&CounterApp::get_interface));
  NVE_CHECK(host.is_loaded() && widget.viewCount() == 5u);
  NveQtNodeWidget* column = frame(widget, CounterApp::kColumn);
  NveQtNodeWidget* title = frame(widget, CounterApp::kTitle);
  NveQtNodeWidget* inc = frame(widget, CounterApp::kIncrement);
  NveQtNodeWidget* count = frame(widget, CounterApp::kCount);
  NveQtNodeWidget* reset = frame(widget, CounterApp::kReset);
  NVE_CHECK(column && title && inc && count && reset);
  NVE_CHECK(column->parentWidget() == &widget);
  NVE_CHECK(title->parentWidget() == column && count->parentWidget() == column);
  NVE_CHECK(title->content() == NodeContent::Text && text_of(widget, CounterApp::kTitle) == "NVE native host");
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 0");
  NVE_CHECK(widget.nativeWidgetForNodeId(1u) == &widget && widget.nativeWidgetForNodeId(99u) == nullptr);
  NVE_CHECK(widget.nodeIdForNativeWidget(inc) == CounterApp::kIncrement && widget.nodeIdForNativeWidget(&widget) == 0u);

  // Children stacked in scene order (paint order: later siblings on top).
  std::vector<QWidget*> stack;
  for (QObject* o : column->children()) {
    if (auto* w = qobject_cast<QWidget*>(o); w != nullptr && w->property(kNodeIdProperty).isValid()) stack.push_back(w);
  }
  NVE_CHECK((stack == std::vector<QWidget*>{title, inc, count, reset}));

  // The runtime laid out text with this host's measurer: the title is exactly its measured size.
  const NveIntrinsicSize title_size = measure_text("NVE native host", 22.0f, 400u, 0.0f);
  NVE_CHECK(approx(title->width(), title_size.width, 1.0) && approx(title->height(), title_size.height, 1.0));
  NVE_CHECK(column->width() == 480 && column->height() > reset->y() + reset->height());
  NVE_CHECK(title->x() == 24 && title->y() == 24);  // column padding
  NVE_CHECK(count->x() == 24 && count->y() > title->y() + 40);

  // Visuals: column background; buttons: background, radius, white inherited text, padded content box.
  NVE_CHECK(column->visual().background == QColor(248, 250, 252));
  NVE_CHECK(inc->visual().background == QColor(37, 99, 235) && inc->visual().radius == 8.0);
  NVE_CHECK(inc->text_style().color == QColor(255, 255, 255) && inc->text_style().font.pixelSize() == 16);
  NVE_CHECK(inc->content_box() == QRectF(16.0, 0.0, inc->width() - 32.0, 40.0) && inc->height() == 40);
  NVE_CHECK(title->text_style().color == QColor(15, 23, 42) && title->text_style().font.pixelSize() == 22);
  // Painted pixels: the button's fill inside its rounded box, the column behind it.
  widget.show();
  const QImage img = render(&widget);
  const QPoint edge = inc->mapTo(&widget, QPoint(3, 20));
  const QPoint corner = inc->mapTo(&widget, QPoint(0, 0));
  NVE_CHECK(same_rgb(pixel(img, edge.x(), edge.y()), 37, 99, 235));
  NVE_CHECK(same_rgb(pixel(img, 470, column->height() - 5), 248, 250, 252));
  NVE_CHECK(same_rgb(pixel(img, corner.x(), corner.y()), 248, 250, 252, 40));  // rounded corner cut

  // Semantics: buttons are activatable + focusable, the title a heading.
  const auto inc_sem = widget.accessibilityInfoForNodeId(CounterApp::kIncrement);
  NVE_CHECK(inc_sem && inc_sem->label == "Increment" && inc_sem->role == QAccessible::Button);
  NVE_CHECK(inc_sem->activatable && inc_sem->focusable && !inc_sem->disabled);
  NVE_CHECK(widget.accessibilityInfoForNodeId(CounterApp::kTitle)->role == QAccessible::Heading);
  NVE_CHECK(inc->focusPolicy() == Qt::StrongFocus && title->focusPolicy() == Qt::NoFocus);
  NVE_CHECK(inc->cursor().shape() == Qt::PointingHandCursor);

  // Mouse click -> Press -> app -> new frame: only the changed label is reconciled.
  const uint64_t before = widget.reconcileUpdateCount();
  click(inc, QPointF(4.0, 4.0));
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 1");
  NVE_CHECK(frame(widget, CounterApp::kCount) == count);  // retained, not recreated
  const uint64_t delta = widget.reconcileUpdateCount() - before;
  NVE_CHECK(delta >= 1u && delta <= 3u);
  // A click on the (unsubscribed) title or released outside the button does nothing.
  click(title, QPointF(2.0, 2.0));
  {
    QMouseEvent press(QEvent::MouseButtonPress, QPointF(2, 2), inc->mapToGlobal(QPointF(2, 2)), Qt::LeftButton,
                      Qt::LeftButton, Qt::NoModifier);
    QMouseEvent release(QEvent::MouseButtonRelease, QPointF(-5, -5), inc->mapToGlobal(QPointF(-5, -5)),
                        Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QApplication::sendEvent(inc, &press);
    QApplication::sendEvent(inc, &release);
  }
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 1");
  // Keyboard activation (Space / Enter) of a focusable button.
  QKeyEvent space(QEvent::KeyPress, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
  QApplication::sendEvent(inc, &space);
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 2");
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(!host.simulate_press(CounterApp::kTitle));  // not subscribed: no change
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 3");
  NVE_CHECK((counts == std::vector<int64_t>{1, 2, 3}));
  NVE_CHECK(host.pump() == 0u);  // idle frame: nothing to reconcile

  // Component props.
  const std::string retitle = "Retitled";
  NveMapEntry prop{};
  prop.key = NveStrSlice{reinterpret_cast<const uint8_t*>("title"), 5u};
  prop.value.tag = NVE_VAL_STR;
  prop.value.data.str_val = NveStrSlice{reinterpret_cast<const uint8_t*>(retitle.data()), retitle.size()};
  NVE_CHECK(host.apply_props(std::span<const NveMapEntry>(&prop, 1u)));
  NVE_CHECK(text_of(widget, CounterApp::kTitle) == "Retitled");
  const uint8_t bogus_patch[] = {1, 2, 3};
  NVE_CHECK(!host.apply_hot_patch(bogus_patch));  // no templates: a no-op
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 3");

  // Live resize relayouts retained widgets in place.
  widget.resize(600, 300);
  NVE_CHECK(column->width() == 600 && frame(widget, CounterApp::kColumn) == column);
  NVE_CHECK(host.config().width == 600.0f && widget.sizeHint() == QSize(600, 300));
  widget.hide();
}

void test_custom_measurer_and_surface_id() {
  NveQtWidget widget(QtHostConfig{.width = 300.0f, .height = 200.0f});
  QtHost& host = widget.host();
  int calls = 0;
  host.set_text_measurer([&calls](std::string_view text, float, uint16_t, float) {
    ++calls;
    return NveIntrinsicSize{static_cast<float>(text.size()) * 10.0f, 17.0f};
  });
  ScriptedApp app;
  app.w.create(1u);
  text_node(app.w, 2u, "abcd");
  const uint32_t kids[] = {2u};
  app.w.set_children(1u, kids);
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  QWidget* t = widget.nativeWidgetForNodeId(2u);
  NVE_CHECK(calls > 0 && t->height() == 17);
  host.set_text_measurer({});  // back to QTextLayout: relayouts
  NVE_CHECK(approx(t->height(), measure_text("abcd", 16.0f, 400u, 0.0f).height, 1.0));

  // A new surface id recreates the session: the app and its views are dropped.
  host.set_surface_id(9u);
  NVE_CHECK(host.surface_id() == 9u && !host.is_loaded() && widget.viewCount() == 0u);
  // The old app instance is destroyed with its session; a new app mounts into the fresh replica.
  NVE_CHECK(app.destroys == 1u);
  ScriptedApp next;
  next.w.create(1u);
  text_node(next.w, 2u, "again");
  next.w.set_children(1u, kids);
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(next.creates == 1u && widget.viewCount() == 1u);
  NVE_CHECK(dynamic_cast<NveQtNodeWidget*>(widget.nativeWidgetForNodeId(2u))->text() == "again");
}

void test_modules_hot_reload_and_state() {
  NveQtWidget widget;
  QtHost& host = widget.host();
  nve::host::ModuleError err = nve::host::ModuleError::None;
  std::string msg;
  NVE_CHECK(!host.load_dylib("", &err, &msg) && err == nve::host::ModuleError::Open);
  NVE_CHECK(!host.load_dylib("/nonexistent/libnope.so", &err, &msg) && err == nve::host::ModuleError::Open);
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_ENTRY_MODULE, &err, &msg) && err == nve::host::ModuleError::MissingEntry);
  NVE_CHECK(!host.load_dylib(NVE_TEST_NO_SURFACE_MODULE, &err, &msg) &&
            err == nve::host::ModuleError::MissingSurface);
  NVE_CHECK(!host.is_loaded() && widget.viewCount() == 0u);

  NVE_CHECK(host.load_dylib(NVE_TEST_COUNTER_MODULE, &err, &msg));
  NVE_CHECK(host.is_loaded() && widget.viewCount() == 5u);
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 2");

  // Focus a button, then hot reload: state carries over (`NveDevApi`), views and focus survive.
  widget.show();
  widget.activateWindow();
  NveQtNodeWidget* inc = frame(widget, CounterApp::kIncrement);
  inc->setFocus(Qt::OtherFocusReason);
  spin();
  NVE_CHECK(inc->hasFocus());
  NVE_CHECK(!host.reload_dylib(NVE_TEST_NO_SURFACE_MODULE, &err, &msg));  // a failed reload keeps the app
  NVE_CHECK(host.simulate_press(CounterApp::kIncrement));
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 3");
  NVE_CHECK(host.reload_dylib(NVE_TEST_COUNTER_MODULE, &err, &msg));
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 3");
  NVE_CHECK(widget.viewCount() == 5u && frame(widget, CounterApp::kIncrement) == inc && inc->hasFocus());
  widget.hide();

  // App-state file: save, mutate, restore; wrong schema / missing file refuse.
  const std::string path = (std::filesystem::temp_directory_path() / "nve_qt_state_test.nvestate").string();
  NVE_CHECK(host.save_state(path, 4u));
  NVE_CHECK(host.simulate_press(CounterApp::kReset));
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 0");
  NVE_CHECK(!host.restore_state(path, 5u));
  NVE_CHECK(!host.restore_state(path + ".missing", 4u));
  NVE_CHECK(host.restore_state(path, 4u));
  NVE_CHECK(text_of(widget, CounterApp::kCount) == "Count: 3");
  std::filesystem::remove(path);

  // Switching apps: views of nodes the new app does not have are dropped; a reused id keeps its
  // view only when its native kind matches.
  ScriptedApp app;
  app.w.create(1u);
  text_node(app.w, 2u, "scripted");
  app.w.create(3u);
  app.w.set_bool(3u, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MULTILINE, false);
  (void)app.w.set_str(3u, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "v");
  const uint32_t kids[] = {2u, 3u};
  app.w.set_children(1u, kids);
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(widget.viewCount() == 2u && text_of(widget, 2u) == "scripted");
  NVE_CHECK(dynamic_cast<QLineEdit*>(widget.nativeWidgetForNodeId(3u)) != nullptr);  // title -> input: recreated
  NVE_CHECK(widget.nativeWidgetForNodeId(CounterApp::kCount) == nullptr);
  NVE_CHECK(widget.nativeWidgetForNodeId(2u)->parentWidget() == &widget);
  spin();  // deferred deletes of the dropped widgets
}

uint64_t frame_id(QtHost& host) {
  NveRuntimeMetrics m{};
  NVE_CHECK(host.session().runtime().metrics(&m) == NVE_RUNTIME_OK);
  return m.frame_id;
}

void test_queued_wakes_never_outlive_the_host() {
  // Wakes are queued to the UI thread (never run inline) and dropped once the host is gone.
  ScriptedApp app;
  app.w.create(1u);
  app.w.create(2u);
  app.w.attach(2u, NVE_TABLE_SCROLL_DESC);
  size_node(app.w, 2u, 100.0f, 50.0f);
  app.w.create(3u);
  size_node(app.w, 3u, 100.0f, 400.0f);
  const uint32_t inner[] = {3u};
  app.w.set_children(2u, inner);
  const uint32_t kids[] = {2u};
  app.w.set_children(1u, kids);
  {
    auto widget = std::make_unique<NveQtWidget>();
    NVE_CHECK(widget->host().attach_interface(&ScriptedApp::get_interface));
    // A runtime scroll wakes the host: the offset is published by the queued pump, not inline.
    const auto moved = widget->host().scroll_by(2u, 0.0f, 30.0f);
    NVE_CHECK(moved && approx(moved->y(), 30.0, 0.01));
    const uint64_t frames = frame_id(widget->host());
    spin();
    NVE_CHECK(frame_id(widget->host()) > frames);
    // Another wake queued, then the host dies before it runs: the queued call is dropped.
    NVE_CHECK(widget->host().scroll_by(2u, 0.0f, 10.0f).has_value());
    widget.reset();
    spin();  // ASan / UBSan: no use after free
  }
  NVE_CHECK(app.creates == 1u && app.destroys == 1u);
}

void test_live_kitchen_sink_if_configured() {
  const char* path = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (path == nullptr || *path == '\0') return;
  NveQtWidget widget(QtHostConfig{.surface_id = 1u, .width = 1280.0f, .height = 800.0f});
  QtHost& host = widget.host();
  nve::host::ModuleError err = nve::host::ModuleError::None;
  std::string msg;
  NVE_CHECK(host.load_dylib(path, &err, &msg) && err == nve::host::ModuleError::None);
  NVE_CHECK(host.is_loaded() && widget.viewCount() > 10u);

  // Text views: leaves painting text (a Dioxus `text { "Count: " {n} }` wrapper is a View whose
  // runs are child leaves, so no frame both paints text and holds node children).
  const auto labels = [&] {
    std::vector<NveQtNodeWidget*> out;
    const auto walk = [&](auto& self, QObject* o) -> void {
      if (auto* f = dynamic_cast<NveQtNodeWidget*>(o); f != nullptr && f->content() == NodeContent::Text) out.push_back(f);
      for (QObject* c : o->children()) self(self, c);
    };
    walk(walk, &widget);
    return out;
  };
  const auto has_node_children = [](const QWidget* w) {
    for (const QObject* c : w->children()) {
      if (c->property(kNodeIdProperty).isValid()) return true;
    }
    return false;
  };
  const auto find_label = [&](std::string_view text) -> NveQtNodeWidget* {
    for (NveQtNodeWidget* f : labels()) {
      if (f->text().toStdString() == text) return f;
    }
    return nullptr;
  };
  // The nearest ancestor node (or the label itself) with a press action.
  const auto pressable = [&](QWidget* w) -> uint32_t {
    for (; w != nullptr && w != &widget; w = w->parentWidget()) {
      const uint32_t id = widget.nodeIdForNativeWidget(w);
      if (id != 0u && widget.accessibilityInfoForNodeId(id)->activatable) return id;
    }
    return 0u;
  };

  NVE_CHECK(find_label("NVE Kitchen Sink") != nullptr && find_label("Count: 0") != nullptr);
  size_t measured = 0u;
  for (NveQtNodeWidget* l : labels()) {
    NVE_CHECK(!has_node_children(l));  // leaves only
    if (!l->isVisibleTo(&widget) || l->text().isEmpty()) continue;
    NVE_CHECK(l->width() > 0 && l->height() >= 10);  // measured layout
    ++measured;
  }
  NVE_CHECK(measured > 10u);
  const uint32_t inc = pressable(find_label("Increment +"));
  const uint32_t reset = pressable(find_label("Reset"));
  NVE_CHECK(inc != 0u && reset != 0u && inc != reset);
  click(widget.nativeWidgetForNodeId(inc), QPointF(3.0, 3.0));  // a real click on the pressable node
  NVE_CHECK(find_label("Count: 1") != nullptr);
  NVE_CHECK(host.simulate_press(inc) && find_label("Count: 2") != nullptr);
  NVE_CHECK(host.simulate_press(reset) && find_label("Count: 0") != nullptr);
  for (int i = 0; i < 3; ++i) NVE_CHECK(host.simulate_press(inc));
  NVE_CHECK(find_label("Count: 3") != nullptr);

  const size_t views = widget.viewCount();
  NveQtNodeWidget* count = find_label("Count: 3");
  NVE_CHECK(host.reload_dylib(path, &err, &msg));
  NVE_CHECK(find_label("Count: 3") == count && widget.viewCount() == views);  // state + views kept
  spin();
}

}  // namespace

void nve_test_qt_input_and_scroll();
void nve_test_qt_a11y_and_media();
void nve_test_qt_canvas_and_pair();
void nve_test_qt_window_manager();

int main(int argc, char** argv) {
  QApplication qapp(argc, argv);
  test_counter_widgets_measurement_and_events();
  test_custom_measurer_and_surface_id();
  test_modules_hot_reload_and_state();
  test_queued_wakes_never_outlive_the_host();
  test_live_kitchen_sink_if_configured();
  nve_test_qt_input_and_scroll();
  nve_test_qt_a11y_and_media();
  nve_test_qt_canvas_and_pair();
  nve_test_qt_window_manager();
  std::cout << "nve_qt_shell_test: ok\n";
  return 0;
}
