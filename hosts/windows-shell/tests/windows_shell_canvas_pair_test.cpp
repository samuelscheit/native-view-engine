#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#else
#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

#include "NveRnwComponentView.hpp"
#include "NveWinWindowManager.hpp"
#include "command_encoder.hpp"
#include "nve_pair_client.hpp"
#include "windows_test_helpers.hpp"

#include <array>
#include <atomic>
#include <chrono>
#include <cstring>
#include <initializer_list>
#include <mutex>
#include <span>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

namespace {

struct MockWinCanvasComponent {
  static inline MockWinCanvasComponent* last_instance = nullptr;
  uint32_t surface_id{1};
  std::string draw_list_attr;
  bool emit_corrupt_raw{false};
  std::string corrupt_raw;
  int tick{0};
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockWinCanvasComponent();
    self->surface_id = sid;
    last_instance = self;
    return self;
  }
  static void destroy(NveAppHandle h) {
    auto* self = static_cast<MockWinCanvasComponent*>(h);
    if (last_instance == self) last_instance = nullptr;
    delete self;
  }
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockWinCanvasComponent*>(h);
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
    if (self->emit_corrupt_raw) {
      set_raw_str(3u, "drawList", "");
      set_text(3u, self->corrupt_raw);
    } else {
      set_raw_str(3u, "drawList", self->draw_list_attr);
    }

    set_box(4u, 120.0f, 24.0f);
    set_text(4u, "WinTick:" + std::to_string(self->tick));

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

struct DrawListOpCounts {
  uint32_t clear_ops{0};
  uint32_t fill_rect_ops{0};
  uint32_t fill_rounded_rect_ops{0};
  uint32_t fill_circle_ops{0};
  uint32_t stroke_line_ops{0};
  uint32_t stroke_polyline_ops{0};
  uint32_t polyline_vertices{0};
  uint32_t clip_rect_ops{0};
  uint32_t restore_clip_ops{0};

  void on_clear(const nve::DrawClearOp&) noexcept { ++clear_ops; }
  void on_fill_rect(const nve::DrawFillRectOp&) noexcept { ++fill_rect_ops; }
  void on_fill_rounded_rect(const nve::DrawFillRoundedRectOp&) noexcept { ++fill_rounded_rect_ops; }
  void on_fill_circle(const nve::DrawFillCircleOp&) noexcept { ++fill_circle_ops; }
  void on_stroke_line(const nve::DrawStrokeLineOp&) noexcept { ++stroke_line_ops; }
  void on_stroke_polyline(const nve::DrawStrokePolylineOp& op) noexcept {
    ++stroke_polyline_ops;
    polyline_vertices += static_cast<uint32_t>(op.points.size());
  }
  void on_clip_rect(const nve::DrawClipRectOp&) noexcept { ++clip_rect_ops; }
  void on_restore_clip(const nve::DrawRestoreClipOp&) noexcept { ++restore_clip_ops; }
};

void test_draw_list_visual_tree_extraction_and_corrupt_survival() {
  const std::vector<float> samples{12.0f, 25.0f, 18.0f, 44.0f, 39.0f, 68.0f};
  const nve::DrawColorRgba bg_col = nve::DrawColorRgba::rgb(15u, 23u, 42u);
  const nve::DrawColorRgba stroke_col = nve::DrawColorRgba::rgb(56u, 189u, 248u);

  nve::DrawListEncoder spark_enc;
  spark_enc.sparkline(samples, 200.0f, 80.0f, stroke_col, bg_col);
  const std::vector<uint8_t> spark_bytes = spark_enc.finish();

  nve::win::NveRnwComponentHost host;
  const NveSurfaceConfig cfg{.surface_id = 23u, .width = 240.0f, .height = 140.0f, .scale_factor = 1.0f};
  NVE_CHECK(host.attach_interface(&MockWinCanvasComponent::get_interface, cfg));
  NVE_CHECK(MockWinCanvasComponent::last_instance != nullptr);
  auto* mock = MockWinCanvasComponent::last_instance;

  const auto* canvas0 = host.visual_tree().find(3u);
  NVE_CHECK(canvas0 != nullptr && canvas0->kind == NVE_NODE_KIND_SKIA_CANVAS && canvas0->draw_list.empty());

  const uint64_t updates_before_draw = host.visual_tree().updated_count();
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(spark_bytes);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().updated_count() == updates_before_draw + 1u);

  const auto* canvas1 = host.visual_tree().find(3u);
  NVE_CHECK(canvas1 != nullptr && canvas1->draw_list == spark_bytes);
  NVE_CHECK(canvas1->text.empty() && canvas1->image_src.empty());

  DrawListOpCounts counts{};
  NVE_CHECK(nve::decode_draw_list(canvas1->draw_list, counts).ok());
  NVE_CHECK(counts.clear_ops == 1u && counts.fill_rounded_rect_ops == 1u && counts.clip_rect_ops == 1u);
  NVE_CHECK(counts.stroke_line_ops == 3u && counts.fill_rect_ops == 1u && counts.stroke_polyline_ops == 1u);
  NVE_CHECK(counts.polyline_vertices == 6u && counts.fill_circle_ops == 1u && counts.restore_clip_ops == 1u);

  const uint64_t updates_before_sibling = host.visual_tree().updated_count();
  mock->tick = 1;
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().updated_count() == updates_before_sibling + 1u);
  NVE_CHECK(host.visual_tree().find(4u)->text == "WinTick:1");
  NVE_CHECK(host.visual_tree().find(3u)->draw_list == spark_bytes);

  const std::vector<float> next_samples{5.0f, 50.0f, 95.0f};
  nve::DrawListEncoder next_enc;
  next_enc.sparkline(next_samples, 200.0f, 80.0f, nve::DrawColorRgba::rgb(244u, 63u, 94u), bg_col);
  const std::vector<uint8_t> next_bytes = next_enc.finish();
  const uint64_t updates_before_next = host.visual_tree().updated_count();
  mock->draw_list_attr = nve::encode_draw_list_attr_base64(next_bytes);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().updated_count() == updates_before_next + 1u);
  NVE_CHECK(host.visual_tree().find(3u)->draw_list == next_bytes);

  mock->emit_corrupt_raw = true;
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(spark_bytes.data()), 18u);
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(host.visual_tree().find(3u)->draw_list.size() == 18u);
  NVE_CHECK(!nve::validate_draw_list(host.visual_tree().find(3u)->draw_list).ok());

  mock->corrupt_raw = "BAD_MAGIC_HEADER_DATA";
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(nve::validate_draw_list(host.visual_tree().find(3u)->draw_list).error ==
            nve::DrawListError::BadMagic);

  std::vector<uint8_t> with_trailing = spark_bytes;
  with_trailing.push_back(0xEEu);
  mock->corrupt_raw.assign(reinterpret_cast<const char*>(with_trailing.data()), with_trailing.size());
  NVE_CHECK(host.render_frame() == 0);
  NVE_CHECK(nve::validate_draw_list(host.visual_tree().find(3u)->draw_list).error ==
            nve::DrawListError::TrailingBytes);
}

#if defined(_WIN32)
using WinTestSock = SOCKET;
using WinTestSockLen = int;
inline bool win_test_sock_valid(WinTestSock s) noexcept { return s != INVALID_SOCKET; }
inline void win_test_close(WinTestSock s) noexcept { if (win_test_sock_valid(s)) ::closesocket(s); }
#else
using WinTestSock = int;
using WinTestSockLen = socklen_t;
inline bool win_test_sock_valid(WinTestSock s) noexcept { return s >= 0; }
inline void win_test_close(WinTestSock s) noexcept { if (win_test_sock_valid(s)) ::close(s); }
#endif

std::vector<uint8_t> build_win_pair_frame(uint64_t frame_id, const std::string& label_text) {
  nve::CommandBufferEncoder enc(1u, frame_id);
  enc.create_node(2u, NVE_COMP_VIEW);
  enc.create_node(3u, NVE_COMP_TEXT);
  enc.create_node(4u, NVE_COMP_PRESSABLE);
  enc.create_node(5u, NVE_COMP_TEXT);
  enc.set_children(NVE_ROOT_NODE_ID, {2u});
  enc.set_children(2u, {3u, 4u});
  enc.set_children(4u, {5u});

  nve::LayoutProps root_box{};
  root_box.width = {1u, 360.0f};
  root_box.height = {1u, 220.0f};
  root_box.padding = {12.0f, 12.0f, 12.0f, 12.0f};
  root_box.gap = 8.0f;
  enc.set_layout(2u, root_box);

  nve::LayoutProps btn{};
  btn.width = {1u, 180.0f};
  btn.height = {1u, 48.0f};
  enc.set_layout(4u, btn);

  enc.set_text(3u, label_text);
  enc.set_text(5u, "Paired Button");

  nve::StyleProps st{};
  st.background_rgba = 0xFF1E293Bu;
  st.text_color_rgba = 0xFFFFFFFFu;
  st.font_size = 18.0f;
  enc.set_style(2u, st);
  enc.set_style(4u, st);
  enc.set_event_mask(4u, nve::kEventMaskPress);
  enc.commit(true);
  const NveBufferSlice slice = enc.finish();
  return std::vector<uint8_t>(slice.ptr, slice.ptr + slice.len);
}

void test_paired_thin_client_loopback_roundtrip() {
#if defined(_WIN32)
  WSADATA wsa_data{};
  NVE_CHECK(::WSAStartup(MAKEWORD(2, 2), &wsa_data) == 0);
#endif
  const std::string token = "nve-win-pair-token";
  const auto frame1 = build_win_pair_frame(1u, "Windows Paired Frame 1");
  const auto frame2 = build_win_pair_frame(2u, "Windows Paired Frame 2");

  WinTestSock listen_fd = ::socket(AF_INET, SOCK_STREAM, 0);
  NVE_CHECK(win_test_sock_valid(listen_fd));
  const int reuse = 1;
#if defined(_WIN32)
  (void)::setsockopt(
      listen_fd, SOL_SOCKET, SO_REUSEADDR, reinterpret_cast<const char*>(&reuse), sizeof(reuse));
#else
  (void)::setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
#endif
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  NVE_CHECK(::bind(listen_fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
  WinTestSockLen alen = static_cast<WinTestSockLen>(sizeof(addr));
  NVE_CHECK(::getsockname(listen_fd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
  const uint16_t port = ntohs(addr.sin_port);
  NVE_CHECK(::listen(listen_fd, 2) == 0);

  std::atomic<bool> saw_input{false};
  std::vector<uint8_t> rx_ev;
  std::mutex ev_mtx;

  auto send_raw = [](WinTestSock fd, std::span<const uint8_t> bytes) -> bool {
    size_t off = 0;
    while (off < bytes.size()) {
#if defined(_WIN32)
      const int n = ::send(fd, reinterpret_cast<const char*>(bytes.data() + off),
                           static_cast<int>(bytes.size() - off), 0);
#else
      const ssize_t n = ::send(fd, bytes.data() + off, bytes.size() - off, 0);
#endif
      if (n <= 0) return false;
      off += static_cast<size_t>(n);
    }
    return true;
  };
  auto recv_raw = [](WinTestSock fd, uint8_t* out, size_t len) -> bool {
    size_t off = 0;
    while (off < len) {
#if defined(_WIN32)
      const int n = ::recv(fd, reinterpret_cast<char*>(out + off), static_cast<int>(len - off), 0);
#else
      const ssize_t n = ::recv(fd, out + off, len - off, 0);
#endif
      if (n <= 0) return false;
      off += static_cast<size_t>(n);
    }
    return true;
  };
  auto read_v2 = [&](WinTestSock fd, std::vector<uint8_t>& buf, nve::WireFrameView& view) -> bool {
    std::array<uint8_t, nve::kWireHeaderSize> h{};
    if (!recv_raw(fd, h.data(), h.size())) return false;
    nve::WireHeader hdr{};
    if (!nve::wire_v2::decode_header(h, hdr)) return false;
    buf.resize(hdr.total_wire_len());
    std::memcpy(buf.data(), h.data(), h.size());
    if (buf.size() > h.size() && !recv_raw(fd, buf.data() + h.size(), buf.size() - h.size())) {
      return false;
    }
    return nve::wire_v2::decode_frame(buf, view);
  };

  std::thread srv([&]() {
    WinTestSock cfd = ::accept(listen_fd, nullptr, nullptr);
    NVE_CHECK(win_test_sock_valid(cfd));
    std::vector<uint8_t> msg;
    nve::WireFrameView view{};
    NVE_CHECK(read_v2(cfd, msg, view) && view.header.kind == nve::WireKind::Hello);
    nve::WireHello hp{};
    NVE_CHECK(nve::wire_v2::decode_hello(view.payload, hp));
    NVE_CHECK(nve::wire_v2::constant_time_eq(hp.bearer_token, token));

    const auto ack = nve::wire_v2::encode_hello_ack({42u, nve::kDefaultWireSchemaVersion, 0x0Fu});
    NVE_CHECK(send_raw(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::Control, nve::WireKind::HelloAck, 1u, ack)));
    NVE_CHECK(send_raw(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 1u, frame1)));
    NVE_CHECK(send_raw(
        cfd, nve::wire_v2::encode_frame(nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 2u, frame2)));

    while (read_v2(cfd, msg, view)) {
      if (view.header.kind == nve::WireKind::EventBatch) {
        std::lock_guard<std::mutex> lock(ev_mtx);
        rx_ev.assign(view.payload.begin(), view.payload.end());
        saw_input.store(true);
        break;
      }
      if (view.header.kind == nve::WireKind::Bye) break;
    }
    win_test_close(cfd);
    win_test_close(listen_fd);
  });

  nve::win::NveRnwComponentHost host;
  std::atomic<uint32_t> wake_hits{0u};
  std::atomic<uint32_t> tree_changes{0u};
  host.set_on_visual_tree_changed([&]() { tree_changes.fetch_add(1u, std::memory_order_relaxed); });

  const NveSurfaceConfig scfg{.surface_id = 1u, .width = 400.0f, .height = 280.0f, .scale_factor = 1.0f};
  std::string err;
  const std::string endpoint = "127.0.0.1:" + std::to_string(port);
  NVE_CHECK(host.connect_paired(
      endpoint,
      token,
      "auto",
      scfg,
      [&]() { wake_hits.fetch_add(1u, std::memory_order_relaxed); },
      &err));
  NVE_CHECK(host.is_paired() && host.is_paired_connected() && !host.is_valid());

  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (host.paired_frame_count() < 2u && std::chrono::steady_clock::now() < deadline) {
    (void)host.poll_paired_frames();
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(host.paired_frame_count() == 2u && wake_hits.load() >= 1u && tree_changes.load() >= 1u);

  const auto* label = host.visual_tree().find(3u);
  NVE_CHECK(label != nullptr && label->text == "Windows Paired Frame 2");
  const auto btn_rect = host.visual_tree().absolute_rect(4u);
  NVE_CHECK(btn_rect.has_value() && btn_rect->width > 10.0f && btn_rect->height > 10.0f);

  NVE_CHECK(host.dispatch_click_at(
                btn_rect->x + btn_rect->width * 0.5f, btn_rect->y + btn_rect->height * 0.5f) == 1u);

  const auto ev_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (!saw_input.load() && std::chrono::steady_clock::now() < ev_deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(saw_input.load());

  std::vector<nve::DecodedEvent> decoded;
  {
    std::lock_guard<std::mutex> lock(ev_mtx);
    NVE_CHECK(nve::decode_event_ring(NveBufferSlice{rx_ev.data(), rx_ev.size()}, decoded));
  }
  NVE_CHECK(decoded.size() == 1u && decoded[0].kind == nve::EventKind::Press && decoded[0].node_id == 4u);

  host.disconnect_paired();
  NVE_CHECK(!host.is_paired() && host.visual_tree().empty());
  srv.join();
#if defined(_WIN32)
  ::WSACleanup();
#endif
}

void test_multi_window_manager_lifecycle_modal_veto_and_cascade() {
  nve::win::NveWinWindowManager mgr;
  bool allow_primary = false;
  std::vector<uint32_t> closed_order;
  int all_closed_calls = 0;

  mgr.set_should_close_callback([&](uint32_t sid) { return sid == 1u ? allow_primary : true; });
  mgr.set_on_window_closed_callback([&](uint32_t sid) { closed_order.push_back(sid); });
  mgr.set_on_all_windows_closed_callback([&]() { ++all_closed_calls; });

  auto* h1 = mgr.open_headless_window({.surface_id = 1u, .title = "Primary", .width = 640, .height = 420});
  auto* h2 = mgr.open_headless_window({.surface_id = 2u, .title = "Inspector", .width = 360, .height = 300});
  NVE_CHECK(h1 != nullptr && h2 != nullptr && h1 != h2);
  NVE_CHECK(mgr.window_count() == 2u && mgr.focused_surface_id() == 2u);
  NVE_CHECK(mgr.open_headless_window({.surface_id = 1u, .title = "Dup"}) == nullptr);

  NVE_CHECK(mgr.set_title(1u, "Primary — Renamed"));
  NVE_CHECK(mgr.spec_for_surface(1u)->title == "Primary — Renamed");
  NVE_CHECK(mgr.set_bounds(1u, 40, 50, 800, 500));
  NVE_CHECK(mgr.spec_for_surface(1u)->width == 800 && mgr.spec_for_surface(1u)->height == 500);

  auto* h3 = mgr.open_headless_window({
      .surface_id = 3u,
      .title = "Modal Dialog",
      .width = 320,
      .height = 180,
      .parent_surface_id = 1u,
      .modal = true,
      .cascade_close = true,
  });
  auto* h4 = mgr.open_headless_window({
      .surface_id = 4u,
      .title = "Popover",
      .width = 200,
      .height = 120,
      .parent_surface_id = 3u,
      .cascade_close = true,
  });
  NVE_CHECK(h3 != nullptr && h4 != nullptr && mgr.active_modal_blocker(1u) == 3u);
  NVE_CHECK(!mgr.focus_window(1u) && mgr.focus_window(2u) && mgr.focused_surface_id() == 2u);

  NVE_CHECK(!mgr.request_close(1u));
  NVE_CHECK(mgr.window_count() == 4u && !mgr.quit_posted());

  allow_primary = true;
  NVE_CHECK(mgr.request_close(1u));
  NVE_CHECK(mgr.window_count() == 1u && !mgr.quit_posted() && all_closed_calls == 0);
  NVE_CHECK((closed_order == std::vector<uint32_t>{4u, 3u, 1u}));

  NVE_CHECK(mgr.request_close(2u));
  NVE_CHECK(mgr.window_count() == 0u && mgr.quit_posted() && all_closed_calls == 1);
}

} // namespace

void nve_test_win_canvas_pair_and_wm() {
  test_draw_list_visual_tree_extraction_and_corrupt_survival();
  test_paired_thin_client_loopback_roundtrip();
  test_multi_window_manager_lifecycle_modal_veto_and_cascade();
}
