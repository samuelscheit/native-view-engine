#include "NveQtWidget.hpp"
#include "command_encoder.hpp"
#include "nve_pair_client.hpp"
#include "qt_test_helpers.hpp"
#ifdef NVE_QT_HAS_QUICK
#include "NveQtQuickItem.hpp"
#endif

#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <QColor>
#include <QCoreApplication>
#include <QImage>
#include <QLabel>
#include <QPainter>

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

namespace {

using nve::qt_test::BinaryCmdHelper;

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

  static uint32_t dispatch_events(NveAppHandle, NveEventBatch) { return 0u; }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    return (iface == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
  }
};

void test_draw_list_canvas_widget_and_corrupt_survival() {
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
    NVE_CHECK(res.ok() && res.ops_decoded >= 8u);
    p.end();

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
    NVE_CHECK(bg_pixels > 5000 && cyan_pixels > 40);
  }

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
    NVE_CHECK(nve::qt::renderDrawListToPainter(p, QRectF(0.0, 0.0, 64.0, 64.0), unbal_bytes).ok());
    NVE_CHECK(!p.hasClipping());
    const QColor inside = img.pixelColor(20, 20);
    const QColor outside = img.pixelColor(4, 4);
    NVE_CHECK(inside.red() > 220 && inside.green() < 50);
    NVE_CHECK(std::abs(outside.red() - 20) <= 2 && std::abs(outside.green() - 20) <= 2);

    nve::DrawListEncoder extra_pop;
    extra_pop.restore_clip().restore_clip().fill_circle(32.0f, 32.0f, 8.0f, nve::DrawColorRgba::rgb(10u, 220u, 90u));
    NVE_CHECK(nve::qt::renderDrawListToPainter(p, QRectF(0.0, 0.0, 64.0, 64.0), extra_pop.finish()).ok());
    NVE_CHECK(!p.hasClipping());
    p.end();
    const QColor center = img.pixelColor(32, 32);
    NVE_CHECK(center.green() > 200 && center.red() < 40);
  }

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
    NVE_CHECK(img.pixelColor(32, 32) == QColor(11, 22, 33, 255));
  }

  nve::qt::NveQtWidget widget;
  widget.resize(240, 140);
  NVE_CHECK(widget.attachInterface(&MockQtCanvasApp::get_interface, 41));
  NVE_CHECK(MockQtCanvasApp::last_instance != nullptr);
  auto* mock = MockQtCanvasApp::last_instance;

  auto* canvas = qobject_cast<nve::qt::NveQtCanvasWidget*>(widget.nativeWidgetForNodeId(3));
  NVE_CHECK(canvas != nullptr && canvas->drawListBytes().empty());

  mock->draw_list_attr = nve::encode_draw_list_attr_base64(spark_bytes);
  NVE_CHECK(widget.renderFrame() == 0);
  NVE_CHECK(canvas->drawListBytes().size() == spark_bytes.size());
  NVE_CHECK(std::memcmp(canvas->drawListBytes().data(), spark_bytes.data(), spark_bytes.size()) == 0);
  const uint64_t redraws_after_first = canvas->redrawRequestCount();
  NVE_CHECK(redraws_after_first == 1u);

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
      if (std::abs(c.red() - 15) <= 2 && std::abs(c.green() - 23) <= 2 && std::abs(c.blue() - 42) <= 2) {
        ++grabbed_bg;
      }
      if (c.blue() > 210 && c.green() > 150 && c.red() < 90) {
        ++grabbed_cyan;
      }
    }
  }
  NVE_CHECK(grabbed_bg > 5000 && grabbed_cyan > 40);

  nve::DrawListEncoder violet_enc;
  violet_enc.sparkline(samples, 200.0f, 80.0f, nve::DrawColorRgba::rgb(236u, 72u, 153u), bg_col);
  mock->draw_list_attr = violet_enc.to_attr_string();
  NVE_CHECK(widget.renderFrame() == 0);
  NVE_CHECK(canvas->redrawRequestCount() == redraws_after_first + 1u);

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
      auto read_frame = [&](int fd, std::vector<uint8_t>& buf, nve::WireFrameView& out) -> bool {
        uint8_t hdr_bytes[nve::kWireHeaderSize]{};
        if (!read_exact(fd, hdr_bytes, nve::kWireHeaderSize)) return false;
        nve::WireHeader hdr{};
        if (!nve::wire_v2::decode_header(hdr_bytes, hdr)) return false;
        buf.resize(hdr.total_wire_len());
        std::memcpy(buf.data(), hdr_bytes, nve::kWireHeaderSize);
        if (buf.size() > nve::kWireHeaderSize &&
            !read_exact(fd, buf.data() + nve::kWireHeaderSize,
                        buf.size() - nve::kWireHeaderSize)) {
          return false;
        }
        return nve::wire_v2::decode_frame(buf, out);
      };
      auto write_all = [](int fd, std::span<const uint8_t> p) -> bool {
        size_t off = 0;
        while (off < p.size()) {
          const ssize_t n = ::send(fd, p.data() + off, p.size() - off, 0);
          if (n <= 0) return false;
          off += static_cast<size_t>(n);
        }
        return true;
      };

      int cfd = ::accept(lfd, nullptr, nullptr);
      NVE_CHECK(cfd >= 0);
      std::vector<uint8_t> msg;
      nve::WireFrameView v{};
      NVE_CHECK(read_frame(cfd, msg, v) && v.header.kind == nve::WireKind::Hello);
      nve::WireHello hp{};
      NVE_CHECK(nve::wire_v2::decode_hello(v.payload, hp));
      NVE_CHECK(nve::wire_v2::constant_time_eq(hp.bearer_token, token));
      const auto ack_b = nve::wire_v2::encode_hello_ack({77u, 2u, 0x0Fu});
      NVE_CHECK(write_all(cfd, nve::wire_v2::encode_frame(
                                   nve::WireChannel::Control, nve::WireKind::HelloAck, 1u, ack_b)));
      NVE_CHECK(write_all(cfd, nve::wire_v2::encode_frame(
                                   nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 1u, f1)));
      NVE_CHECK(write_all(cfd, nve::wire_v2::encode_frame(
                                   nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 2u, f2)));

      while (read_frame(cfd, msg, v)) {
        if (v.header.kind == nve::WireKind::EventBatch) {
          std::vector<nve::DecodedEvent> dec;
          if (nve::decode_event_ring(NveBufferSlice{v.payload.data(), v.payload.size()}, dec) &&
              dec.size() == 1u && dec[0].kind == nve::EventKind::Press && dec[0].node_id == 4u) {
            saw_press.store(true);
            break;
          }
        } else if (v.header.kind == nve::WireKind::Bye) {
          break;
        }
      }
      ::close(cfd);
      ::close(lfd);
    });
  };

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
      if (lbl != nullptr && lbl->text() == QStringLiteral("Qt Paired V2")) break;
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
      if (item.sceneNodeText(3u) == "Quick Paired V2") break;
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

void nve_test_qt_canvas_and_pair() {
  test_draw_list_canvas_widget_and_corrupt_survival();
  test_paired_thin_client_widgets_and_quick();
}
