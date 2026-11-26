#include "NveGtkHost.hpp"
#include "command_encoder.hpp"
#include "nve_pair_client.hpp"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace {

bool approx_eq(double a, double b, double eps = 1.5) {
  return std::fabs(a - b) <= eps;
}

struct MockGtkCanvasComponent {
  static inline MockGtkCanvasComponent* last_instance = nullptr;
  uint32_t surface_id{1u};
  std::string draw_list_attr;
  bool emit_raw_corrupt{false};
  std::string raw_corrupt;
  int tick{0};
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockGtkCanvasComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockGtkCanvasComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockGtkCanvasComponent*>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0u;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0u);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0u);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0u);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_text = [&](uint32_t id, const std::string& s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };
    auto set_box = [&](uint32_t id, float w, float h) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push<uint8_t>(1u); self->push<float>(w);
      self->push<uint8_t>(1u); self->push<float>(h);
      for (int i = 0; i < 4; ++i) { self->push<uint8_t>(0u); self->push<float>(0.0f); }
      self->push<uint8_t>(0u); self->push<uint8_t>(0u); self->push<uint8_t>(0u);
      self->push<float>(0.0f); self->push<float>(0.0f); self->push<float>(0.0f);
      for (int i = 0; i < 8; ++i) self->push<float>(0.0f);
      ++ops;
    };
    auto set_raw_str = [&](uint32_t id, std::string_view k, std::string_view v) {
      self->push<uint8_t>(NVE_OP_SET_RAW_PROP);
      self->push<uint32_t>(id);
      self->push<uint16_t>(static_cast<uint16_t>(k.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), k.begin(), k.end());
      self->push<uint8_t>(4u);
      self->push<uint32_t>(static_cast<uint32_t>(v.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), v.begin(), v.end());
      ++ops;
    };

    create_node(2u, NVE_COMP_VIEW);
    create_node(3u, NVE_COMP_SKIA_CANVAS);
    create_node(4u, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2u});
    set_children(2u, {3u, 4u});

    set_box(2u, 240.0f, 140.0f);
    set_box(3u, 200.0f, 80.0f);
    if (self->emit_raw_corrupt) {
      set_raw_str(3u, "drawList", "");
      set_text(3u, self->raw_corrupt);
    } else {
      set_raw_str(3u, "drawList", self->draw_list_attr);
    }

    set_box(4u, 120.0f, 24.0f);
    set_text(4u, "Tick:" + std::to_string(self->tick));

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle, NveEventBatch) { return 0u; }

  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    return (iface == NVE_IFACE_SURFACE) ? &kSurface : nullptr;
  }
};

void test_cairo_draw_list_sparkline_and_drawing_area() {
  const std::vector<float> samples{14.0f, 28.0f, 19.0f, 47.0f, 38.0f, 70.0f};
  const nve::DrawColorRgba bg_col = nve::DrawColorRgba::rgb(15u, 23u, 42u);
  const nve::DrawColorRgba stroke_col = nve::DrawColorRgba::rgb(56u, 189u, 248u);

  nve::DrawListEncoder spark_enc;
  spark_enc.sparkline(samples, 200.0f, 80.0f, stroke_col, bg_col);
  const std::vector<uint8_t> spark_bytes = spark_enc.finish();

  {
    cairo_surface_t* surf = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 200, 80);
    cairo_t* cr = cairo_create(surf);
    const auto res = nve::render_draw_list_to_cairo(cr, 200.0, 80.0, spark_bytes);
    NVE_CHECK(res.ok() && res.ops_decoded >= 8u && cairo_status(cr) == CAIRO_STATUS_SUCCESS);
    cairo_surface_flush(surf);

    const unsigned char* data = cairo_image_surface_get_data(surf);
    const int stride = cairo_image_surface_get_stride(surf);
    NVE_CHECK(data != nullptr && stride >= 200 * 4);

    int bg_pixels = 0;
    int stroke_pixels = 0;
    for (int y = 0; y < 80; ++y) {
      const auto* row = reinterpret_cast<const uint32_t*>(data + y * stride);
      for (int x = 0; x < 200; ++x) {
        const uint32_t px = row[x];
        const uint8_t a = static_cast<uint8_t>((px >> 24u) & 0xFFu);
        const uint8_t r = static_cast<uint8_t>((px >> 16u) & 0xFFu);
        const uint8_t g = static_cast<uint8_t>((px >> 8u) & 0xFFu);
        const uint8_t b = static_cast<uint8_t>(px & 0xFFu);
        if (a == 255u && std::abs(static_cast<int>(r) - 15) <= 2 &&
            std::abs(static_cast<int>(g) - 23) <= 2 &&
            std::abs(static_cast<int>(b) - 42) <= 2) {
          ++bg_pixels;
        }
        if (a == 255u && b > 210u && g > 150u && r < 90u) {
          ++stroke_pixels;
        }
      }
    }
    NVE_CHECK(bg_pixels > 5000 && stroke_pixels > 40);

    double cx0 = 0.0, cy0 = 0.0, cx1 = 0.0, cy1 = 0.0;
    cairo_clip_extents(cr, &cx0, &cy0, &cx1, &cy1);

    nve::DrawListEncoder unbal;
    unbal.clip_rect(10.0f, 10.0f, 30.0f, 30.0f)
        .clip_rect(15.0f, 15.0f, 20.0f, 20.0f)
        .fill_rect(0.0f, 0.0f, 200.0f, 80.0f, nve::DrawColorRgba::rgb(240u, 30u, 50u));
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, unbal.finish()).ok());

    double ax0 = 0.0, ay0 = 0.0, ax1 = 0.0, ay1 = 0.0;
    cairo_clip_extents(cr, &ax0, &ay0, &ax1, &ay1);
    NVE_CHECK(approx_eq(ax0, cx0) && approx_eq(ay0, cy0) &&
              approx_eq(ax1, cx1) && approx_eq(ay1, cy1));

    nve::DrawListEncoder extra_pop;
    extra_pop.restore_clip().restore_clip().fill_circle(
        50.0f, 40.0f, 6.0f, nve::DrawColorRgba::rgb(34u, 197u, 94u));
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, extra_pop.finish()).ok());
    NVE_CHECK(cairo_status(cr) == CAIRO_STATUS_SUCCESS);

    const std::vector<uint8_t> bad_magic{'N', 'O', 'P', 'E', '0', '0', '0', '0', '0', 1, 0, 0, 0, 0, 0};
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, bad_magic).error ==
              nve::DrawListError::BadMagic);

    std::vector<uint8_t> truncated = spark_bytes;
    truncated.resize(20u);
    NVE_CHECK(!nve::render_draw_list_to_cairo(cr, 200.0, 80.0, truncated).ok());

    std::vector<uint8_t> trailing = spark_bytes;
    trailing.push_back(0xFFu);
    NVE_CHECK(nve::render_draw_list_to_cairo(cr, 200.0, 80.0, trailing).error ==
              nve::DrawListError::TrailingBytes);
    NVE_CHECK(cairo_status(cr) == CAIRO_STATUS_SUCCESS);

    cairo_destroy(cr);
    cairo_surface_destroy(surf);
  }

  NveSurfaceConfig cfg{.surface_id = 21u, .width = 240.0f, .height = 140.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  NVE_CHECK(host.attach_interface(&MockGtkCanvasComponent::get_interface, 21u));
  NVE_CHECK(MockGtkCanvasComponent::last_instance != nullptr);
  auto* mock = MockGtkCanvasComponent::last_instance;

  GtkWidget* canvas = host.widget_for_node_id(3u);
  NVE_CHECK(canvas != nullptr && GTK_IS_DRAWING_AREA(canvas));
  NVE_CHECK(host.canvas_draw_list_for_node_id(3u).empty());

  mock->draw_list_attr = nve::encode_draw_list_attr_base64(spark_bytes);
  NVE_CHECK(host.render_frame() == 0);
  const auto cached = host.canvas_draw_list_for_node_id(3u);
  NVE_CHECK(cached.size() == spark_bytes.size());
  NVE_CHECK(std::memcmp(cached.data(), spark_bytes.data(), spark_bytes.size()) == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 1u);

  mock->tick = 1;
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 1u);

  nve::DrawListEncoder alt_enc;
  alt_enc.sparkline(samples, 200.0f, 80.0f, nve::DrawColorRgba::rgb(236u, 72u, 153u), bg_col);
  mock->draw_list_attr = alt_enc.to_attr_string();
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 2u);

  mock->emit_raw_corrupt = true;
  mock->raw_corrupt.assign(reinterpret_cast<const char*>(spark_bytes.data()), 18u);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.canvas_redraw_count_for_node_id(3u) == 3u);
}

void test_paired_thin_client_gtk_host() {
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

  const std::string token = "gtk-pair-token";
  const auto f1 = build_frame(1u, "GTK Paired V1");
  const auto f2 = build_frame(2u, "GTK Paired V2");

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
  const uint16_t port = ntohs(addr.sin_port);
  NVE_CHECK(::listen(lfd, 2) == 0);

  std::atomic<bool> saw_press{false};
  std::thread srv([&]() {
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
    NVE_CHECK(write_all(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::Control, nve::WireKind::HelloAck, 1u, ack_b)));

    NVE_CHECK(write_all(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 1u, f1)));
    NVE_CHECK(write_all(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 2u, f2)));

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

  NveSurfaceConfig cfg{.surface_id = 1u, .width = 320.0f, .height = 200.0f, .scale_factor = 1.0f};
  nve::NveGtkHost host(cfg);
  std::string err;
  NVE_CHECK(host.connect_paired_endpoint(
      "127.0.0.1:" + std::to_string(port), token, "tcp", &err, 1u));
  NVE_CHECK(host.is_paired() && host.is_paired_connected() && host.is_loaded());

  const auto dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (std::chrono::steady_clock::now() < dl) {
    while (g_main_context_iteration(nullptr, FALSE)) {}
    GtkWidget* lbl = host.widget_for_node_id(3u);
    if (lbl != nullptr && GTK_IS_LABEL(lbl) &&
        std::string(gtk_label_get_text(GTK_LABEL(lbl))) == "GTK Paired V2") {
      break;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  GtkWidget* lbl = host.widget_for_node_id(3u);
  NVE_CHECK(lbl != nullptr && GTK_IS_LABEL(lbl));
  NVE_CHECK(std::string(gtk_label_get_text(GTK_LABEL(lbl))) == "GTK Paired V2");

  NVE_CHECK(host.simulate_press(4u, 6.0f, 8.0f) == 1u);
  const auto ev_dl = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (!saw_press.load() && std::chrono::steady_clock::now() < ev_dl) {
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(saw_press.load());

  host.disconnect_paired();
  NVE_CHECK(!host.is_paired());
  srv.join();
}

} // namespace

void nve_test_gtk_canvas_and_pair() {
  test_cairo_draw_list_sparkline_and_drawing_area();
  test_paired_thin_client_gtk_host();
}
