#include "nve_abi.h"
#include "nve_host_runtime.hpp"
#include "nve_rn_component.hpp"
#include "command_decoder.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"

#include <cmath>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <limits>
#include <random>
#include <string>
#include <unordered_map>
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

using nve::CommandBufferEncoder;

bool approx_eq(float a, float b, float eps = 0.5f) {
  return std::fabs(a - b) <= eps;
}

std::unordered_map<uint32_t, nve::RenderedNodeSnapshot> snapshot_by_id(NveFabricHostHandle host) {
  std::unordered_map<uint32_t, nve::RenderedNodeSnapshot> map;
  for (auto& snap : nve::fabric_host_snapshot_nodes(host)) {
    map.emplace(snap.node_id, std::move(snap));
  }
  return map;
}

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
#else
  assert(nve::fabric_host_has_yoga_layout() == false);
#endif

  // Verify justifyContent=SpaceBetween + alignItems=Center + Text measure in BOTH Yoga & Standalone modes.
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
  assert(r22.width > 10.0f && r22.height > 10.0f); /* measured text */
  assert(approx_eq(r22.x + r22.width, 300.0f)); /* SpaceBetween pushes 2nd item flush right */
  assert(approx_eq(r22.y + r22.height * 0.5f, 50.0f)); /* vertically centered */

  // Verify auto-sized flex-column card (nested Text wrappers) + centered button labels + reverse/end/evenly.
  {
    NveSurfaceConfig card_cfg{.surface_id = 2, .width = 640.0f, .height = 420.0f, .scale_factor = 2.0f};
    NveFabricHostHandle card_host = nve_fabric_host_create(card_cfg);
    CommandBufferEncoder ce(2, 1);

    // Outer page: 100% x 100%, padding=24, gap=16
    ce.create_node(2, NVE_COMP_VIEW);
    // Card: width=100%, height=auto, padding=20, gap=8
    ce.create_node(3, NVE_COMP_VIEW);
    ce.create_node(4, NVE_COMP_TEXT); /* wrapper */
    ce.create_node(5, NVE_COMP_TEXT); /* leaf "NATIVE VIEW ENGINE" */
    ce.create_node(6, NVE_COMP_TEXT); /* wrapper */
    ce.create_node(7, NVE_COMP_TEXT); /* leaf "Count: 0" */
    // Button row: width=100%, height=auto, row, gap=12
    ce.create_node(8, NVE_COMP_VIEW);
    ce.create_node(9, NVE_COMP_PRESSABLE); /* 160x48 center/center */
    ce.create_node(10, NVE_COMP_TEXT); /* wrapper */
    ce.create_node(11, NVE_COMP_TEXT); /* leaf "Increment +" */
    ce.create_node(12, NVE_COMP_PRESSABLE); /* 120x48 center/center */
    ce.create_node(13, NVE_COMP_TEXT); /* wrapper */
    ce.create_node(14, NVE_COMP_TEXT); /* leaf "Reset" */

    ce.set_children(NVE_ROOT_NODE_ID, {2});
    ce.set_children(2, {3, 8});
    ce.set_children(3, {4, 6});
    ce.set_children(4, {5});
    ce.set_children(6, {7});
    ce.set_children(8, {9, 12});
    ce.set_children(9, {10});
    ce.set_children(10, {11});
    ce.set_children(12, {13});
    ce.set_children(13, {14});

    ce.set_text(5, "NATIVE VIEW ENGINE");
    ce.set_text(7, "Count: 0");
    ce.set_text(11, "Increment +");
    ce.set_text(14, "Reset");

    nve::StyleProps st14{};
    st14.font_size = 14.0f;
    ce.set_style(4, st14);
    nve::StyleProps st28{};
    st28.font_size = 28.0f;
    ce.set_style(6, st28);
    nve::StyleProps st16{};
    st16.font_size = 16.0f;
    ce.set_style(10, st16);
    ce.set_style(13, st16);

    nve::LayoutProps page_lp{};
    page_lp.width = {2, 100.0f};
    page_lp.height = {2, 100.0f};
    page_lp.padding = {24.0f, 24.0f, 24.0f, 24.0f};
    page_lp.gap = 16.0f;
    ce.set_layout(2, page_lp);

    nve::LayoutProps card_lp{};
    card_lp.width = {2, 100.0f};
    card_lp.padding = {20.0f, 20.0f, 20.0f, 20.0f};
    card_lp.gap = 8.0f;
    ce.set_layout(3, card_lp);

    nve::LayoutProps row_lp{};
    row_lp.width = {2, 100.0f};
    row_lp.flex_direction = 1; /* Row */
    row_lp.gap = 12.0f;
    ce.set_layout(8, row_lp);

    nve::LayoutProps btn1_lp{};
    btn1_lp.width = {1, 160.0f};
    btn1_lp.height = {1, 48.0f};
    btn1_lp.justify_content = 1; /* Center */
    btn1_lp.align_items = 2; /* Center */
    ce.set_layout(9, btn1_lp);

    nve::LayoutProps btn2_lp = btn1_lp;
    btn2_lp.width = {1, 120.0f};
    ce.set_layout(12, btn2_lp);

    ce.commit(true);
    assert(nve_fabric_host_submit_frame(card_host, ce.finish(), nullptr) == 0);

    nve::LayoutRect rc_card{}, rc_t1{}, rc_t2{}, rc_row{}, rc_b1{}, rc_b1w{}, rc_b1l{}, rc_b2{}, rc_b2w{}, rc_b2l{};
    assert(nve::fabric_host_get_node_layout(card_host, 3, &rc_card));
    assert(nve::fabric_host_get_node_layout(card_host, 4, &rc_t1));
    assert(nve::fabric_host_get_node_layout(card_host, 6, &rc_t2));
    assert(nve::fabric_host_get_node_layout(card_host, 8, &rc_row));
    assert(nve::fabric_host_get_node_layout(card_host, 9, &rc_b1));
    assert(nve::fabric_host_get_node_layout(card_host, 10, &rc_b1w));
    assert(nve::fabric_host_get_node_layout(card_host, 11, &rc_b1l));
    assert(nve::fabric_host_get_node_layout(card_host, 12, &rc_b2));
    assert(nve::fabric_host_get_node_layout(card_host, 13, &rc_b2w));
    assert(nve::fabric_host_get_node_layout(card_host, 14, &rc_b2l));

    // Card must auto-size to wrap both texts + 40px vertical padding + 8px gap
    assert(approx_eq(rc_card.width, 592.0f));
    assert(rc_card.height > 80.0f);
    assert(approx_eq(rc_t1.y, 20.0f));
    assert(approx_eq(rc_t2.y, rc_t1.y + rc_t1.height + 8.0f));
    assert(approx_eq(rc_card.height, rc_t2.y + rc_t2.height + 20.0f, 1.5f));

    // Button row sits below card by 16px gap and auto-sizes to 48px height
    assert(approx_eq(rc_row.y, rc_card.y + rc_card.height + 16.0f, 1.5f));
    assert(approx_eq(rc_row.height, 48.0f));
    assert(approx_eq(rc_b1.x, 0.0f) && approx_eq(rc_b1.width, 160.0f) && approx_eq(rc_b1.height, 48.0f));
    assert(approx_eq(rc_b2.x, 172.0f) && approx_eq(rc_b2.width, 120.0f) && approx_eq(rc_b2.height, 48.0f));

    // Button labels must be centered horizontally & vertically inside each button
    const float b1_cx = rc_b1w.x + rc_b1l.x + rc_b1l.width * 0.5f;
    const float b1_cy = rc_b1w.y + rc_b1l.y + rc_b1l.height * 0.5f;
    assert(approx_eq(b1_cx, 80.0f, 1.5f));
    assert(approx_eq(b1_cy, 24.0f, 1.5f));

    const float b2_cx = rc_b2w.x + rc_b2l.x + rc_b2l.width * 0.5f;
    const float b2_cy = rc_b2w.y + rc_b2l.y + rc_b2l.height * 0.5f;
    assert(approx_eq(b2_cx, 60.0f, 1.5f));
    assert(approx_eq(b2_cy, 24.0f, 1.5f));

    // Additional parity checks: RowReverse + FlexEnd + AlignEnd + flex_shrink + min/max bounds
    CommandBufferEncoder ce2(2, 2);
    ce2.create_node(30, NVE_COMP_VIEW); /* 200x100 RowReverse, FlexStart, FlexEnd */
    ce2.create_node(31, NVE_COMP_VIEW); /* 60x30 */
    ce2.create_node(32, NVE_COMP_VIEW); /* 40x20 */
    ce2.create_node(40, NVE_COMP_VIEW); /* 200x60 Row, SpaceEvenly, Center */
    ce2.create_node(41, NVE_COMP_VIEW); /* 40x20 */
    ce2.create_node(42, NVE_COMP_VIEW); /* 40x20 */
    ce2.create_node(50, NVE_COMP_VIEW); /* 100x40 Row, shrink test */
    ce2.create_node(51, NVE_COMP_VIEW); /* 80x40, shrink=1 */
    ce2.create_node(52, NVE_COMP_VIEW); /* 80x40, shrink=1 */
    ce2.set_children(NVE_ROOT_NODE_ID, {30, 40, 50});
    ce2.set_children(30, {31, 32});
    ce2.set_children(40, {41, 42});
    ce2.set_children(50, {51, 52});

    nve::LayoutProps rev_lp{};
    rev_lp.width = {1, 200.0f};
    rev_lp.height = {1, 100.0f};
    rev_lp.flex_direction = 3; /* RowReverse */
    rev_lp.align_items = 3; /* FlexEnd */
    rev_lp.gap = 10.0f;
    ce2.set_layout(30, rev_lp);
    nve::LayoutProps c31{};
    c31.width = {1, 60.0f};
    c31.height = {1, 30.0f};
    ce2.set_layout(31, c31);
    nve::LayoutProps c32{};
    c32.width = {1, 40.0f};
    c32.height = {1, 20.0f};
    ce2.set_layout(32, c32);

    nve::LayoutProps ev_lp{};
    ev_lp.width = {1, 200.0f};
    ev_lp.height = {1, 60.0f};
    ev_lp.flex_direction = 1; /* Row */
    ev_lp.justify_content = 5; /* SpaceEvenly: leftover=120 -> 3 gaps of 40 */
    ev_lp.align_items = 2; /* Center */
    ce2.set_layout(40, ev_lp);
    ce2.set_layout(41, c32);
    ce2.set_layout(42, c32);

    nve::LayoutProps sh_lp{};
    sh_lp.width = {1, 100.0f};
    sh_lp.height = {1, 40.0f};
    sh_lp.flex_direction = 1; /* Row */
    ce2.set_layout(50, sh_lp);
    nve::LayoutProps sh_child{};
    sh_child.width = {1, 80.0f};
    sh_child.height = {1, 40.0f};
    sh_child.flex_shrink = 1.0f;
    ce2.set_layout(51, sh_child);
    ce2.set_layout(52, sh_child);

    ce2.commit(true);
    assert(nve_fabric_host_submit_frame(card_host, ce2.finish(), nullptr) == 0);

    nve::LayoutRect r31{}, r32{}, r41{}, r42{}, r51{}, r52{};
    assert(nve::fabric_host_get_node_layout(card_host, 31, &r31));
    assert(nve::fabric_host_get_node_layout(card_host, 32, &r32));
    assert(nve::fabric_host_get_node_layout(card_host, 41, &r41));
    assert(nve::fabric_host_get_node_layout(card_host, 42, &r42));
    assert(nve::fabric_host_get_node_layout(card_host, 51, &r51));
    assert(nve::fabric_host_get_node_layout(card_host, 52, &r52));

    // RowReverse + FlexEnd: 31 at right edge (x=140, y=70), 32 to its left (x=90, y=80)
    assert(approx_eq(r31.x, 140.0f) && approx_eq(r31.y, 70.0f));
    assert(approx_eq(r32.x, 90.0f) && approx_eq(r32.y, 80.0f));
    // SpaceEvenly: 41 at x=40, y=20; 42 at x=120, y=20
    assert(approx_eq(r41.x, 40.0f) && approx_eq(r41.y, 20.0f));
    assert(approx_eq(r42.x, 120.0f) && approx_eq(r42.y, 20.0f));
    // flex_shrink: two 80px children shrink equally to 50px each inside 100px row
    assert(approx_eq(r51.width, 50.0f) && approx_eq(r52.width, 50.0f));

    nve_fabric_host_destroy(card_host);
  }

  nve_fabric_host_destroy(host);
}

// 4. Test NveRnComponentBridge end-to-end (set_props_direct, dispatch_events_direct, render, native events, drain_emitted_events, snapshot/restore).
struct MockEmbeddedRustComponent {
  uint32_t surface_id{1};
  std::string last_props_json;
  std::string last_direct_title;
  int64_t last_direct_initial_count{0};
  bool last_direct_enabled{false};
  double last_direct_ratio{0.0};
  bool saw_direct_null{false};
  uint32_t direct_props_calls{0};
  uint32_t direct_event_calls{0};
  std::string last_direct_event_name;
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
        const NveMapEntry entry{
            nve::make_str_slice("count"),
            nve::make_i64_value(static_cast<int64_t>(self->count)),
        };
        const NveValue payload = nve::make_map_value(&entry, 1);
        self->outbound_events.push_custom_value(
            self->surface_id, 2, ev.timestamp_us, "onIncrement", payload);
        dirty = 1;
      }
    }
    return dirty;
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    if (!self || !batch.ptr || batch.len == 0) return 0;
    uint32_t dirty = 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent& ev = batch.ptr[i];
      ++self->direct_event_calls;
      if (ev.kind == 1 && ev.node_id == 2) {
        self->count += 1;
        dirty = 1;
      } else if (ev.kind == 8) {
        self->last_direct_event_name = std::string(nve::str_slice_view(ev.name));
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
    const NveMapEntry entry{nve::make_str_slice("accepted"), nve::make_bool_value(true)};
    const NveValue payload = nve::make_map_value(&entry, 1);
    self->outbound_events.push_custom_value(
        self->surface_id, NVE_ROOT_NODE_ID, 1000, "onPropsApplied", payload);
    return 1;
  }

  static uint32_t set_props_direct(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<MockEmbeddedRustComponent*>(h);
    ++self->direct_props_calls;
    for (uint32_t i = 0; i < count; ++i) {
      const std::string_view k = nve::str_slice_view(entries[i].key);
      const NveValue& v = entries[i].value;
      if (k == "title" && v.tag == NVE_VAL_STR) {
        self->last_direct_title = std::string(nve::str_slice_view(v.data.str_val));
      } else if (k == "initialCount" && v.tag == NVE_VAL_I64) {
        self->last_direct_initial_count = v.data.i64_val;
      } else if (k == "enabled" && v.tag == NVE_VAL_BOOL) {
        self->last_direct_enabled = v.data.bool_val != 0;
      } else if (k == "ratio" && v.tag == NVE_VAL_F64) {
        self->last_direct_ratio = v.data.f64_val;
      } else if (k == "optional" && v.tag == NVE_VAL_NULL) {
        self->saw_direct_null = true;
      }
    }
    const NveMapEntry entry{nve::make_str_slice("accepted"), nve::make_bool_value(true)};
    const NveValue payload = nve::make_map_value(&entry, 1);
    self->outbound_events.push_custom_value(
        self->surface_id, NVE_ROOT_NODE_ID, 1000, "onPropsApplied", payload);
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
        .dispatch_events_direct = &dispatch_events_direct,
    };
    static const NveLayoutApi kLayout{
        .measure = &measure,
        .hit_test = &hit_test,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
        .set_props_direct = &set_props_direct,
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
  assert(bridge.has_direct_events());
  assert(bridge.has_direct_props());
  auto measured = bridge.measure(NveSizeConstraints{0.0f, 0.0f, 320.0f, 120.0f});
  assert(measured.has_value() && approx_eq(measured->width, 200.0f) && approx_eq(measured->height, 60.0f));
  assert(bridge.hit_test(10.0f, 10.0f, 1) == NVE_GESTURE_CLAIM);

  // 1. Update props via typed PropMap -> zero-copy BumpArena -> set_props_direct -> auto-render
  nve::PropMap props;
  props["title"] = std::string("Hello \"RN\"");
  props["initialCount"] = static_cast<int64_t>(5);
  props["enabled"] = true;
  props["ratio"] = 0.75;
  props["optional"] = nullptr;

  assert(bridge.update_props(props, /*auto_render=*/true));
  assert(bridge.frame_count() == 1);
  assert(bridge.node_count() == 3);

  auto* mock_app = static_cast<MockEmbeddedRustComponent*>(bridge.app_handle());
  assert(mock_app->direct_props_calls == 1);
  assert(mock_app->last_direct_title == "Hello \"RN\"");
  assert(mock_app->last_direct_initial_count == 5);
  assert(mock_app->last_direct_enabled == true);
  assert(std::fabs(mock_app->last_direct_ratio - 0.75) < 1e-9);
  assert(mock_app->saw_direct_null);

  auto rect = bridge.node_layout(2);
  assert(rect.has_value());
  assert(approx_eq(rect->width, 200.0f) && approx_eq(rect->height, 60.0f));

  auto evs = bridge.drain_emitted_events();
  assert(evs.size() == 1);
  assert(evs[0].kind == nve::EventKind::Custom);
  assert(evs[0].name == "onPropsApplied");
  const auto ev0_view = nve::BinValueView::parse(evs[0].payload_bytes.data(), evs[0].payload_bytes.size());
  assert(ev0_view.has_value() && ev0_view->tag() == NVE_VAL_MAP);
  assert(ev0_view->map_get("accepted")->as_bool() == true);
  assert(emitted_via_cb.size() == 1);

  // 2. Simulate native press -> dispatch -> outbound binary onIncrement event
  nve::fabric_host_enqueue_press(bridge.fabric_host(), 2, 99999, 15.0f, 25.0f);
  const NveBufferSlice inbound = nve_fabric_host_drain_events(bridge.fabric_host());
  assert(bridge.dispatch_native_events(inbound, /*auto_render=*/true) == 1);
  assert(bridge.frame_count() == 2);

  auto evs2 = bridge.drain_emitted_events();
  assert(evs2.size() == 1);
  assert(evs2[0].name == "onIncrement");
  const auto ev2_view = nve::BinValueView::parse(evs2[0].payload_bytes.data(), evs2[0].payload_bytes.size());
  assert(ev2_view.has_value() && ev2_view->map_get("count")->as_i64() == 1);
  assert(emitted_via_cb.size() == 2);

  // 2b. Zero-copy direct event dispatch via NveRnComponentBridge::dispatch_direct_events
  NveDirectEvent direct_ev{};
  direct_ev.surface_id = 42;
  direct_ev.node_id = 2;
  direct_ev.timestamp_us = 100500;
  direct_ev.kind = 8;
  direct_ev.payload_tag = 5;
  direct_ev.name = nve::make_str_slice("onDirectPinch");
  direct_ev.value = nve::make_f64_value(2.5);
  assert(bridge.dispatch_direct_events(NveEventBatch{&direct_ev, 1}, /*auto_render=*/true) == 1);
  assert(mock_app->direct_event_calls == 1);
  assert(mock_app->last_direct_event_name == "onDirectPinch");
  assert(bridge.frame_count() == 3);

  // 3. Snapshot and restore state
  const auto snap = bridge.snapshot_state();
  assert(snap.size() == 1 && snap[0] == '1');
  const std::string next_state = "7";
  bridge.restore_state(
      NveBufferSlice{reinterpret_cast<const uint8_t*>(next_state.data()), next_state.size()},
      /*auto_render=*/true);
  assert(bridge.frame_count() == 4);
  const auto snap2 = bridge.snapshot_state();
  assert(snap2.size() == 1 && snap2[0] == '7');
}

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
// 5. Test real facebook::jsi::Runtime subclass (MockJsiRecorderRuntime):
//    zero-copy BumpArena direct FFI path, binary EventRing path, zero-malloc multi-frame reuse,
//    post-assignment mutation sync, BigInt, ArrayBuffer, and cyclic object depth guard.
void test_mock_jsi_recorder_runtime() {
  nve::MockJsiRecorderRuntime rt;

  struct RawBuffer final : public facebook::jsi::MutableBuffer {
    std::vector<uint8_t> bytes{0xCA, 0xFE, 0xBA, 0xBE};
    size_t size() const override { return bytes.size(); }
    uint8_t* data() override { return bytes.data(); }
  };

  // (a) Direct arena FFI path + zero-copy pointer identity + BigInt + ArrayBuffer + post-assignment mutation
  {
    facebook::jsi::Object root(rt);
    root.setProperty(rt, "event", "onSvgLoad");
    root.setProperty(rt, "width", 128.5);
    root.setProperty(rt, "animated", true);
    root.setProperty(rt, "tag", nullptr);
    root.setProperty(rt, "big", facebook::jsi::BigInt::fromInt64(rt, -9007199254740993LL));
    root.setProperty(rt, "blob", facebook::jsi::ArrayBuffer(rt, std::make_shared<RawBuffer>()));

    facebook::jsi::Array pts(rt, 2);
    pts.setValueAtIndex(rt, 0, 10.0);
    pts.setValueAtIndex(rt, 1, 20.0);
    root.setProperty(rt, "points", pts);

    // Mutate nested object AFTER setting it on `root` to verify materialize_node synchronization!
    facebook::jsi::Object child(rt);
    root.setProperty(rt, "nested", child);
    child.setProperty(rt, "lateKey", "lateVal");

    rt.record_direct_event(1, 77, 123456, "onSvgLoad", facebook::jsi::Value(rt, root));

    bool sink_called = false;
    const uint32_t rc = rt.dispatch_direct([&](NveEventBatch batch) -> uint32_t {
      sink_called = true;
      assert(batch.len == 1);
      const NveDirectEvent& ev = batch.ptr[0];
      assert(ev.surface_id == 1 && ev.node_id == 77 && ev.timestamp_us == 123456);
      assert(ev.kind == 8 && ev.payload_tag == 5);
      assert(nve::str_slice_view(ev.name) == "onSvgLoad");
      assert(ev.value.tag == NVE_VAL_MAP);

      auto find_field = [&](std::string_view key) -> const NveValue* {
        for (size_t i = 0; i < ev.value.data.map_val.len; ++i) {
          if (nve::str_slice_view(ev.value.data.map_val.ptr[i].key) == key) {
            return &ev.value.data.map_val.ptr[i].value;
          }
        }
        return nullptr;
      };

      const NveValue* f_event = find_field("event");
      assert(f_event && f_event->tag == NVE_VAL_STR &&
             nve::str_slice_view(f_event->data.str_val) == "onSvgLoad");
      const NveValue* f_width = find_field("width");
      assert(f_width && f_width->tag == NVE_VAL_F64 &&
             std::fabs(f_width->data.f64_val - 128.5) < 1e-9);
      const NveValue* f_anim = find_field("animated");
      assert(f_anim && f_anim->tag == NVE_VAL_BOOL && f_anim->data.bool_val == 1);
      const NveValue* f_tag = find_field("tag");
      assert(f_tag && f_tag->tag == NVE_VAL_NULL);
      const NveValue* f_big = find_field("big");
      assert(f_big && f_big->tag == NVE_VAL_I64 &&
             f_big->data.i64_val == -9007199254740993LL);
      const NveValue* f_blob = find_field("blob");
      assert(f_blob && f_blob->tag == NVE_VAL_BYTES && f_blob->data.bytes_val.len == 4 &&
             f_blob->data.bytes_val.ptr[0] == 0xCA && f_blob->data.bytes_val.ptr[3] == 0xBE);
      const NveValue* f_pts = find_field("points");
      assert(f_pts && f_pts->tag == NVE_VAL_ARRAY && f_pts->data.array_val.len == 2);
      assert(std::fabs(f_pts->data.array_val.ptr[0].data.f64_val - 10.0) < 1e-9);
      assert(std::fabs(f_pts->data.array_val.ptr[1].data.f64_val - 20.0) < 1e-9);
      const NveValue* f_nested = find_field("nested");
      assert(f_nested && f_nested->tag == NVE_VAL_MAP && f_nested->data.map_val.len == 1);
      assert(nve::str_slice_view(f_nested->data.map_val.ptr[0].key) == "lateKey");
      assert(nve::str_slice_view(f_nested->data.map_val.ptr[0].value.data.str_val) == "lateVal");
      return 1u;
    });
    assert(sink_called && rc == 1u);

    // (b) Binary EventRing path -> decode_event_ring -> zero-copy BinValueView
    nve::EventRingWriter ring;
    ring.clear();
    assert(rt.record_custom_event(ring, 1, 77, 123456, "onSvgLoad", facebook::jsi::Value(rt, root)));

    const auto& bytes = ring.finalize();
    std::vector<nve::DecodedEvent> decoded;
    assert(nve::decode_event_ring(NveBufferSlice{bytes.data(), bytes.size()}, decoded));
    assert(decoded.size() == 1);
    assert(decoded[0].kind == nve::EventKind::Custom);
    assert(decoded[0].node_id == 77);
    assert(decoded[0].name == "onSvgLoad");

    const auto view = nve::BinValueView::parse(
        decoded[0].payload_bytes.data(), decoded[0].payload_bytes.size());
    assert(view.has_value() && view->tag() == NVE_VAL_MAP);
    const auto ev_str = view->map_get("event")->as_str();
    assert(ev_str.has_value() && *ev_str == "onSvgLoad");
    // Zero-copy pointer identity inside decoded[0].payload_bytes!
    const auto* p_start = reinterpret_cast<const char*>(decoded[0].payload_bytes.data());
    const auto* p_end = p_start + decoded[0].payload_bytes.size();
    assert(ev_str->data() >= p_start && ev_str->data() < p_end);
    assert(std::fabs(*view->map_get("width")->as_f64() - 128.5) < 1e-9);
    assert(view->map_get("animated")->as_bool() == true);
    assert(view->map_get("tag")->is_null());
    assert(view->map_get("big")->as_i64() == -9007199254740993LL);
    assert(view->map_get("blob")->as_bytes()->len == 4);
    assert(view->map_get("points")->array_len() == 2);
    assert(std::fabs(*view->map_get("points")->array_at(0)->as_f64() - 10.0) < 1e-9);
    assert(std::fabs(*view->map_get("points")->array_at(1)->as_f64() - 20.0) < 1e-9);
    assert(view->map_get("nested")->map_get("lateKey")->as_str() == "lateVal");
  }

  // (c) Cyclic object reference must be bounded safely by NVE_BIN_VALUE_MAX_DEPTH
  rt.reset_frame();
  {
    facebook::jsi::Object cyc(rt);
    cyc.setProperty(rt, "self", cyc);
    const NveValue captured = rt.capture_value(facebook::jsi::Value(rt, cyc));
    assert(captured.tag == NVE_VAL_MAP);
    std::vector<uint8_t> bin;
    assert(!nve::encode_bin_value(bin, captured)); // depth > 64 cleanly rejected
    assert(bin.empty());
  }

  // (d) 1,200-frame arena reuse test: zero extra slab allocations after frame 1!
  rt.reset_frame();
  size_t slabs_after_warmup = 0;
  for (int frame = 0; frame < 1200; ++frame) {
    rt.reset_frame();
    for (int k = 0; k < 8; ++k) {
      facebook::jsi::Object ev(rt);
      ev.setProperty(rt, "event", "onGestureUpdate");
      ev.setProperty(rt, "x", static_cast<double>(frame + k));
      ev.setProperty(rt, "y", 42.5);
      ev.setProperty(rt, "active", (k & 1) == 0);
      facebook::jsi::Array v(rt, 2);
      v.setValueAtIndex(rt, 0, 1.5);
      v.setValueAtIndex(rt, 1, -2.5);
      ev.setProperty(rt, "velocity", std::move(v));
      rt.record_direct_event(1, static_cast<uint32_t>(10 + k), static_cast<uint64_t>(frame), "onGestureUpdate", facebook::jsi::Value(rt, ev));
    }
    assert(rt.direct_event_batch().len == 8);
    if (frame == 0) {
      slabs_after_warmup = rt.arena().slab_count();
    } else {
      assert(rt.arena().slab_count() == slabs_after_warmup);
    }
  }
}
#endif

// 5b. Test BumpArena alignment/reuse across 1,500 frames and BinValueView roundtrip + 600+ seeded fuzz corpus.
void test_bump_arena_and_bin_value_fuzz() {
  // BumpArena alignment & slab reuse across 1,500 frames
  {
    nve::BumpArena arena(2048u);
    size_t warmup_slabs = 0;
    for (int frame = 0; frame < 1500; ++frame) {
      arena.reset();
      for (size_t align : {size_t{1}, size_t{2}, size_t{4}, size_t{8}, size_t{16}, size_t{32}, size_t{64}}) {
        void* p = arena.alloc(37u, align);
        assert(p != nullptr);
        assert((reinterpret_cast<uintptr_t>(p) & (align - 1u)) == 0u);
      }
      // Force crossing slab boundary on frame 0, then verify zero new slabs for frames 1..1499.
      auto* big = arena.alloc_array<NveMapEntry>(80u);
      assert(big != nullptr);
      big[0] = NveMapEntry{arena.copy_str("key"), nve::make_i64_value(frame)};
      if (frame == 0) {
        warmup_slabs = arena.slab_count();
        assert(warmup_slabs >= 2u);
      } else {
        assert(arena.slab_count() == warmup_slabs);
      }
    }
  }

  // Roundtrip all 8 NveValueTag variants + nested containers + exact depth boundary (64 ok, 65 fail)
  std::vector<uint8_t> baseline;
  {
    const uint8_t blob[4] = {0x11, 0x22, 0x33, 0x44};
    const NveValue arr_elems[3] = {
        nve::make_null_value(),
        nve::make_bool_value(false),
        nve::make_i64_value(-1234567890123LL),
    };
    const NveMapEntry inner_entries[2] = {
        {nve::make_str_slice("arr"), nve::make_array_value(arr_elems, 3)},
        {nve::make_str_slice("bytes"), nve::make_bytes_value(NveBufferSlice{blob, sizeof(blob)})},
    };
    const NveMapEntry root_entries[4] = {
        {nve::make_str_slice("str"), nve::make_str_value("hello-utf8")},
        {nve::make_str_slice("num"), nve::make_f64_value(3.141592653589793)},
        {nve::make_str_slice("inner"), nve::make_map_value(inner_entries, 2)},
        {nve::make_str_slice("flag"), nve::make_bool_value(true)},
    };
    const NveValue root = nve::make_map_value(root_entries, 4);
    assert(nve::encode_bin_value(baseline, root));

    const auto view = nve::BinValueView::parse(baseline.data(), baseline.size());
    assert(view.has_value() && view->tag() == NVE_VAL_MAP && view->map_len() == 4);
    assert(view->map_get("str")->as_str() == "hello-utf8");
    assert(std::fabs(*view->map_get("num")->as_f64() - 3.141592653589793) < 1e-12);
    assert(view->map_get("flag")->as_bool() == true);
    const auto inner = view->map_get("inner");
    assert(inner.has_value() && inner->map_get("arr")->array_at(2)->as_i64() == -1234567890123LL);
    assert(inner->map_get("bytes")->as_bytes()->len == 4);

    // Depth boundary: 64 ok, 65 rejected
    auto build_nested = [](uint32_t depth) {
      std::vector<uint8_t> cur{static_cast<uint8_t>(NVE_VAL_NULL)};
      for (uint32_t d = 0; d < depth; ++d) {
        std::vector<uint8_t> next;
        next.reserve(cur.size() + 9u);
        next.push_back(static_cast<uint8_t>(NVE_VAL_ARRAY));
        const uint32_t payload_len = static_cast<uint32_t>(4u + cur.size());
        const uint32_t count = 1u;
        next.insert(next.end(), reinterpret_cast<const uint8_t*>(&payload_len),
                    reinterpret_cast<const uint8_t*>(&payload_len) + 4);
        next.insert(next.end(), reinterpret_cast<const uint8_t*>(&count),
                    reinterpret_cast<const uint8_t*>(&count) + 4);
        next.insert(next.end(), cur.begin(), cur.end());
        cur = std::move(next);
      }
      return cur;
    };
    const auto ok64 = build_nested(NVE_BIN_VALUE_MAX_DEPTH);
    assert(nve::BinValueView::parse(ok64.data(), ok64.size()).has_value());
    const auto bad65 = build_nested(NVE_BIN_VALUE_MAX_DEPTH + 1u);
    assert(!nve::BinValueView::parse(bad65.data(), bad65.size()).has_value());
  }

  // 600+ seeded fuzz corpus for BinValueView::parse (verified under ASan+UBSan)
  std::mt19937_64 rng(0xB1A7C0DE5EEDCAFEULL);
  auto rand_u8 = [&]() -> uint8_t { return static_cast<uint8_t>(rng() & 0xFFu); };
  size_t fuzz_cases = 0;

  // Prefix truncations
  for (size_t cut = 0; cut < baseline.size(); ++cut) {
    assert(!nve::BinValueView::parse(baseline.data(), cut).has_value());
    ++fuzz_cases;
  }
  // Trailing byte
  {
    std::vector<uint8_t> trailing = baseline;
    trailing.push_back(0x00);
    assert(!nve::BinValueView::parse(trailing.data(), trailing.size()).has_value());
    ++fuzz_cases;
  }
  // Invalid bool, invalid tag, invalid UTF-8, oversized counts
  {
    const uint8_t bad_bool[2] = {NVE_VAL_BOOL, 2};
    const uint8_t bad_tag[1] = {99};
    const uint8_t bad_utf8[8] = {NVE_VAL_STR, 3, 0, 0, 0, 0xFF, 0xFE, 0x80};
    const uint8_t huge_arr[9] = {NVE_VAL_ARRAY, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF};
    const uint8_t huge_map[9] = {NVE_VAL_MAP, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF};
    assert(!nve::BinValueView::parse(bad_bool, sizeof(bad_bool)).has_value());
    assert(!nve::BinValueView::parse(bad_tag, sizeof(bad_tag)).has_value());
    assert(!nve::BinValueView::parse(bad_utf8, sizeof(bad_utf8)).has_value());
    assert(!nve::BinValueView::parse(huge_arr, sizeof(huge_arr)).has_value());
    assert(!nve::BinValueView::parse(huge_map, sizeof(huge_map)).has_value());
    fuzz_cases += 5;
  }
  // 300 random buffers
  for (int i = 0; i < 300; ++i) {
    const size_t len = static_cast<size_t>(rng() % 513u);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    if (const auto parsed = nve::BinValueView::parse(buf.data(), buf.size())) {
      (void)parsed->as_str();
      (void)parsed->map_get("str");
      (void)parsed->array_at(0);
    }
    ++fuzz_cases;
  }
  // 250 mutated copies of baseline
  for (int i = 0; i < 250; ++i) {
    std::vector<uint8_t> mut = baseline;
    const size_t edits = 1u + static_cast<size_t>(rng() % 6u);
    for (size_t e = 0; e < edits; ++e) {
      const size_t mode = rng() % 4u;
      if (mode == 0 && !mut.empty()) {
        mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8u));
      } else if (mode == 1 && !mut.empty()) {
        mut[rng() % mut.size()] = rand_u8();
      } else if (mode == 2 && mut.size() > 1) {
        mut.resize(1u + static_cast<size_t>(rng() % (mut.size() - 1u)));
      } else {
        mut.push_back(rand_u8());
      }
    }
    if (const auto parsed = nve::BinValueView::parse(mut.data(), mut.size())) {
      (void)parsed->as_str();
      (void)parsed->map_get("inner");
      (void)parsed->array_at(0);
    }
    ++fuzz_cases;
  }
  assert(fuzz_cases >= 600);
}

// 6. Test NVE_COMP_TEXT multi-level style inheritance, explicit overrides, reparent reset, and default opaque black.
void test_text_style_inheritance() {
  NveSurfaceConfig cfg{.surface_id = 11, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);

  // Tree:
  //   root (1)
  //     ├── text_default (10)            [no explicit style -> default 0xFF000000, 16pt]
  //     └── text_parent (20)             [explicit 0xFF112233, 24pt]
  //           ├── text_child (21)        [inherits 0xFF112233, 24pt]
  //           ├── view_middle (22)       [non-text wrapper]
  //           │     └── text_grand (23)  [inherits 0xFF112233, 24pt across View]
  //           └── text_override (24)     [explicit 0xFFAA5500, 12pt]
  //                 └── text_nested (25) [inherits 0xFFAA5500, 12pt from override]
  CommandBufferEncoder enc(11, 1);
  enc.create_node(10, NVE_COMP_TEXT);
  enc.create_node(20, NVE_COMP_TEXT);
  enc.create_node(21, NVE_COMP_TEXT);
  enc.create_node(22, NVE_COMP_VIEW);
  enc.create_node(23, NVE_COMP_TEXT);
  enc.create_node(24, NVE_COMP_TEXT);
  enc.create_node(25, NVE_COMP_TEXT);

  enc.set_text(10, "Default black text");
  enc.set_text(21, "Inherited child");
  enc.set_text(23, "Inherited grandchild");
  enc.set_text(25, "Inherited under override");

  nve::StyleProps parent_style{};
  parent_style.text_color_rgba = 0xFF112233u;
  parent_style.font_size = 24.0f;
  enc.set_style(20, parent_style);

  nve::StyleProps override_style{};
  override_style.text_color_rgba = 0xFFAA5500u;
  override_style.font_size = 12.0f;
  enc.set_style(24, override_style);

  enc.set_children(NVE_ROOT_NODE_ID, {10, 20});
  enc.set_children(20, {21, 22, 24});
  enc.set_children(22, {23});
  enc.set_children(24, {25});
  enc.commit(true);

  assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

  auto snaps = snapshot_by_id(host);
  // Default opaque black (0xFF000000) & 16pt on standalone Text
  assert(snaps.at(10).text_color_rgba == 0xFF000000u);
  assert(approx_eq(snaps.at(10).font_size, 16.0f, 1e-3f));

  // Multi-level inheritance (direct child and grandchild across View)
  assert(snaps.at(20).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(20).font_size, 24.0f, 1e-3f));
  assert(snaps.at(21).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(21).font_size, 24.0f, 1e-3f));
  assert(snaps.at(23).text_color_rgba == 0xFF112233u);
  assert(approx_eq(snaps.at(23).font_size, 24.0f, 1e-3f));

  // Explicit child override and inheritance beneath the override
  assert(snaps.at(24).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps.at(24).font_size, 12.0f, 1e-3f));
  assert(snaps.at(25).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps.at(25).font_size, 12.0f, 1e-3f));

  // Verify inherited font_size affects measured height (24pt child > 12pt nested override)
  nve::LayoutRect r21{}, r25{};
  assert(nve::fabric_host_get_node_layout(host, 21, &r21));
  assert(nve::fabric_host_get_node_layout(host, 25, &r25));
  assert(r21.height > r25.height);

  // Frame 2: dynamically update parent style -> non-explicit descendants update, explicit subtree stays isolated
  CommandBufferEncoder enc2(11, 2);
  nve::StyleProps updated_parent{};
  updated_parent.text_color_rgba = 0xFF00CC88u;
  updated_parent.font_size = 32.0f;
  enc2.set_style(20, updated_parent);
  enc2.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);

  auto snaps2 = snapshot_by_id(host);
  assert(snaps2.at(21).text_color_rgba == 0xFF00CC88u);
  assert(approx_eq(snaps2.at(21).font_size, 32.0f, 1e-3f));
  assert(snaps2.at(23).text_color_rgba == 0xFF00CC88u);
  assert(approx_eq(snaps2.at(23).font_size, 32.0f, 1e-3f));
  assert(snaps2.at(24).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps2.at(24).font_size, 12.0f, 1e-3f));
  assert(snaps2.at(25).text_color_rgba == 0xFFAA5500u);
  assert(approx_eq(snaps2.at(25).font_size, 12.0f, 1e-3f));

  // Frame 3: reparent node 23 directly under root (no Text ancestor) -> resets to default 0xFF000000 / 16pt
  CommandBufferEncoder enc3(11, 3);
  enc3.set_children(22, {});
  enc3.set_children(NVE_ROOT_NODE_ID, {10, 20, 23});
  enc3.commit(true);
  assert(nve_fabric_host_submit_frame(host, enc3.finish(), nullptr) == 0);

  auto snaps3 = snapshot_by_id(host);
  assert(snaps3.at(23).text_color_rgba == 0xFF000000u);
  assert(approx_eq(snaps3.at(23).font_size, 16.0f, 1e-3f));

  nve_fabric_host_destroy(host);
}

// 7. Test fabric_host_set_text_measurer callback, max_width wrapping, empty strings, and heuristic reset.
void test_text_measurer_callback() {
  NveSurfaceConfig cfg{.surface_id = 12, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);

  int call_count = 0;
  bool saw_empty = false;
  float last_narrow_max_w = -1.0f;
  float last_wide_max_w = -1.0f;
  float last_font_size = 0.0f;

  // Distinctive custom measurer: 10pt per char, 40pt line height (clearly distinct from 1.3x heuristic).
  nve::fabric_host_set_text_measurer(
      host,
      [&](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
        ++call_count;
        if (text.empty()) {
          saw_empty = true;
        }
        last_font_size = font_size;
        const float raw_w = static_cast<float>(text.size()) * (font_size * 0.625f);
        const float line_h = font_size * 2.5f;
        if (text.find("NARROW:") == 0) {
          last_narrow_max_w = max_width;
        } else if (text.find("WIDE:") == 0) {
          last_wide_max_w = max_width;
        }
        if (max_width > 0.0f && raw_w > max_width) {
          const float lines = std::ceil(raw_w / max_width);
          return NveIntrinsicSize{max_width, lines * line_h};
        }
        return NveIntrinsicSize{raw_w, line_h};
      });

  CommandBufferEncoder enc(12, 1);
  enc.create_node(2, NVE_COMP_VIEW); /* narrow container: 120px wide, 10px padding -> 100px max_w */
  enc.create_node(3, NVE_COMP_TEXT); /* narrow wrapping text */
  enc.create_node(4, NVE_COMP_VIEW); /* wide container: 360px wide */
  enc.create_node(5, NVE_COMP_TEXT); /* wide single-line text */
  enc.create_node(6, NVE_COMP_TEXT); /* empty text */

  nve::LayoutProps narrow_box{};
  narrow_box.width = {1, 120.0f};
  narrow_box.padding = {0.0f, 10.0f, 0.0f, 10.0f};
  narrow_box.align_items = 1; /* FlexStart so width isn't stretched beyond measured width */
  enc.set_layout(2, narrow_box);

  nve::LayoutProps wide_box{};
  wide_box.width = {1, 360.0f};
  wide_box.align_items = 1; /* FlexStart */
  enc.set_layout(4, wide_box);

  nve::StyleProps st{};
  st.font_size = 16.0f;
  enc.set_style(3, st);
  enc.set_style(5, st);
  enc.set_style(6, st);

  // 30 chars @ 16pt * 0.625 = 300pt unwrapped: wraps to 3 lines (120pt) in 100pt box, 1 line (40pt) in 360pt box
  enc.set_text(3, "NARROW:01234567890123456789012");
  enc.set_text(5, "WIDE:0123456789012345678901234");
  enc.set_text(6, "");

  enc.set_children(NVE_ROOT_NODE_ID, {2, 4, 6});
  enc.set_children(2, {3});
  enc.set_children(4, {5});
  enc.commit(true);

  assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  assert(call_count >= 2);
  assert(!saw_empty); /* empty text short-circuits without invoking callback */
  assert(approx_eq(last_font_size, 16.0f, 0.1f));
  assert(approx_eq(last_narrow_max_w, 100.0f, 1.0f));
  assert(approx_eq(last_wide_max_w, 360.0f, 1.0f));

  nve::LayoutRect r_narrow{}, r_wide{}, r_empty{};
  assert(nve::fabric_host_get_node_layout(host, 3, &r_narrow));
  assert(nve::fabric_host_get_node_layout(host, 5, &r_wide));
  assert(nve::fabric_host_get_node_layout(host, 6, &r_empty));

  // Narrow wraps to 3 lines @ 40pt = 120pt; wide stays 1 line @ 40pt
  assert(approx_eq(r_narrow.width, 100.0f, 1.0f));
  assert(approx_eq(r_narrow.height, 120.0f, 1.0f));
  assert(approx_eq(r_wide.width, 300.0f, 1.0f));
  assert(approx_eq(r_wide.height, 40.0f, 1.0f));
  assert(r_narrow.height > r_wide.height * 2.0f);
  assert(approx_eq(r_empty.height, 0.0f, 0.5f));

  // Reset measurer to empty callback -> falls back to built-in heuristic (16 * 1.3 = 20.8pt/line)
  const int calls_before_reset = call_count;
  nve::fabric_host_set_text_measurer(host, {});
  nve::fabric_host_set_surface_size(host, 400.0f, 600.0f, 2.0f);

  assert(call_count == calls_before_reset);
  nve::LayoutRect r_wide_heuristic{}, r_narrow_heuristic{};
  assert(nve::fabric_host_get_node_layout(host, 5, &r_wide_heuristic));
  assert(nve::fabric_host_get_node_layout(host, 3, &r_narrow_heuristic));
  // Heuristic single line height is ~20.8pt (far below custom 40pt)
  assert(approx_eq(r_wide_heuristic.height, 16.0f * 1.3f, 1.5f));
  assert(r_narrow_heuristic.height > r_wide_heuristic.height);

  // Misbehaving callback returning NaN / negative values must not crash or yield NaN layouts
  nve::fabric_host_set_text_measurer(
      host,
      [](std::string_view, float, float) -> NveIntrinsicSize {
        return NveIntrinsicSize{std::numeric_limits<float>::quiet_NaN(), -50.0f};
      });
  nve::fabric_host_set_surface_size(host, 400.0f, 600.0f, 2.0f);
  nve::LayoutRect r_sanitized{};
  assert(nve::fabric_host_get_node_layout(host, 5, &r_sanitized));
  assert(std::isfinite(r_sanitized.width) && r_sanitized.width >= 0.0f);
  assert(std::isfinite(r_sanitized.height) && r_sanitized.height >= 0.0f);

  nve_fabric_host_destroy(host);
}

// 8. Deterministic seeded fuzz corpus (600+ CommandBuffers + 300+ EventRings) asserting zero crashes/UB
//    and clean negative error codes on malformed inputs.
void test_seeded_fuzz_corpus() {
  std::mt19937_64 rng(0x4E56455F46555A5AuLL);
  auto rand_u8 = [&]() -> uint8_t { return static_cast<uint8_t>(rng() & 0xFFu); };
  auto rand_range = [&](size_t lo, size_t hi) -> size_t {
    return lo + static_cast<size_t>(rng() % (hi - lo + 1));
  };

  NveSurfaceConfig cfg{.surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};
  NveFabricHostHandle shared_host = nve_fabric_host_create(cfg);
  assert(shared_host != nullptr);

  // Build a rich valid baseline CommandBuffer containing all 10 opcodes for truncation/mutation fuzzing.
  CommandBufferEncoder baseline_enc(1, 1);
  baseline_enc.create_node(2, NVE_COMP_VIEW);
  baseline_enc.create_node(3, NVE_COMP_TEXT);
  baseline_enc.create_node(4, NVE_COMP_CUSTOM, "FuzzComp");
  baseline_enc.set_children(NVE_ROOT_NODE_ID, {2});
  baseline_enc.set_children(2, {3, 4});
  baseline_enc.set_text(3, "Fuzz baseline text");
  nve::LayoutProps base_lp{};
  base_lp.width = {1, 200.0f};
  base_lp.height = {1, 80.0f};
  base_lp.padding = {4.0f, 4.0f, 4.0f, 4.0f};
  baseline_enc.set_layout(2, base_lp);
  nve::StyleProps base_st{};
  base_st.background_rgba = 0xFF112233u;
  base_st.font_size = 16.0f;
  baseline_enc.set_style(3, base_st);
  baseline_enc.set_raw_prop_null(4, "n");
  baseline_enc.set_raw_prop_bool(4, "b", true);
  baseline_enc.set_raw_prop_i64(4, "i", -42);
  baseline_enc.set_raw_prop_f64(4, "f", 3.14);
  baseline_enc.set_raw_prop_string(4, "s", "hello");
  baseline_enc.set_raw_prop_bytes(4, "bytes", {1, 2, 3, 4});
  baseline_enc.set_event_mask(2, (1ULL << 0) | (1ULL << 6));
  baseline_enc.dispatch_command(4, "cmd", {9, 8});
  baseline_enc.remove_node(4);
  baseline_enc.commit(true);
  const NveBufferSlice base_slice = baseline_enc.finish();
  const std::vector<uint8_t> base_bytes(base_slice.ptr, base_slice.ptr + base_slice.len);

  size_t cmd_fuzz_cases = 0;

  // (a) Every prefix truncation of the baseline CommandBuffer MUST fail cleanly with a negative error code.
  for (size_t cut = 0; cut < base_bytes.size(); ++cut) {
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{base_bytes.data(), cut}, nullptr);
    assert(rc < 0);
    ++cmd_fuzz_cases;
  }

  // (b) Explicit adversarial/oversized/cyclic/NaN CommandBuffers (must all return < 0 without OOM/crash).
  {
    auto expect_err = [&](const std::vector<uint8_t>& buf, int32_t expected_or_any_neg = 0) {
      const int32_t rc = nve_fabric_host_submit_frame(
          shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
      if (expected_or_any_neg < 0) {
        assert(rc == expected_or_any_neg);
      } else {
        assert(rc < 0);
      }
      ++cmd_fuzz_cases;
    };

    // Oversized SET_CHILDREN count (0xFFFFFFFF) -> -4 without OOM
    {
      CommandBufferEncoder enc(1, 10);
      enc.create_node(50, NVE_COMP_VIEW);
      enc.finish();
      auto& raw = enc.raw();
      uint32_t op_count = 2;
      std::memcpy(raw.data() + 20, &op_count, 4);
      raw.push_back(NVE_OP_SET_CHILDREN);
      uint32_t pid = 50;
      uint32_t huge_cnt = 0xFFFFFFFFu;
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&pid), reinterpret_cast<uint8_t*>(&pid) + 4);
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&huge_cnt), reinterpret_cast<uint8_t*>(&huge_cnt) + 4);
      expect_err(raw, -4);
    }

    // Oversized SET_TEXT length (0xFFFFFFFF) -> -4 without OOM
    {
      CommandBufferEncoder enc(1, 11);
      enc.create_node(51, NVE_COMP_TEXT);
      enc.finish();
      auto& raw = enc.raw();
      uint32_t op_count = 2;
      std::memcpy(raw.data() + 20, &op_count, 4);
      raw.push_back(NVE_OP_SET_TEXT);
      uint32_t nid = 51;
      uint32_t huge_len = 0xFFFFFFFFu;
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&nid), reinterpret_cast<uint8_t*>(&nid) + 4);
      raw.insert(raw.end(), reinterpret_cast<uint8_t*>(&huge_len), reinterpret_cast<uint8_t*>(&huge_len) + 4);
      expect_err(raw, -4);
    }

    // Self-parenting cycle (52 -> {52}) -> -6
    {
      CommandBufferEncoder enc(1, 12);
      enc.create_node(52, NVE_COMP_VIEW);
      enc.set_children(52, {52});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // Root-as-child (53 -> {NVE_ROOT_NODE_ID}) -> -6
    {
      CommandBufferEncoder enc(1, 13);
      enc.create_node(53, NVE_COMP_VIEW);
      enc.set_children(53, {NVE_ROOT_NODE_ID});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // 2-node cycle (54 -> 55 -> 54) -> -6
    {
      CommandBufferEncoder enc(1, 14);
      enc.create_node(54, NVE_COMP_VIEW);
      enc.create_node(55, NVE_COMP_VIEW);
      enc.set_children(54, {55});
      enc.set_children(55, {54});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // Duplicate child in same parent (56 -> {57, 57}) -> -6
    {
      CommandBufferEncoder enc(1, 15);
      enc.create_node(56, NVE_COMP_VIEW);
      enc.create_node(57, NVE_COMP_VIEW);
      enc.set_children(56, {57, 57});
      enc.commit(true);
      enc.finish();
      expect_err(enc.raw(), -6);
    }

    // NaN / Inf layout dimensions and negative flex_grow -> -6
    {
      CommandBufferEncoder enc(1, 16);
      enc.create_node(58, NVE_COMP_VIEW);
      nve::LayoutProps bad_lp{};
      bad_lp.width = {1, std::numeric_limits<float>::quiet_NaN()};
      enc.set_layout(58, bad_lp);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
    {
      CommandBufferEncoder enc(1, 17);
      enc.create_node(59, NVE_COMP_VIEW);
      nve::LayoutProps bad_lp{};
      bad_lp.flex_grow = -1.0f;
      enc.set_layout(59, bad_lp);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
    {
      CommandBufferEncoder enc(1, 18);
      enc.create_node(60, NVE_COMP_TEXT);
      nve::StyleProps bad_st{};
      bad_st.font_size = std::numeric_limits<float>::infinity();
      enc.set_style(60, bad_st);
      enc.finish();
      expect_err(enc.raw(), -6);
    }
  }

  // (c) 200 pure random buffers (sizes 0..2048) -> must all return clean negative error codes.
  for (int i = 0; i < 200; ++i) {
    const size_t len = (i == 0) ? 0 : rand_range(1, 2048);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
    assert(rc < 0);
    ++cmd_fuzz_cases;
  }

  // (d) 200 valid-header + random op_count + random payload buffers.
  for (int i = 0; i < 200; ++i) {
    const size_t payload_len = rand_range(0, 512);
    std::vector<uint8_t> buf(24 + payload_len);
    const uint32_t magic = NVE_ABI_MAGIC;
    const uint16_t version = NVE_ABI_VERSION;
    const uint16_t flags = 0;
    const uint32_t sid = 1;
    const uint64_t fid = static_cast<uint64_t>(i + 100);
    const uint32_t op_count = (i % 5 == 0) ? 0xFFFFFFFFu : static_cast<uint32_t>(rand_range(1, 64));
    std::memcpy(buf.data() + 0, &magic, 4);
    std::memcpy(buf.data() + 4, &version, 2);
    std::memcpy(buf.data() + 6, &flags, 2);
    std::memcpy(buf.data() + 8, &sid, 4);
    std::memcpy(buf.data() + 12, &fid, 8);
    std::memcpy(buf.data() + 20, &op_count, 4);
    for (size_t j = 24; j < buf.size(); ++j) {
      buf[j] = rand_u8();
    }
    const int32_t rc = nve_fabric_host_submit_frame(
        shared_host, NveBufferSlice{buf.data(), buf.size()}, nullptr);
    // Random bytes with op_count >= 1 will almost always fail cleanly with < 0; even if by rare
    // chance a valid encoding occurs, rc <= 0 and zero crashes/UB.
    assert(rc <= 0);
    ++cmd_fuzz_cases;
  }

  // (e) 200 mutated copies of the rich baseline buffer (bit flips, random overwrites, random splice/append).
  for (int i = 0; i < 200; ++i) {
    std::vector<uint8_t> mut = base_bytes;
    const size_t num_edits = rand_range(1, 6);
    for (size_t e = 0; e < num_edits; ++e) {
      const size_t mode = rng() % 4;
      if (mode == 0 && !mut.empty()) {
        mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8));
      } else if (mode == 1 && !mut.empty()) {
        mut[rng() % mut.size()] = rand_u8();
      } else if (mode == 2 && mut.size() > 1) {
        mut.resize(rand_range(1, mut.size() - 1));
      } else {
        const size_t extra = rand_range(1, 32);
        for (size_t k = 0; k < extra; ++k) mut.push_back(rand_u8());
      }
    }
    NveFabricHostHandle fresh = nve_fabric_host_create(cfg);
    const int32_t rc = nve_fabric_host_submit_frame(
        fresh, NveBufferSlice{mut.data(), mut.size()}, nullptr);
    assert(rc <= 0);
    nve_fabric_host_destroy(fresh);
    ++cmd_fuzz_cases;
  }

  assert(cmd_fuzz_cases >= 600);

  // Verify shared_host survived 600+ hostile fuzz buffers and still mounts a clean valid frame!
  {
    CommandBufferEncoder recovery(1, 9999);
    recovery.create_node(9001, NVE_COMP_VIEW);
    recovery.create_node(9002, NVE_COMP_TEXT);
    recovery.set_text(9002, "Recovered cleanly");
    nve::LayoutProps lp{};
    lp.width = {1, 250.0f};
    lp.height = {1, 100.0f};
    recovery.set_layout(9001, lp);
    recovery.set_children(NVE_ROOT_NODE_ID, {9001});
    recovery.set_children(9001, {9002});
    recovery.commit(true);
    assert(nve_fabric_host_submit_frame(shared_host, recovery.finish(), nullptr) == 0);
    nve::LayoutRect r{};
    assert(nve::fabric_host_get_node_layout(shared_host, 9001, &r));
    assert(approx_eq(r.width, 250.0f) && approx_eq(r.height, 100.0f));
  }

  // Deep chain stress (1,000 nested nodes -> exercises iterative text style propagation & safe layout).
  {
    NveFabricHostHandle deep_host = nve_fabric_host_create(cfg);
    CommandBufferEncoder deep_enc(1, 1);
    uint32_t prev = NVE_ROOT_NODE_ID;
    for (uint32_t id = 2; id <= 1001; ++id) {
      deep_enc.create_node(id, (id == 1001) ? NVE_COMP_TEXT : NVE_COMP_VIEW);
      deep_enc.set_children(prev, {id});
      prev = id;
    }
    deep_enc.set_text(1001, "Deep leaf");
    deep_enc.commit(true);
    assert(nve_fabric_host_submit_frame(deep_host, deep_enc.finish(), nullptr) == 0);
    nve_fabric_host_destroy(deep_host);
  }

  nve_fabric_host_destroy(shared_host);

  // (f) EventRing fuzz corpus (300+ cases): truncations, oversized counts/lengths, trailing garbage, random bytes.
  nve::EventRingWriter valid_ring;
  valid_ring.clear();
  valid_ring.push_press(1, 2, 100, 12.5f, 34.5f);
  valid_ring.push_long_press(1, 2, 200, 15.0f, 36.0f);
  valid_ring.push_scroll(1, 3, 300, 0.0f, 120.0f, 393.0f, 2000.0f);
  valid_ring.push_text_change(1, 4, 400, "hello world");
  valid_ring.push_empty(1, 5, 500, 5); /* Focus */
  valid_ring.push_empty(1, 5, 600, 6); /* Blur */
  valid_ring.push_layout(1, 6, 700, 0.0f, 0.0f, 300.0f, 200.0f);
  const std::string custom_json = "{\"ok\":true}";
  valid_ring.push_custom(
      1, 7, 800, "onCustom", reinterpret_cast<const uint8_t*>(custom_json.data()), custom_json.size());
  const std::vector<uint8_t> valid_ring_bytes = valid_ring.finalize();

  std::vector<nve::DecodedEvent> out_events;
  assert(nve::decode_event_ring(
      NveBufferSlice{valid_ring_bytes.data(), valid_ring_bytes.size()}, out_events));
  assert(out_events.size() == 8);

  // Null pointer + non-zero len -> false; zero len -> true
  assert(!nve::decode_event_ring(NveBufferSlice{nullptr, 16}, out_events));
  assert(nve::decode_event_ring(NveBufferSlice{nullptr, 0}, out_events));
  assert(out_events.empty());

  size_t ring_fuzz_cases = 0;

  // Every non-empty prefix truncation of valid_ring_bytes must return false and leave out_events empty.
  for (size_t cut = 1; cut < valid_ring_bytes.size(); ++cut) {
    assert(!nve::decode_event_ring(NveBufferSlice{valid_ring_bytes.data(), cut}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // Trailing garbage byte after valid ring -> false and out_events cleared.
  {
    std::vector<uint8_t> trailing = valid_ring_bytes;
    trailing.push_back(0xAA);
    assert(!nve::decode_event_ring(NveBufferSlice{trailing.data(), trailing.size()}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // Oversized count (0xFFFFFFFF) -> false without OOM.
  {
    const uint8_t huge_cnt[4] = {0xFF, 0xFF, 0xFF, 0xFF};
    assert(!nve::decode_event_ring(NveBufferSlice{huge_cnt, sizeof(huge_cnt)}, out_events));
    assert(out_events.empty());
    ++ring_fuzz_cases;
  }

  // 150 random EventRing buffers + 100 mutated copies of valid_ring_bytes.
  for (int i = 0; i < 150; ++i) {
    const size_t len = rand_range(1, 1024);
    std::vector<uint8_t> buf(len);
    for (auto& b : buf) b = rand_u8();
    const bool ok = nve::decode_event_ring(NveBufferSlice{buf.data(), buf.size()}, out_events);
    if (!ok) {
      assert(out_events.empty());
    }
    ++ring_fuzz_cases;
  }
  for (int i = 0; i < 100; ++i) {
    std::vector<uint8_t> mut = valid_ring_bytes;
    const size_t flips = rand_range(1, 4);
    for (size_t f = 0; f < flips; ++f) {
      mut[rng() % mut.size()] ^= static_cast<uint8_t>(1u << (rng() % 8));
    }
    const bool ok = nve::decode_event_ring(NveBufferSlice{mut.data(), mut.size()}, out_events);
    if (!ok) {
      assert(out_events.empty());
    }
    ++ring_fuzz_cases;
  }

  assert(ring_fuzz_cases >= 300);
}

// 9. Test nve_host_runtime.hpp utilities: unpack_rgba, current_timestamp_us, view_tag_for_snapshot,
//    and load_nve_dylib error codes.
void test_host_runtime_utilities_and_dylib_errors() {
  const uint64_t t1 = nve::current_timestamp_us();
  const uint64_t t2 = nve::current_timestamp_us();
  assert(t2 >= t1);

  // RGBA unpacking (little-endian: R low byte, A high byte)
  {
    const nve::RgbaColor clear = nve::unpack_rgba(0x00000000u);
    assert(clear.r8 == 0 && clear.g8 == 0 && clear.b8 == 0 && clear.a8 == 0);
    assert(approx_eq(clear.r, 0.0f, 1e-6f) && approx_eq(clear.a, 0.0f, 1e-6f));

    const nve::RgbaColor black = nve::unpack_rgba(0xFF000000u);
    assert(black.r8 == 0 && black.g8 == 0 && black.b8 == 0 && black.a8 == 255);
    assert(approx_eq(black.r, 0.0f, 1e-6f) && approx_eq(black.a, 1.0f, 1e-6f));

    const nve::RgbaColor white = nve::unpack_rgba(0xFFFFFFFFu);
    assert(white.r8 == 255 && white.g8 == 255 && white.b8 == 255 && white.a8 == 255);
    assert(approx_eq(white.r, 1.0f, 1e-6f) && approx_eq(white.g, 1.0f, 1e-6f) &&
           approx_eq(white.b, 1.0f, 1e-6f) && approx_eq(white.a, 1.0f, 1e-6f));

    const nve::RgbaColor sample = nve::unpack_rgba(0x80336699u);
    assert(sample.r8 == 0x99u && sample.g8 == 0x66u && sample.b8 == 0x33u && sample.a8 == 0x80u);
    assert(approx_eq(sample.r, 0x99 / 255.0f, 1e-5f));
    assert(approx_eq(sample.g, 0x66 / 255.0f, 1e-5f));
    assert(approx_eq(sample.b, 0x33 / 255.0f, 1e-5f));
    assert(approx_eq(sample.a, 0x80 / 255.0f, 1e-5f));
  }

  // View tags distinguish leaf text vs wrapper text
  {
    nve::RenderedNodeSnapshot leaf_text{};
    leaf_text.kind = NVE_COMP_TEXT;
    nve::RenderedNodeSnapshot wrapper_text{};
    wrapper_text.kind = NVE_COMP_TEXT;
    wrapper_text.children = {10};
    assert(nve::view_tag_for_snapshot(leaf_text) != nve::view_tag_for_snapshot(wrapper_text));

    nve::RenderedNodeSnapshot view_empty{};
    view_empty.kind = NVE_COMP_VIEW;
    nve::RenderedNodeSnapshot view_with_kids{};
    view_with_kids.kind = NVE_COMP_VIEW;
    view_with_kids.children = {10};
    assert(nve::view_tag_for_snapshot(view_empty) == nve::view_tag_for_snapshot(view_with_kids));
  }

  // load_nve_dylib error codes: empty path -> 1, missing file -> 1
  {
    nve::LoadedModule empty_res = nve::load_nve_dylib("");
    assert(!empty_res.ok());
    assert(empty_res.error_code == 1);
    assert(!empty_res.error_message.empty());
    assert(!empty_res.lib.is_open());

    nve::LoadedModule missing_res = nve::load_nve_dylib("/nonexistent/path/nve_missing_module.so");
    assert(!missing_res.ok());
    assert(missing_res.error_code == 1);
    assert(!missing_res.error_message.empty());
    assert(!missing_res.lib.is_open());
  }

#ifdef NVE_BRIDGE_FIXTURE_NO_ENTRY
  {
    nve::LoadedModule no_entry = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_NO_ENTRY);
    assert(!no_entry.ok());
    assert(no_entry.error_code == 2);
    assert(!no_entry.error_message.empty());
    assert(!no_entry.lib.is_open());
  }
#endif

#ifdef NVE_BRIDGE_FIXTURE_NO_SURFACE
  {
    nve::LoadedModule no_surface = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_NO_SURFACE);
    assert(!no_surface.ok());
    assert(no_surface.error_code == 3);
    assert(!no_surface.error_message.empty());
    assert(!no_surface.lib.is_open());
  }
#endif

#ifdef NVE_BRIDGE_FIXTURE_VALID
  {
    nve::LoadedModule valid = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_VALID);
    assert(valid.ok());
    assert(valid.error_code == 0);
    assert(valid.lib.is_open());
    assert(valid.get_interface != nullptr);
    assert(valid.module.surface != nullptr);
  }
#endif
}

struct MockReconciledView {
  uint32_t node_id{0};
  uint32_t instance_id{0};
  bool is_text_wrapper{false};
  std::string text;
  int update_count{0};
  std::vector<uint32_t> mounted_children;
};

struct MockReconcilerAdapter {
  uint32_t next_instance_id{1};
  int root_updates{0};
  uint32_t last_root_bg{0};
  int creates{0};
  std::vector<uint32_t> destroyed_nodes;
  std::vector<uint32_t> root_children;

  void update_root(const nve::RenderedNodeSnapshot& snap) {
    ++root_updates;
    last_root_bg = snap.background_rgba;
  }

  std::unique_ptr<MockReconciledView> create_view(const nve::RenderedNodeSnapshot& snap) {
    ++creates;
    auto view = std::make_unique<MockReconciledView>();
    view->node_id = snap.node_id;
    view->instance_id = next_instance_id++;
    view->is_text_wrapper = (snap.kind == NVE_COMP_TEXT && !snap.children.empty());
    return view;
  }

  void update_view(
      std::unique_ptr<MockReconciledView>& view,
      const nve::RenderedNodeSnapshot& snap) {
    assert(view != nullptr);
    view->text = snap.text;
    view->update_count += 1;
  }

  void destroy_view(uint32_t node_id, std::unique_ptr<MockReconciledView>& view) {
    assert(view != nullptr);
    assert(view->node_id == node_id);
    destroyed_nodes.push_back(node_id);
    view.reset();
  }

  template <typename LookupFn>
  void sync_children(
      uint32_t parent_id,
      std::unique_ptr<MockReconciledView>* parent_view,
      const std::vector<uint32_t>& child_ids,
      LookupFn&& lookup_fn) {
    std::vector<uint32_t> resolved;
    for (uint32_t cid : child_ids) {
      auto* child_ptr = lookup_fn(cid);
      if (child_ptr != nullptr && *child_ptr != nullptr) {
        resolved.push_back((*child_ptr)->node_id);
      }
    }
    assert(lookup_fn(999999u) == nullptr);
    if (parent_id == NVE_ROOT_NODE_ID) {
      assert(parent_view == nullptr);
      root_children = std::move(resolved);
    } else {
      assert(parent_view != nullptr && *parent_view != nullptr);
      (*parent_view)->mounted_children = std::move(resolved);
    }
  }
};

// 10. Test nve::RetainedViewReconciler across mount, update, text leaf<->wrapper tag swap,
//     child reordering, pruning, and clear.
void test_retained_view_reconciler() {
  nve::RetainedViewReconciler<std::unique_ptr<MockReconciledView>, MockReconcilerAdapter>
      reconciler;
  MockReconcilerAdapter adapter;

  auto make_snap = [](uint32_t id,
                      uint16_t kind,
                      std::string text = {},
                      std::vector<uint32_t> children = {},
                      uint32_t bg = 0) {
    nve::RenderedNodeSnapshot s{};
    s.node_id = id;
    s.kind = kind;
    s.text = std::move(text);
    s.children = std::move(children);
    s.background_rgba = bg;
    return s;
  };

  // Frame 1: initial mount (root -> [2(VIEW)], 2 -> [3(leaf TEXT), 4(PRESSABLE)])
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}, 0xFF102030u),
          make_snap(2, NVE_COMP_VIEW, "", {3, 4}),
          make_snap(3, NVE_COMP_TEXT, "Leaf"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(reconciler.size() == 3);
  assert(adapter.root_updates == 1);
  assert(adapter.last_root_bg == 0xFF102030u);
  assert(adapter.creates == 3);
  assert(adapter.destroyed_nodes.empty());
  assert(adapter.root_children == std::vector<uint32_t>({2}));
  assert(reconciler.find(2) != nullptr);
  assert((*reconciler.find(2))->mounted_children == std::vector<uint32_t>({3, 4}));
  assert(!(*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->text == "Leaf");
  const uint32_t leaf_text_inst = (*reconciler.find(3))->instance_id;
  const uint32_t view2_inst = (*reconciler.find(2))->instance_id;

  // Frame 2: incremental update + child reordering (no view recreation!)
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}, 0xFF102030u),
          make_snap(2, NVE_COMP_VIEW, "", {4, 3}),
          make_snap(3, NVE_COMP_TEXT, "Leaf updated"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(adapter.creates == 3);
  assert(adapter.destroyed_nodes.empty());
  assert((*reconciler.find(2))->instance_id == view2_inst);
  assert((*reconciler.find(3))->instance_id == leaf_text_inst);
  assert((*reconciler.find(3))->text == "Leaf updated");
  assert((*reconciler.find(3))->update_count == 2);
  assert((*reconciler.find(2))->mounted_children == std::vector<uint32_t>({4, 3}));

  // Frame 3: text leaf -> wrapper tag swap (node 3 gains child 5) => destroys & recreates node 3
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}),
          make_snap(2, NVE_COMP_VIEW, "", {4, 3}),
          make_snap(3, NVE_COMP_TEXT, "", {5}),
          make_snap(5, NVE_COMP_TEXT, "Nested span"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(reconciler.size() == 4);
  assert(adapter.creates == 5); /* +1 recreated node 3, +1 new node 5 */
  assert(adapter.destroyed_nodes == std::vector<uint32_t>({3}));
  assert((*reconciler.find(3))->instance_id != leaf_text_inst);
  assert((*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->mounted_children == std::vector<uint32_t>({5}));
  const uint32_t wrapper_text_inst = (*reconciler.find(3))->instance_id;

  // Frame 4: wrapper -> leaf tag swap + pruning node 4 and node 5
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}),
          make_snap(2, NVE_COMP_VIEW, "", {3}),
          make_snap(3, NVE_COMP_TEXT, "Back to leaf"),
      },
      adapter);
  assert(reconciler.size() == 2);
  assert(reconciler.find(4) == nullptr);
  assert(reconciler.find(5) == nullptr);
  assert((*reconciler.find(3))->instance_id != wrapper_text_inst);
  assert(!(*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->text == "Back to leaf");

  // Clear destroys remaining views
  reconciler.clear(adapter);
  assert(reconciler.size() == 0);
  assert(reconciler.empty());
  assert(reconciler.find(2) == nullptr);
  assert(reconciler.find(3) == nullptr);
}

// 11. Test nve::NativeHostSession (attach, custom text measurer re-applied across reload,
//     press dispatch, props update, atomic failed reload, and real dylib hot-reload).
void test_native_host_session() {
  NveSurfaceConfig cfg{.surface_id = 55, .width = 360.0f, .height = 240.0f, .scale_factor = 2.0f};
  nve::NativeHostSession session;
  assert(!session.is_valid());
  assert(session.bridge() == nullptr);
  assert(session.render() == -100);
  assert(session.dispatch_press(2, 5.0f, 5.0f) == 0);
  assert(session.drain_events().empty());
  assert(session.snapshot_nodes().empty());

  int measurer_calls = 0;
  session.set_text_measurer(
      [&](std::string_view text, float, float) -> NveIntrinsicSize {
        ++measurer_calls;
        return NveIntrinsicSize{static_cast<float>(text.size()) * 11.0f, 44.0f};
      });

  std::vector<nve::DecodedEvent> callback_events;
  session.set_event_callback(
      [&](const nve::DecodedEvent& ev) { callback_events.push_back(ev); });

  // Attach in-process interface (automatically installs text measurer & renders frame 1)
  assert(session.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
  assert(session.is_valid());
  assert(session.bridge() != nullptr);
  assert(session.snapshot_nodes().size() == 3);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  assert(measurer_calls >= 1);
  auto text_rect = session.bridge()->node_layout(3);
  assert(text_rect.has_value() && approx_eq(text_rect->height, 44.0f, 1.0f));
#endif

  // Update props via JSON -> emits onPropsApplied
  assert(session.update_props_json("{\"title\":\"HostSession\"}"));
  auto props_evs = session.drain_events();
  assert(props_evs.size() == 1);
  assert(props_evs[0].name == "onPropsApplied");
  assert(callback_events.size() == 1);

  // Dispatch press -> increments counter to 1 and emits binary onIncrement
  assert(session.dispatch_press(2, 12.0f, 18.0f) == 1);
  auto press_evs = session.drain_events();
  assert(press_evs.size() == 1);
  assert(press_evs[0].name == "onIncrement");
  const auto press_view = nve::BinValueView::parse(
      press_evs[0].payload_bytes.data(), press_evs[0].payload_bytes.size());
  assert(press_view.has_value() && press_view->map_get("count")->as_i64() == 1);
  assert(callback_events.size() == 2);

  // Resize surface
  session.set_surface_size(480.0f, 320.0f, 2.0f);
  assert(approx_eq(session.surface_config().width, 480.0f));
  assert(approx_eq(session.surface_config().height, 320.0f));

  // Failed reload_dylib must be atomic: returns false with code 1 and preserves active session/state
  int err_code = 0;
  std::string err_msg;
  assert(!session.reload_dylib("/nonexistent/bad_reload.dylib", &err_code, &err_msg));
  assert(err_code == 1);
  assert(!err_msg.empty());
  assert(session.is_valid());
  const auto preserved_state = session.bridge()->snapshot_state();
  assert(preserved_state.size() == 1 && preserved_state[0] == '1');

#ifdef NVE_BRIDGE_FIXTURE_VALID
  // Full real-dylib load_dylib + press + reload_dylib verifying state preservation AND
  // custom text measurer re-application across reload!
  {
    nve::NativeHostSession dylib_session;
    int dylib_measurer_calls = 0;
    dylib_session.set_text_measurer(
        [&](std::string_view text, float, float) -> NveIntrinsicSize {
          ++dylib_measurer_calls;
          return NveIntrinsicSize{static_cast<float>(text.size()) * 10.0f, 52.0f};
        });

    int load_err = -1;
    std::string load_msg;
    assert(dylib_session.load_dylib(NVE_BRIDGE_FIXTURE_VALID, cfg, &load_err, &load_msg));
    assert(load_err == 0 && load_msg.empty());
    assert(dylib_session.is_valid());
    assert(dylib_session.dispatch_press(2, 10.0f, 10.0f) == 1);
    assert(dylib_session.dispatch_press(2, 10.0f, 10.0f) == 1);
    assert(dylib_session.update_props_json("{\"v\":1}"));

    const int calls_before_reload = dylib_measurer_calls;
    int reload_err = -1;
    std::string reload_msg;
    assert(dylib_session.reload_dylib(NVE_BRIDGE_FIXTURE_VALID, &reload_err, &reload_msg));
    assert(reload_err == 0 && reload_msg.empty());
    assert(dylib_session.is_valid());

    // State ("2") must survive reload_dylib, and text node 3 reflects "Fixture:2"
    const auto reloaded_snaps = dylib_session.snapshot_nodes();
    bool saw_preserved_text = false;
    for (const auto& s : reloaded_snaps) {
      if (s.node_id == 3 && s.text == "Fixture:2") {
        saw_preserved_text = true;
      }
    }
    assert(saw_preserved_text);

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    // Custom text measurer must have been re-applied on the reloaded bridge
    assert(dylib_measurer_calls > calls_before_reload);
    auto reloaded_rect = dylib_session.bridge()->node_layout(3);
    assert(reloaded_rect.has_value() && approx_eq(reloaded_rect->height, 52.0f, 1.0f));
#else
    (void)calls_before_reload;
#endif
  }
#endif
}

} // namespace

int main() {
  test_all_opcodes_and_custom_components();
  test_corrupt_buffer_error_codes();
  test_flexbox_layout();
  test_rn_component_bridge();
#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
  test_mock_jsi_recorder_runtime();
#endif
  test_bump_arena_and_bin_value_fuzz();
  test_text_style_inheritance();
  test_text_measurer_callback();
  test_seeded_fuzz_corpus();
  test_host_runtime_utilities_and_dylib_errors();
  test_retained_view_reconciler();
  test_native_host_session();

  std::cout << "nve_fabric_bridge_test passed ("
            << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI" : "Standalone")
            << " mode)\n";
  return 0;
}
