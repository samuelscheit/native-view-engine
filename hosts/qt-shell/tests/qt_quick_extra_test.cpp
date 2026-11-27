#include "NveQtQuickItem.hpp"
#include "qt_test_helpers.hpp"

#include <QAccessible>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQuickWindow>
#include <QSGNode>

#include <cstdlib>
#include <iostream>
#include <memory>
#include <string>
#include <string_view>

namespace {

using nve::qt_test::BinaryCmdHelper;

void send_quick_click(QQuickItem* item, const QPointF& pos) {
  QMouseEvent press(QEvent::MouseButtonPress, pos, pos, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
  QMouseEvent release(QEvent::MouseButtonRelease, pos, pos, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
  QCoreApplication::sendEvent(item, &press);
  QCoreApplication::sendEvent(item, &release);
}

void send_quick_key(QQuickItem* item, int key, const QString& text) {
  QKeyEvent ev(QEvent::KeyPress, key, Qt::NoModifier, text);
  QCoreApplication::sendEvent(item, &ev);
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
  NVE_CHECK(!cold->replaySnapshot(snap_path, 99, 1, &restore_us));
  NVE_CHECK(cold->replaySnapshot(snap_path, 4, 1, &restore_us));
  std::cout << "[qt_quick_test] Cold mmap Frame-0 restore (fixture dylib): "
            << restore_us << " us\n";

  NVE_CHECK(!cold->isAppLoaded() && cold->nodeCount() == 3u && cold->sceneNodeText(3) == "Fixture:3");

  win.show();
  QCoreApplication::processEvents();
  std::unique_ptr<QSGNode> f0_sg(cold->syncSceneGraphNode(nullptr));
  NVE_CHECK(f0_sg != nullptr && cold->sceneGraphStats().textTextureNodes >= 1u);

  NVE_CHECK(cold->loadDylib(QStringLiteral(NVE_TEST_VALID_DYLIB), 1));
  NVE_CHECK(cold->isAppLoaded() && cold->sceneNodeText(3) == "Fixture:3");

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
      self->cmd.set_layout(2, 1, 1, 200.0f, 1, 100.0f, 0.0f, 0.0f, 0.0f);
      self->cmd.set_layout(3, 0, 1, 100.0f, 1, 80.0f, 0.0f, 0.0f, 0.0f);
      self->cmd.set_style(3, 0xFFFF0000u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 14.0f, 10);
      self->cmd.set_layout(4, 0, 1, 100.0f, 1, 80.0f, 0.0f, 0.0f, 0.0f);
      self->cmd.set_style(4, 0xFF00FF00u, 0xFFFFFFFFu, 0, 0.0f, 0.0f, 1.0f, 14.0f, 1);
      return self->cmd.finish();
    }
    static uint32_t dispatch_events(NveAppHandle, NveEventBatch) { return 0u; }
    static const void* get_interface(uint32_t id) {
      static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
      return (id == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
    }
  };

  nve::NveQtQuickItem item;
  item.setSize(QSizeF(200, 100));
  NVE_CHECK(item.attachInterface(&ZIndexComponent::get_interface, 1));

  std::unique_ptr<QSGNode> sg(item.syncSceneGraphNode(nullptr));
  NVE_CHECK(sg != nullptr);
  QSGNode* root_wrap = sg->firstChild();
  NVE_CHECK(root_wrap != nullptr);
  QSGNode* c2_wrap = root_wrap->firstChild();
  NVE_CHECK(c2_wrap != nullptr && c2_wrap->childCount() == 2);
  NVE_CHECK(c2_wrap->childAtIndex(0)->type() == QSGNode::TransformNodeType);
  NVE_CHECK(c2_wrap->childAtIndex(1)->type() == QSGNode::BasicNodeType);
  NVE_CHECK(item.hitTestNode(QPointF(50.0, 40.0), true) == 3u);
  NVE_CHECK(item.hitTestNode(QPointF(150.0, 40.0), true) == 4u);
}

void test_live_counter_and_studio_dylibs() {
  const char* counter_env = std::getenv("NVE_TEST_KITCHEN_SINK_DYLIB");
  if (counter_env != nullptr && counter_env[0] != '\0') {
    QQuickWindow win;
    win.resize(1280, 800);
    auto* item = new nve::NveQtQuickItem(win.contentItem());
    item->setSize(QSizeF(1280, 800));

    const QString counter_path = QString::fromUtf8(counter_env);
    int err_code = -1;
    QString err_msg;
    NVE_CHECK(item->loadDylib(counter_path, 1, &err_code, &err_msg));
    NVE_CHECK(err_code == 0 && err_msg.isEmpty());

    auto find_node_by_text = [&](std::string_view needle) -> uint32_t {
      const NveSceneView view = item->sceneView();
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        if (nve::scene_node_text(view, view.nodes[i]) == needle) return view.nodes[i].node_id;
      }
      return 0u;
    };

    const uint32_t title_id = find_node_by_text("NVE Kitchen Sink");
    const uint32_t count_id = find_node_by_text("Count: 0");
    const uint32_t inc_text_id = find_node_by_text("Increment +");
    const uint32_t reset_text_id = find_node_by_text("Reset");
    NVE_CHECK(title_id != 0u && count_id != 0u && inc_text_id != 0u && reset_text_id != 0u);

    const QRectF inc_bounds = item->sceneNodeGlobalBounds(inc_text_id);
    NVE_CHECK(item->hitTestNode(inc_bounds.center(), true) != 0u);

    win.show();
    QCoreApplication::processEvents();
    std::unique_ptr<QSGNode> sg(item->syncSceneGraphNode(nullptr));
    NVE_CHECK(item->sceneGraphStats().textRasterizations >= 4u);

    send_quick_click(item, inc_bounds.center());
    (void)item->syncSceneGraphNode(sg.get());
    send_quick_click(item, inc_bounds.center());
    (void)item->syncSceneGraphNode(sg.get());
    NVE_CHECK(find_node_by_text("Count: 2") != 0u);
    NVE_CHECK(item->sceneGraphStats().textCacheHits >= 6u);

    NVE_CHECK(item->hotReload(counter_path, &err_code, &err_msg));
    NVE_CHECK(err_code == 0 && find_node_by_text("Count: 2") != 0u);

    const QString counter_snap = QDir::temp().filePath(
        QStringLiteral("nve_qt_quick_counter_%1.nvesnap").arg(QCoreApplication::applicationPid()));
    QFile::remove(counter_snap);
    NVE_CHECK(item->saveSnapshot(counter_snap, 1));
    const NveMapEntry reset_initial[1] = {
        NveMapEntry{nve::make_str_slice("initialCount"), nve::make_i64_value(0)},
    };
    NVE_CHECK(item->applyProps(reset_initial, 1u));

    nve::NveQtQuickItem cold_counter;
    cold_counter.setSize(QSizeF(640, 420));
    uint64_t restore_us = 0;
    NVE_CHECK(cold_counter.replaySnapshot(counter_snap, 1, 1, &restore_us));
    std::cout << "[qt_quick_test] Cold mmap Frame-0 restore (counter dylib): "
              << restore_us << " us\n";

    auto cold_has_text = [&](std::string_view needle) {
      const NveSceneView view = cold_counter.sceneView();
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        if (nve::scene_node_text(view, view.nodes[i]) == needle) return true;
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
    const NveMapEntry studio_props[2] = {
        NveMapEntry{nve::make_str_slice("accent"), nve::make_str_value("violet")},
        NveMapEntry{nve::make_str_slice("view"), nve::make_str_value("metrics")},
    };
    NVE_CHECK(studio->applyProps(studio_props, 2u));

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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickA11yImeComponent*>(h);
    if (batch.events == nullptr) return 0u;
    auto read_text = [&](const NveEvent& ev) -> std::string {
      if (ev.bytes.len == 0u || batch.bytes == nullptr || ev.bytes.offset > batch.bytes_len ||
          ev.bytes.len > batch.bytes_len - ev.bytes.offset) {
        return {};
      }
      return std::string(
          reinterpret_cast<const char*>(batch.bytes + ev.bytes.offset), ev.bytes.len);
    };
    for (uint32_t i = 0u; i < batch.event_count; ++i) {
      const NveEvent& ev = batch.events[i];
      if (ev.node == 5u && ev.table == NVE_TABLE_TEXT_INPUT &&
          ev.event == NVE_TEXT_INPUT_EVENT_CHANGE) {
        ++self->direct_text_calls;
        self->last_text = read_text(ev);
        self->input_text = self->last_text;
        return 1u;
      }
      if (ev.node == 5u && ev.table == NVE_TABLE_TEXT_INPUT &&
          ev.event == NVE_TEXT_INPUT_EVENT_SUBMIT) {
        ++self->direct_submit_calls;
        self->last_submit = read_text(ev);
      }
    }
    return 0u;
  }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    return (iface == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
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

  NVE_CHECK(item->accessibleRoleForNodeId(3) == QAccessible::Button);
  NVE_CHECK(item->accessibleNameForNodeId(3) == QStringLiteral("Disabled Action"));
  NVE_CHECK(item->accessibleDescriptionForNodeId(3) == QStringLiteral("Currently unavailable"));
  NVE_CHECK(item->accessibleRoleForNodeId(4) == QAccessible::Heading);
  NVE_CHECK(item->accessibleNameForNodeId(4) == QStringLiteral("Quick Heading"));
  NVE_CHECK(item->accessibleRoleForNodeId(5) == QAccessible::EditableText);
  NVE_CHECK(item->accessibleNameForNodeId(5) == QStringLiteral("Quick Search"));
  const auto input_sem = item->accessibilityInfoForNodeId(5);
  NVE_CHECK(input_sem.has_value() && input_sem->placeholder == QStringLiteral("Search IME..."));

  const QRectF disabled_btn_bounds = item->sceneNodeGlobalBounds(3);
  NVE_CHECK(disabled_btn_bounds.isValid());
  NVE_CHECK(item->hitTestNode(disabled_btn_bounds.center(), true) == 0u);

  const QRectF input_bounds = item->sceneNodeGlobalBounds(5);
  NVE_CHECK(input_bounds.isValid());
  send_quick_click(item, input_bounds.center());
  NVE_CHECK(item->focusedNodeId() == 5u && item->inputMethodQuery(Qt::ImEnabled).toBool());

  {
    QList<QInputMethodEvent::Attribute> attrs;
    QInputMethodEvent preedit_ev(QStringLiteral("かんじ"), attrs);
    QCoreApplication::sendEvent(item, &preedit_ev);
    NVE_CHECK(item->imePreeditText(5) == QStringLiteral("かんじ"));
    NVE_CHECK(mock->direct_text_calls == 0);

    send_quick_key(item, Qt::Key_Return, QStringLiteral("\r"));
    NVE_CHECK(mock->direct_submit_calls == 0);
  }

  {
    QInputMethodEvent commit_ev;
    commit_ev.setCommitString(QStringLiteral("漢字 naïve"));
    QCoreApplication::sendEvent(item, &commit_ev);
    NVE_CHECK(item->imePreeditText(5).isEmpty());
    NVE_CHECK(mock->direct_text_calls == 1 && mock->last_text == "漢字 naïve");
    NVE_CHECK(item->sceneNodeText(5) == "漢字 naïve");
  }

  {
    QInputMethodEvent replace_ev;
    replace_ev.setCommitString(QStringLiteral("café"), -5, 5);
    QCoreApplication::sendEvent(item, &replace_ev);
    NVE_CHECK(mock->direct_text_calls == 2 && mock->last_text == "漢字 café");
    NVE_CHECK(item->sceneNodeText(5) == "漢字 café");
  }

  send_quick_key(item, Qt::Key_Return, QStringLiteral("\r"));
  NVE_CHECK(mock->direct_submit_calls == 1 && mock->last_submit == "漢字 café");
}

} // namespace

void nve_test_qt_quick_extra() {
  test_mmap_snapshot_roundtrip();
  test_z_index_hit_test_and_paint_order();
  test_live_counter_and_studio_dylibs();
  test_accessibility_and_ime_input_method();
}
