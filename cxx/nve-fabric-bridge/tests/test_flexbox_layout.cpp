#include "test_common.hpp"

namespace nve_test {

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

    // Post-flex wrapping text re-measure: a Row with a fixed 100px sibling + flex_shrink text container
    // shrinks the text container from unconstrained width to 100px, causing multi-line wrap height!
    CommandBufferEncoder ce3(2, 3);
    ce3.create_node(60, NVE_COMP_VIEW); /* 200x200 Row, alignItems=FlexStart */
    ce3.create_node(61, NVE_COMP_VIEW); /* fixed 100x40 */
    ce3.create_node(62, NVE_COMP_TEXT); /* auto width/height, shrink=1, long text */
    ce3.set_children(NVE_ROOT_NODE_ID, {60});
    ce3.set_children(60, {61, 62});
    ce3.set_text(62, "012345678901234567890123456789"); /* 30 chars @ 16pt -> ~268pt wide */

    nve::LayoutProps wrap_row{};
    wrap_row.width = {1, 200.0f};
    wrap_row.height = {1, 200.0f};
    wrap_row.flex_direction = 1; /* Row */
    wrap_row.align_items = 1; /* FlexStart */
    ce3.set_layout(60, wrap_row);
    nve::LayoutProps fixed_sib{};
    fixed_sib.width = {1, 100.0f};
    fixed_sib.height = {1, 40.0f};
    fixed_sib.flex_shrink = 0.0f;
    ce3.set_layout(61, fixed_sib);
    nve::LayoutProps shrink_txt{};
    shrink_txt.flex_shrink = 1.0f;
    ce3.set_layout(62, shrink_txt);
    ce3.commit(true);
    assert(nve_fabric_host_submit_frame(card_host, ce3.finish(), nullptr) == 0);

    nve::LayoutRect r62{};
    assert(nve::fabric_host_get_node_layout(card_host, 62, &r62));
    assert(approx_eq(r62.width, 100.0f, 1.5f));
    // Must wrap to >= 2 lines (~41.6pt+ at 16pt font), NOT single-line ~20.8pt!
    assert(r62.height > 35.0f);

    // Verify fabric_host_enqueue_text_change encodes a valid TextChange event in the ring.
    nve::fabric_host_enqueue_text_change(card_host, 62, 12345, "hello typed");
    const NveBufferSlice tc_slice = nve_fabric_host_drain_events(card_host);
    std::vector<nve::DecodedEvent> tc_events;
    assert(nve::decode_event_ring(tc_slice, tc_events));
    assert(tc_events.size() == 1);
    assert(tc_events[0].kind == nve::EventKind::TextChange);
    assert(tc_events[0].node_id == 62);
    assert(tc_events[0].text == "hello typed");

    nve_fabric_host_destroy(card_host);
  }

  nve_fabric_host_destroy(host);
}

// 3b. Deep parity tests for post-flex wrapped text re-measurement, auto-height propagation,
//     true Baseline alignment (mixed font sizes + nested containers + margins), align_self,

} // namespace nve_test
