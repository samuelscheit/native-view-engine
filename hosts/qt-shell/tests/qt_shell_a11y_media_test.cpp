#include "NveQtWidget.hpp"
#include "qt_test_helpers.hpp"

#include <QAccessible>
#include <QBuffer>
#include <QColor>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QImage>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QMouseEvent>
#include <QScrollArea>
#include <QScrollBar>
#include <QUrl>

#include <cstdlib>
#include <iostream>
#include <string>

namespace {

using nve::qt_test::approx_eq;
using nve::qt_test::BinaryCmdHelper;

void send_left_click(QWidget* target, const QPointF& local_pos) {
  const QPointF global_pos = target->mapToGlobal(local_pos);
  QMouseEvent press(
      QEvent::MouseButtonPress, local_pos, global_pos, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
  QMouseEvent release(
      QEvent::MouseButtonRelease, local_pos, global_pos, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
  QCoreApplication::sendEvent(target, &press);
  QCoreApplication::sendEvent(target, &release);
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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockA11yImeQtApp*>(h);
    if (batch.ptr == nullptr) return 0u;
    for (uint32_t i = 0u; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 2u) {
        ++self->press_calls;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::TextChange) && ev.node_id == 5u) {
        ++self->text_change_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_text.assign(sv.data(), sv.size());
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Custom) && ev.node_id == 5u &&
                 nve::str_slice_view(ev.name) == "onSubmit") {
        ++self->submit_calls;
        const auto sv = nve::str_slice_view(ev.text);
        self->last_submit.assign(sv.data(), sv.size());
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockA11yImeQtApp*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "disabled" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->disabled_btn = entries[i].value.data.bool_val != 0u;
      } else if (k == "secure" && entries[i].value.tag == NVE_VAL_BOOL) {
        self->secure_input = entries[i].value.data.bool_val != 0u;
      } else if (k == "serverText" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        self->input_text.assign(v.data(), v.size());
      }
    }
    return 1u;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    static const NveComponentApi kComp{&set_props, nullptr};
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

  NVE_CHECK(edit->testAttribute(Qt::WA_InputMethodEnabled));
  NVE_CHECK(edit->placeholderText() == QStringLiteral("Type CJK or accented text..."));
  NVE_CHECK(edit->accessibleName() == QStringLiteral("Search Query"));
  NVE_CHECK(edit->objectName() == QStringLiteral("qt-search-input"));
  NVE_CHECK(edit->echoMode() == QLineEdit::Normal);
  NVE_CHECK(widget.accessibleRoleForNodeId(5) == QAccessible::EditableText);

  {
    QList<QInputMethodEvent::Attribute> attrs;
    QInputMethodEvent preedit_ev(QStringLiteral("にほん"), attrs);
    QCoreApplication::sendEvent(edit, &preedit_ev);
    NVE_CHECK(mock->text_change_calls == 0);

    NVE_CHECK(widget.applyProps(nve::PropMap{{"serverText", std::string("stale")}}));
    NVE_CHECK(edit->text().isEmpty());
  }

  {
    QInputMethodEvent commit_ev;
    commit_ev.setCommitString(QStringLiteral("日本語 café 🚀"));
    QCoreApplication::sendEvent(edit, &commit_ev);
    NVE_CHECK(edit->text() == QStringLiteral("日本語 café 🚀"));
    NVE_CHECK(mock->text_change_calls == 1);
    NVE_CHECK(mock->last_text == "日本語 café 🚀");
  }

  {
    QKeyEvent ret_ev(QEvent::KeyPress, Qt::Key_Return, Qt::NoModifier, QStringLiteral("\r"));
    QCoreApplication::sendEvent(edit, &ret_ev);
    NVE_CHECK(mock->submit_calls == 1);
    NVE_CHECK(mock->last_submit == "日本語 café 🚀");
  }

  NVE_CHECK(widget.applyProps(nve::PropMap{{"disabled", true}, {"secure", true}}));
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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQtImageScrollApp*>(h);
    if (batch.ptr == nullptr) return 0u;
    for (uint32_t i = 0u; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Scroll)) {
        ++self->scroll_events;
        self->last_scroll_node = ev.node_id;
        self->last_scroll_y = ev.y;
        self->last_scroll_content_h = ev.height;
      } else if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 7u) {
        ++self->counter;
        return 1u;
      }
    }
    return 0u;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockQtImageScrollApp*>(h);
    for (uint32_t i = 0u; i < count; ++i) {
      const auto k = nve::str_slice_view(entries[i].key);
      if (k == "resizeMode" && entries[i].value.tag == NVE_VAL_STR) {
        const auto v = nve::str_slice_view(entries[i].value.data.str_val);
        self->resize_mode.assign(v.data(), v.size());
      } else if (k == "bump" && entries[i].value.tag == NVE_VAL_BOOL &&
                 entries[i].value.data.bool_val != 0u) {
        ++self->counter;
      }
    }
    return 1u;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    static const NveComponentApi kComp{&set_props, nullptr};
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

  widget.show();
  QCoreApplication::processEvents();
  const QImage rendered = img3->grab().toImage();
  NVE_CHECK(!rendered.isNull() && rendered.width() == 48 && rendered.height() == 48);
  const QColor center_px = rendered.pixelColor(24, 24);
  NVE_CHECK(center_px.red() > 200 && center_px.green() < 60 && center_px.blue() < 70);

  NVE_CHECK(widget.applyProps(nve::PropMap{{"resizeMode", std::string("contain")}}));
  NVE_CHECK(img3->property("nveImageResizeMode").toInt() == static_cast<int>(nve::qt::ImageResizeMode::Contain));
  NVE_CHECK(widget.applyProps(nve::PropMap{{"resizeMode", std::string("stretch")}}));
  NVE_CHECK(img3->property("nveImageResizeMode").toInt() == static_cast<int>(nve::qt::ImageResizeMode::Stretch));
  NVE_CHECK(widget.applyProps(nve::PropMap{{"resizeMode", std::string("center")}}));
  NVE_CHECK(img3->property("nveImageResizeMode").toInt() == static_cast<int>(nve::qt::ImageResizeMode::Center));

  NVE_CHECK(scroll5->widget() != nullptr && scroll5->widget()->height() >= 200);
  NVE_CHECK(scroll5->verticalScrollBar() != nullptr);
  scroll5->verticalScrollBar()->setValue(45);
  QCoreApplication::processEvents();
  NVE_CHECK(mock->scroll_events >= 1);
  NVE_CHECK(mock->last_scroll_node == 5u);
  NVE_CHECK(approx_eq(mock->last_scroll_y, 45.0, 2.0));
  NVE_CHECK(mock->last_scroll_content_h >= 200.0f);

  const uint64_t updates_before = widget.reconcileUpdateCount();
  widget.simulatePressOnNodeId(7, 4.0f, 4.0f);
  NVE_CHECK(counter7->text() == QStringLiteral("Counter:1"));
  NVE_CHECK(widget.reconcileUpdateCount() - updates_before == 1u);

  QFile::remove(tmp_png);
}

} // namespace

void nve_test_qt_a11y_and_media() {
  test_accessibility_and_ime_text_input();
  test_image_scroll_and_incremental_reconciliation();
}
