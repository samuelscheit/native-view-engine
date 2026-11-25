#include "nve_abi.h"
#include "nve_rn_component.hpp"
#include "command_decoder.hpp"
#include "mock_jsi_recorder.hpp"

#include <cmath>
#include <cstdlib>
#include <cstring>
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

#undef assert
#define assert(expr) NVE_CHECK(expr)

namespace {

bool approx_eq(float a, float b, float eps = 0.5f) {
  return std::fabs(a - b) <= eps;
}

class CommandBufferEncoder {
 public:
  CommandBufferEncoder(uint32_t surface_id, uint64_t frame_id) {
    push<uint32_t>(NVE_ABI_MAGIC);
    push<uint16_t>(NVE_ABI_VERSION);
    push<uint16_t>(0);
    push<uint32_t>(surface_id);
    push<uint64_t>(frame_id);
    push<uint32_t>(0); /* op_count patched in finish() */
  }

  void create_node(uint32_t id, uint16_t kind, std::string_view custom_name = {}) {
    push<uint8_t>(NVE_OP_CREATE_NODE);
    push<uint32_t>(id);
    push<uint16_t>(kind);
    push<uint16_t>(static_cast<uint16_t>(custom_name.size()));
    push_bytes(custom_name.data(), custom_name.size());
    ++op_count_;
  }

  void remove_node(uint32_t id) {
    push<uint8_t>(NVE_OP_REMOVE_NODE);
    push<uint32_t>(id);
    ++op_count_;
  }

  void set_children(uint32_t parent_id, const std::vector<uint32_t>& children) {
    push<uint8_t>(NVE_OP_SET_CHILDREN);
    push<uint32_t>(parent_id);
    push<uint32_t>(static_cast<uint32_t>(children.size()));
    for (uint32_t c : children) {
      push<uint32_t>(c);
    }
    ++op_count_;
  }

  void set_text(uint32_t id, std::string_view text) {
    push<uint8_t>(NVE_OP_SET_TEXT);
    push<uint32_t>(id);
    push<uint32_t>(static_cast<uint32_t>(text.size()));
    push_bytes(text.data(), text.size());
    ++op_count_;
  }

  void set_layout(uint32_t id, const nve::LayoutProps& l) {
    push<uint8_t>(NVE_OP_SET_LAYOUT);
    push<uint32_t>(id);
    push_dim(l.width);
    push_dim(l.height);
    push_dim(l.min_width);
    push_dim(l.min_height);
    push_dim(l.max_width);
    push_dim(l.max_height);
    push<uint8_t>(l.flex_direction);
    push<uint8_t>(l.justify_content);
    push<uint8_t>(l.align_items);
    push<float>(l.flex_grow);
    push<float>(l.flex_shrink);
    push<float>(l.gap);
    push_insets(l.padding);
    push_insets(l.margin);
    ++op_count_;
  }

  void set_style(uint32_t id, const nve::StyleProps& s) {
    push<uint8_t>(NVE_OP_SET_STYLE);
    push<uint32_t>(id);
    push<uint32_t>(s.background_rgba);
    push<uint32_t>(s.text_color_rgba);
    push<uint32_t>(s.border_color_rgba);
    push<float>(s.border_width);
    push<float>(s.border_radius);
    push<float>(s.opacity);
    push<float>(s.font_size);
    push<int32_t>(s.z_index);
    ++op_count_;
  }

  void set_raw_prop_null(uint32_t id, std::string_view key) {
    write_raw_prop_header(id, key, 0);
  }

  void set_raw_prop_bool(uint32_t id, std::string_view key, bool v) {
    write_raw_prop_header(id, key, 1);
    push<uint8_t>(v ? 1 : 0);
  }

  void set_raw_prop_i64(uint32_t id, std::string_view key, int64_t v) {
    write_raw_prop_header(id, key, 2);
    push<int64_t>(v);
  }

  void set_raw_prop_f64(uint32_t id, std::string_view key, double v) {
    write_raw_prop_header(id, key, 3);
    push<double>(v);
  }

  void set_raw_prop_string(uint32_t id, std::string_view key, std::string_view v) {
    write_raw_prop_header(id, key, 4);
    push<uint32_t>(static_cast<uint32_t>(v.size()));
    push_bytes(v.data(), v.size());
  }

  void set_raw_prop_bytes(uint32_t id, std::string_view key, const std::vector<uint8_t>& v) {
    write_raw_prop_header(id, key, 5);
    push<uint32_t>(static_cast<uint32_t>(v.size()));
    push_bytes(v.data(), v.size());
  }

  void set_event_mask(uint32_t id, uint64_t mask) {
    push<uint8_t>(NVE_OP_SET_EVENT_MASK);
    push<uint32_t>(id);
    push<uint64_t>(mask);
    ++op_count_;
  }

  void dispatch_command(uint32_t id, std::string_view name, const std::vector<uint8_t>& payload) {
    push<uint8_t>(NVE_OP_DISPATCH_COMMAND);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(name.size()));
    push_bytes(name.data(), name.size());
    push<uint32_t>(static_cast<uint32_t>(payload.size()));
    push_bytes(payload.data(), payload.size());
    ++op_count_;
  }

  void commit(bool sync_mount) {
    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(sync_mount ? 1 : 0);
    ++op_count_;
  }

  NveBufferSlice finish() {
    std::memcpy(buf_.data() + 20, &op_count_, sizeof(uint32_t));
    return NveBufferSlice{buf_.data(), buf_.size()};
  }

  std::vector<uint8_t>& raw() { return buf_; }

 private:
  template <typename T>
  void push(T value) {
    const auto* p = reinterpret_cast<const uint8_t*>(&value);
    buf_.insert(buf_.end(), p, p + sizeof(T));
  }

  void push_bytes(const void* data, size_t len) {
    if (len == 0 || data == nullptr) return;
    const auto* p = static_cast<const uint8_t*>(data);
    buf_.insert(buf_.end(), p, p + len);
  }

  void push_dim(const nve::Dimension& d) {
    push<uint8_t>(d.tag);
    push<float>(d.value);
  }

  void push_insets(const nve::EdgeInsets& e) {
    push<float>(e.top);
    push<float>(e.right);
    push<float>(e.bottom);
    push<float>(e.left);
  }

  void write_raw_prop_header(uint32_t id, std::string_view key, uint8_t tag) {
    push<uint8_t>(NVE_OP_SET_RAW_PROP);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(key.size()));
    push_bytes(key.data(), key.size());
    push<uint8_t>(tag);
    ++op_count_;
  }

  uint32_t op_count_{0};
  std::vector<uint8_t> buf_;
};

// 1. Test all 10 NveOpcode variants + custom component hooks + snapshot inspection.
void test_all_opcodes_and_custom_components() {
  NveSurfaceConfig cfg{.surface_id = 7, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);
  assert(nve_fabric_host_node_count(host) == 1);

  bool saw_custom_props = false;
  bool saw_custom_command = false;
  nve::fabric_host_register_custom_component(
      host,
      nve::CustomComponentDescriptor{
          .component_name = "RNSVGCircle",
          .on_update_props =
              [&](uint32_t node_id, const nve::RawPropMap& props) {
                assert(node_id == 4);
                assert(std::holds_alternative<std::nullptr_t>(props.at("nilProp")));
                assert(std::get<bool>(props.at("visible")) == true);
                assert(std::get<int64_t>(props.at("segments")) == 64);
                assert(std::fabs(std::get<double>(props.at("radius")) - 42.5) < 1e-6);
                assert(std::get<std::string>(props.at("fill")) == "#ff00aa");
                const auto& blob = std::get<std::vector<uint8_t>>(props.at("matrix"));
                assert(blob.size() == 3 && blob[0] == 1 && blob[2] == 3);
                saw_custom_props = true;
              },
          .on_dispatch_command =
              [&](uint32_t node_id, std::string_view cmd, NveBufferSlice payload) {
                assert(node_id == 4);
                assert(cmd == "animatePulse");
                assert(payload.len == 2 && payload.ptr[0] == 0xAA && payload.ptr[1] == 0x55);
                saw_custom_command = true;
              },
      });

  CommandBufferEncoder enc(7, 1);
  enc.create_node(2, NVE_COMP_VIEW);
  enc.create_node(3, NVE_COMP_TEXT);
  enc.create_node(4, NVE_COMP_CUSTOM, "RNSVGCircle");
  enc.create_node(5, NVE_COMP_VIEW);

  enc.set_children(NVE_ROOT_NODE_ID, {2});
  enc.set_children(2, {3, 4});
  enc.set_text(3, "Hello NVE Fabric");

  nve::LayoutProps container_layout{};
  container_layout.width = {1, 300.0f};
  container_layout.height = {1, 200.0f};
  container_layout.padding = {10.0f, 10.0f, 10.0f, 10.0f};
  container_layout.gap = 8.0f;
  enc.set_layout(2, container_layout);

  nve::StyleProps style{};
  style.background_rgba = 0xFF332211u;
  style.text_color_rgba = 0xFFFFFFFFu;
  style.border_color_rgba = 0xFF00FF00u;
  style.border_width = 2.0f;
  style.border_radius = 12.0f;
  style.opacity = 0.9f;
  style.font_size = 18.0f;
  style.z_index = 5;
  enc.set_style(2, style);

  enc.set_raw_prop_null(4, "nilProp");
  enc.set_raw_prop_bool(4, "visible", true);
  enc.set_raw_prop_i64(4, "segments", 64);
  enc.set_raw_prop_f64(4, "radius", 42.5);
  enc.set_raw_prop_string(4, "fill", "#ff00aa");
  enc.set_raw_prop_bytes(4, "matrix", {1, 2, 3});

  enc.set_event_mask(2, (1ULL << 0) | (1ULL << 6)); /* PRESS | LAYOUT */
  enc.dispatch_command(4, "animatePulse", {0xAA, 0x55});
  enc.remove_node(5);
  enc.commit(true);

  NveFrameStats stats{};
  const int32_t rc = nve_fabric_host_submit_frame(host, enc.finish(), &stats);
  assert(rc == 0);
  assert(stats.op_count == 19);
  assert(stats.mutation_count == 17); /* all ops except DispatchCommand and Commit */
  assert(nve_fabric_host_node_count(host) == 4); /* root + 2 + 3 + 4 (5 removed) */
  assert(saw_custom_props);
  assert(saw_custom_command);

  // Node 2 subscribed to EventMask::LAYOUT, so commit should have emitted a Layout event!
  const NveBufferSlice ev_slice = nve_fabric_host_drain_events(host);
  std::vector<nve::DecodedEvent> events;
  assert(nve::decode_event_ring(ev_slice, events));
  assert(events.size() == 1);
  assert(events[0].kind == nve::EventKind::Layout);
  assert(events[0].node_id == 2);
  assert(approx_eq(events[0].width, 300.0f));
  assert(approx_eq(events[0].height, 200.0f));

  // Snapshot check
  const auto snaps = nve::fabric_host_snapshot_nodes(host);
  assert(snaps.size() == 4);

  nve_fabric_host_destroy(host);
}

// 2. Test corrupt buffer error codes thoroughly.
void test_corrupt_buffer_error_codes() {
  NveSurfaceConfig cfg{.surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);

  // Null host
  assert(nve_fabric_host_submit_frame(nullptr, NveBufferSlice{nullptr, 0}, nullptr) == -100);

  // Truncated header (< 24 bytes)
  const uint8_t short_buf[8] = {0};
  assert(nve_fabric_host_submit_frame(host, NveBufferSlice{short_buf, sizeof(short_buf)}, nullptr) == -1);

  // Null pointer with non-zero len
  assert(nve_fabric_host_submit_frame(host, NveBufferSlice{nullptr, 24}, nullptr) == -1);

  // Bad magic -> -2
  {
    CommandBufferEncoder enc(1, 1);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[0] = 0xDE;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -2);
  }

  // Bad version -> -2
  {
    CommandBufferEncoder enc(1, 1);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[4] = 99;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -2);
  }

  // Missing opcode byte (op_count=1 but 0 payload bytes) -> -3
  {
    CommandBufferEncoder enc(1, 1);
    enc.finish();
    uint32_t one = 1;
    std::memcpy(enc.raw().data() + 20, &one, 4);
    assert(nve_fabric_host_submit_frame(
               host, NveBufferSlice{enc.raw().data(), enc.raw().size()}, nullptr) == -3);
  }

  // Truncated op payload -> -4
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(2, NVE_COMP_VIEW);
    auto slice = enc.finish();
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{slice.ptr, slice.len - 2}, nullptr) == -4);
  }

  // Unknown target node id -> -5
  {
    CommandBufferEncoder enc(1, 1);
    enc.set_text(999, "missing");
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -5);
  }

  // Removing root node -> -5
  {
    CommandBufferEncoder enc(1, 1);
    enc.remove_node(NVE_ROOT_NODE_ID);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -5);
  }

  // Invalid raw prop tag -> -6
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(10, NVE_COMP_VIEW);
    enc.set_raw_prop_null(10, "k");
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad.back() = 99; /* corrupt tag byte */
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -6);
  }

  // Invalid ComponentKind -> -6
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(11, 0x1234);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -6);
  }

  // Unknown opcode -> -7
  {
    CommandBufferEncoder enc(1, 1);
    enc.commit(false);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[24] = 250;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -7);
  }

  // Trailing garbage bytes -> -8
  {
    CommandBufferEncoder enc(1, 1);
    enc.commit(false);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad.push_back(0x42);
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -8);
  }

  nve_fabric_host_destroy(host);
}

// 3. Test real Yoga Flexbox layout (and standalone fallback layout)!
void test_flexbox_layout() {
  NveSurfaceConfig cfg{.surface_id = 1, .width = 400.0f, .height = 200.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);

  // Build a Row container (width=400, height=200, padding=20, gap=10) with:
  //   - child 10: fixed width=100, height=80
  //   - child 11: flex_grow=1, height=80   -> remaining = 400 - 40(pad) - 100 - 20(2*gap) = 240; grow 1/(1+2) = 80
  //   - child 12: flex_grow=2, height=100% -> grow 2/(1+2) = 160, height = 160 (200 - 40 pad)
  CommandBufferEncoder enc(1, 1);
  enc.create_node(2, NVE_COMP_VIEW);
  enc.create_node(10, NVE_COMP_VIEW);
  enc.create_node(11, NVE_COMP_VIEW);
  enc.create_node(12, NVE_COMP_VIEW);

  enc.set_children(NVE_ROOT_NODE_ID, {2});
  enc.set_children(2, {10, 11, 12});

  nve::LayoutProps row{};
  row.width = {1, 400.0f};
  row.height = {1, 200.0f};
  row.flex_direction = 1; /* Row */
  row.padding = {20.0f, 20.0f, 20.0f, 20.0f};
  row.gap = 10.0f;
  enc.set_layout(2, row);

  nve::LayoutProps c10{};
  c10.width = {1, 100.0f};
  c10.height = {1, 80.0f};
  enc.set_layout(10, c10);

  nve::LayoutProps c11{};
  c11.height = {1, 80.0f};
  c11.flex_grow = 1.0f;
  enc.set_layout(11, c11);

  nve::LayoutProps c12{};
  c12.height = {2, 100.0f}; /* 100% of container inner height (160) */
  c12.flex_grow = 2.0f;
  enc.set_layout(12, c12);

  enc.commit(true);

  NveFrameStats stats{};
  assert(nve_fabric_host_submit_frame(host, enc.finish(), &stats) == 0);

  nve::LayoutRect r2{}, r10{}, r11{}, r12{};
  assert(nve::fabric_host_get_node_layout(host, 2, &r2));
  assert(nve::fabric_host_get_node_layout(host, 10, &r10));
  assert(nve::fabric_host_get_node_layout(host, 11, &r11));
  assert(nve::fabric_host_get_node_layout(host, 12, &r12));

  assert(approx_eq(r2.x, 0.0f) && approx_eq(r2.y, 0.0f));
  assert(approx_eq(r2.width, 400.0f) && approx_eq(r2.height, 200.0f));

  assert(approx_eq(r10.x, 20.0f) && approx_eq(r10.y, 20.0f));
  assert(approx_eq(r10.width, 100.0f) && approx_eq(r10.height, 80.0f));

  assert(approx_eq(r11.x, 130.0f) && approx_eq(r11.y, 20.0f));
  assert(approx_eq(r11.width, 80.0f) && approx_eq(r11.height, 80.0f));

  assert(approx_eq(r12.x, 220.0f) && approx_eq(r12.y, 20.0f));
  assert(approx_eq(r12.width, 160.0f) && approx_eq(r12.height, 160.0f));

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  assert(nve::fabric_host_has_yoga_layout() == true);

  // Additional Yoga-specific test: justifyContent=SpaceBetween + alignItems=Center + RowReverse + Text measure
  CommandBufferEncoder enc2(1, 2);
  enc2.create_node(20, NVE_COMP_VIEW);
  enc2.create_node(21, NVE_COMP_VIEW);
  enc2.create_node(22, NVE_COMP_TEXT);
  enc2.set_children(NVE_ROOT_NODE_ID, {20});
  enc2.set_children(20, {21, 22});
  enc2.set_text(22, "YogaText");

  nve::LayoutProps sb{};
  sb.width = {1, 300.0f};
  sb.height = {1, 100.0f};
  sb.flex_direction = 1; /* Row */
  sb.justify_content = 3; /* SpaceBetween */
  sb.align_items = 2; /* Center */
  enc2.set_layout(20, sb);

  nve::LayoutProps b21{};
  b21.width = {1, 60.0f};
  b21.height = {1, 40.0f};
  enc2.set_layout(21, b21);

  enc2.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc2.finish(), &stats) == 0);

  nve::LayoutRect r21{}, r22{};
  assert(nve::fabric_host_get_node_layout(host, 21, &r21));
  assert(nve::fabric_host_get_node_layout(host, 22, &r22));
  assert(approx_eq(r21.x, 0.0f));
  assert(approx_eq(r21.y, 30.0f)); /* centered in 100px height: (100-40)/2 = 30 */
  assert(r22.width > 10.0f && r22.height > 10.0f); /* measured by Yoga text measure func */
  assert(approx_eq(r22.x + r22.width, 300.0f)); /* SpaceBetween pushes 2nd item flush right */
#else
  assert(nve::fabric_host_has_yoga_layout() == false);
#endif

  nve_fabric_host_destroy(host);
}

// 4. Test NveRnComponentBridge end-to-end (set_props, render, native events, drain_emitted_events, snapshot/restore).
struct MockEmbeddedRustComponent {
  uint32_t surface_id{1};
  std::string last_props_json;
  int count{0};
  std::vector<uint8_t> last_cmd_buf;
  nve::EventRingWriter outbound_events;
  std::vector<uint8_t> outbound_snapshot;
  std::string saved_state_buf;

  static NveAppHandle create(uint32_t sid) {
    auto* self = new MockEmbeddedRustComponent();
    self->surface_id = sid;
    self->outbound_events.clear();
    return self;
  }

  static void destroy(NveAppHandle h) {
    delete static_cast<MockEmbeddedRustComponent*>(h);
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    CommandBufferEncoder enc(self->surface_id, frame_id);
    enc.create_node(2, NVE_COMP_PRESSABLE);
    enc.create_node(3, NVE_COMP_TEXT);
    enc.set_children(NVE_ROOT_NODE_ID, {2});
    enc.set_children(2, {3});
    enc.set_text(3, "Count: " + std::to_string(self->count));
    nve::LayoutProps lp{};
    lp.width = {1, 200.0f};
    lp.height = {1, 60.0f};
    enc.set_layout(2, lp);
    enc.commit(true);
    auto slice = enc.finish();
    self->last_cmd_buf.assign(slice.ptr, slice.ptr + slice.len);
    return NveBufferSlice{self->last_cmd_buf.data(), self->last_cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    uint32_t dirty = 0;
    for (const auto& ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 2) {
        self->count += 1;
        const std::string json = "{\"count\":" + std::to_string(self->count) + "}";
        self->outbound_events.push_custom(
            self->surface_id,
            2,
            ev.timestamp_us,
            "onIncrement",
            reinterpret_cast<const uint8_t*>(json.data()),
            json.size());
        dirty = 1;
      }
    }
    return dirty;
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch_json) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->last_props_json.assign(
        reinterpret_cast<const char*>(patch_json.ptr), patch_json.len);
    return 1;
  }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->saved_state_buf = std::to_string(self->count);
    return NveBufferSlice{
        reinterpret_cast<const uint8_t*>(self->saved_state_buf.data()),
        self->saved_state_buf.size()};
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    std::string s(reinterpret_cast<const char*>(state.ptr), state.len);
    self->count = std::stoi(s);
  }

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->last_props_json.assign(reinterpret_cast<const char*>(props.ptr), props.len);
    const std::string payload = "{\"accepted\":true}";
    self->outbound_events.push_custom(
        self->surface_id,
        NVE_ROOT_NODE_ID,
        1000,
        "onPropsApplied",
        reinterpret_cast<const uint8_t*>(payload.data()),
        payload.size());
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    self->outbound_snapshot = self->outbound_events.finalize();
    self->outbound_events.clear();
    return NveBufferSlice{self->outbound_snapshot.data(), self->outbound_snapshot.size()};
  }

  static NveIntrinsicSize measure(NveAppHandle, NveSizeConstraints c) {
    return NveIntrinsicSize{
        .width = c.max_w > 0.0f ? (c.max_w < 200.0f ? c.max_w : 200.0f) : 200.0f,
        .height = 60.0f,
    };
  }

  static uint8_t hit_test(NveAppHandle, float, float, uint8_t) {
    return NVE_GESTURE_CLAIM;
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
    };
    static const NveLayoutApi kLayout{
        .measure = &measure,
        .hit_test = &hit_test,
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
      case NVE_IFACE_LAYOUT: return &kLayout;
      case NVE_IFACE_COMPONENT: return &kComponent;
      case NVE_IFACE_DEV: return &kDev;
      default: return nullptr;
    }
  }
};

void test_rn_component_bridge() {
  std::vector<nve::DecodedEvent> emitted_via_cb;
  NveSurfaceConfig cfg{.surface_id = 42, .width = 320.0f, .height = 120.0f, .scale_factor = 3.0f};
  nve::NveRnComponentBridge bridge(
      &MockEmbeddedRustComponent::get_interface,
      cfg,
      [&](const nve::DecodedEvent& ev) { emitted_via_cb.push_back(ev); });

  assert(bridge.is_valid());
  auto measured = bridge.measure(NveSizeConstraints{0.0f, 0.0f, 320.0f, 120.0f});
  assert(measured.has_value() && approx_eq(measured->width, 200.0f) && approx_eq(measured->height, 60.0f));
  assert(bridge.hit_test(10.0f, 10.0f, 1) == NVE_GESTURE_CLAIM);

  // 1. Update props via typed PropMap -> JSON -> vtable.set_props -> auto-render
  nve::PropMap props;
  props["title"] = std::string("Hello \"RN\"");
  props["initialCount"] = static_cast<int64_t>(5);
  props["enabled"] = true;
  props["ratio"] = 0.75;
  props["optional"] = nullptr;

  const std::string serialized = nve::serialize_props_json(props);
  assert(serialized.find("\"title\":\"Hello \\\"RN\\\"\"") != std::string::npos);
  assert(serialized.find("\"initialCount\":5") != std::string::npos);
  assert(serialized.find("\"enabled\":true") != std::string::npos);
  assert(serialized.find("\"optional\":null") != std::string::npos);

  assert(bridge.update_props(props, /*auto_render=*/true));
  assert(bridge.frame_count() == 1);
  assert(bridge.node_count() == 3);

  auto rect = bridge.node_layout(2);
  assert(rect.has_value());
  assert(approx_eq(rect->width, 200.0f) && approx_eq(rect->height, 60.0f));

  auto evs = bridge.drain_emitted_events();
  assert(evs.size() == 1);
  assert(evs[0].kind == nve::EventKind::Custom);
  assert(evs[0].name == "onPropsApplied");
  assert(evs[0].payload_as_string() == "{\"accepted\":true}");
  assert(emitted_via_cb.size() == 1);

  // 2. Simulate native press -> dispatch -> outbound onIncrement event
  nve::fabric_host_enqueue_press(bridge.fabric_host(), 2, 99999, 15.0f, 25.0f);
  const NveBufferSlice inbound = nve_fabric_host_drain_events(bridge.fabric_host());
  assert(bridge.dispatch_native_events(inbound, /*auto_render=*/true) == 1);
  assert(bridge.frame_count() == 2);

  auto evs2 = bridge.drain_emitted_events();
  assert(evs2.size() == 1);
  assert(evs2[0].name == "onIncrement");
  assert(evs2[0].payload_as_string() == "{\"count\":1}");
  assert(emitted_via_cb.size() == 2);

  // 3. Snapshot and restore state
  const auto snap = bridge.snapshot_state();
  assert(snap.size() == 1 && snap[0] == '1');
  const std::string next_state = "7";
  bridge.restore_state(
      NveBufferSlice{reinterpret_cast<const uint8_t*>(next_state.data()), next_state.size()},
      /*auto_render=*/true);
  assert(bridge.frame_count() == 3);
  const auto snap2 = bridge.snapshot_state();
  assert(snap2.size() == 1 && snap2[0] == '7');
}

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
// 5. Test real facebook::jsi::Runtime subclass (MockJsiRecorderRuntime) with zero Hermes!
void test_mock_jsi_recorder_runtime() {
  nve::MockJsiRecorderRuntime rt;
  facebook::jsi::Object root(rt);
  root.setProperty(rt, "event", "onSvgLoad");
  root.setProperty(rt, "width", 128.5);
  root.setProperty(rt, "animated", true);
  root.setProperty(rt, "tag", nullptr);

  facebook::jsi::Array pts(rt, 2);
  pts.setValueAtIndex(rt, 0, 10);
  pts.setValueAtIndex(rt, 1, 20);
  root.setProperty(rt, "points", std::move(pts));

  nve::EventRingWriter ring;
  ring.clear();
  rt.record_custom_event(ring, 1, 77, 123456, "onSvgLoad", facebook::jsi::Value(rt, root));

  const auto& bytes = ring.finalize();
  std::vector<nve::DecodedEvent> decoded;
  assert(nve::decode_event_ring(NveBufferSlice{bytes.data(), bytes.size()}, decoded));
  assert(decoded.size() == 1);
  assert(decoded[0].kind == nve::EventKind::Custom);
  assert(decoded[0].node_id == 77);
  assert(decoded[0].name == "onSvgLoad");
  const std::string_view json = decoded[0].payload_as_string();
  assert(json.find("\"event\":\"onSvgLoad\"") != std::string_view::npos);
  assert(json.find("\"width\":128.5") != std::string_view::npos);
  assert(json.find("\"animated\":true") != std::string_view::npos);
  assert(json.find("\"tag\":null") != std::string_view::npos);
  assert(json.find("\"points\":[10,20]") != std::string_view::npos);
}
#endif

} // namespace

int main() {
  test_all_opcodes_and_custom_components();
  test_corrupt_buffer_error_codes();
  test_flexbox_layout();
  test_rn_component_bridge();
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  test_mock_jsi_recorder_runtime();
#endif

  std::cout << "nve_fabric_bridge_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI" : "Standalone")
            << " mode)\n";
  return 0;
}
