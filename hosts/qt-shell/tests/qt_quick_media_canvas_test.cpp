#include "NveQtQuickItem.hpp"
#include "qt_test_helpers.hpp"

#include <QBuffer>
#include <QColor>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QImage>
#include <QQuickWindow>
#include <QSGNode>
#include <QUrl>

#include <cmath>
#include <memory>
#include <string>
#include <vector>

namespace {

using nve::qt_test::BinaryCmdHelper;

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
  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickImageComponent*>(h);
    if (batch.ptr != nullptr && batch.len > 0u) {
      ++self->ticks;
      return 1u;
    }
    return 0u;
  }
  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
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
  NVE_CHECK(s1.imageTextureNodes == 2u && s1.cachedImageTextures == 2u && s1.textTextureNodes == 1u);

  const QSGNode* img3_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3);
  const QSGNode* img4_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 4);
  NVE_CHECK(img3_before != nullptr && img4_before != nullptr);

  item->dispatchPress(5, 2.0f, 2.0f);
  NVE_CHECK(item->sceneNodeText(5) == "Tick:1");
  NVE_CHECK(item->syncSceneGraphNode(sg_root.get()) == sg_root.get());
  NVE_CHECK(item->sceneGraphStats().dirtyNodesUpdated == 1u);
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
  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockQuickCanvasComponent*>(h);
    if (batch.ptr != nullptr && batch.len > 0u) {
      ++self->ticks;
      return 1u;
    }
    return 0u;
  }
  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
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

  win.show();
  QCoreApplication::processEvents();
  const QImage grabbed1 = win.grabWindow();
  NVE_CHECK(!grabbed1.isNull() && grabbed1.width() > 0 && grabbed1.height() > 0);
  int bg_px = 0;
  int cyan_px = 0;
  for (int y = 0; y < grabbed1.height(); ++y) {
    for (int x = 0; x < grabbed1.width(); ++x) {
      const QColor c = grabbed1.pixelColor(x, y);
      if (std::abs(c.red() - 18) <= 3 && std::abs(c.green() - 28) <= 3 && std::abs(c.blue() - 52) <= 3) {
        ++bg_px;
      }
      if (c.blue() > 210 && c.green() > 150 && c.red() < 90) {
        ++cyan_px;
      }
    }
  }
  NVE_CHECK(bg_px > 4000 && cyan_px > 35);

  mock->draw_list_attr = nve::encode_draw_list_attr_base64(pink_bytes);
  NVE_CHECK(item->renderFrame() == 0);
  QCoreApplication::processEvents();
  const QImage grabbed2 = win.grabWindow();
  int pink_px = 0;
  for (int y = 0; y < grabbed2.height(); ++y) {
    for (int x = 0; x < grabbed2.width(); ++x) {
      const QColor c = grabbed2.pixelColor(x, y);
      if (c.red() > 210 && c.green() < 100 && c.blue() > 70 && c.blue() < 130) ++pink_px;
    }
  }
  NVE_CHECK(pink_px > 35);

  mock->emit_corrupt_raw = true;
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(cyan_bytes.data()), 18u);
  NVE_CHECK(item->renderFrame() == 0);
  QCoreApplication::processEvents();
  NVE_CHECK(!win.grabWindow().isNull());

  mock->emit_corrupt_raw = false;
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(cyan_bytes);
  NVE_CHECK(item->renderFrame() == 0);

  std::unique_ptr<QSGNode> sg_root(item->syncSceneGraphNode(nullptr));
  NVE_CHECK(sg_root != nullptr);
  const nve::SceneGraphStats s1 = item->sceneGraphStats();
  NVE_CHECK(s1.canvasTextureNodes == 1u && s1.cachedCanvasTextures == 1u && s1.canvasRasterizations == 1u);

  const QSGNode* canvas_sg_before = nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3);
  NVE_CHECK(canvas_sg_before != nullptr);
  item->dispatchPress(4, 2.0f, 2.0f);
  NVE_CHECK(item->sceneNodeText(4) == "Frame:1");
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats s2 = item->sceneGraphStats();
  NVE_CHECK(s2.canvasRasterizations == 1u && s2.canvasTextureNodes == 1u);
  NVE_CHECK(nve::NveQtQuickItem::retainedSGNodeForNodeId(sg_root.get(), 3) == canvas_sg_before);

  mock->draw_list_attr = nve::encode_draw_list_attr_base64(pink_bytes);
  NVE_CHECK(item->renderFrame() == 0);
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats s3 = item->sceneGraphStats();
  NVE_CHECK(s3.canvasRasterizations == 2u && s3.canvasTextureNodes == 1u);

  mock->emit_corrupt_raw = true;
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(cyan_bytes.data()), 18u);
  NVE_CHECK(item->renderFrame() == 0);
  (void)item->syncSceneGraphNode(sg_root.get());
  const nve::SceneGraphStats s4 = item->sceneGraphStats();
  NVE_CHECK(s4.canvasTextureNodes == 0u && s4.cachedCanvasTextures == 0u);

  sg_root.reset();
}

} // namespace

void nve_test_qt_quick_media_canvas() {
  test_quick_image_rendering_and_retained_sg_survival();
  test_quick_draw_list_canvas_gpu_cache_and_corrupt_survival();
}
