#include "test_common.hpp"

namespace nve_test {

//     flex_basis/max_width redistribution, and edge cases (empty text, zero width, non-finite).
void test_standalone_flexbox_parity() {
  // (a) Row with fixed sibling + flex:1 long wrapped text inside an auto-height Row,
  //     followed by a sibling below: verifies text wraps to resolved width, auto-height Row
  //     grows to fit wrapped lines, and the below sibling does not overlap.
  {
    NveSurfaceConfig cfg{.surface_id = 31, .width = 320.0f, .height = 600.0f, .scale_factor = 2.0f};
    NveFabricHostHandle host = nve_fabric_host_create(cfg);
    assert(host != nullptr);

    // Deterministic custom measurer: 10pt per char, 20pt per line.
    // 50 chars = 500pt unwrapped.
    // Row inner width = 300 - 20(padding) = 280; fixed sibling = 100, gap = 10 -> flex text width = 170.
    // At 170pt, ceil(500 / 170) = 3 lines -> 60pt height (vs 2 lines = 40pt at 280pt!).
    int measure_calls = 0;
    nve::fabric_host_set_text_measurer(
        host,
        [&](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
          ++measure_calls;
          const float char_w = font_size * 0.625f; /* 10pt at 16pt font */
          const float line_h = font_size * 1.25f;  /* 20pt at 16pt font */
          const float raw_w = static_cast<float>(text.size()) * char_w;
          if (max_width > 0.0f && raw_w > max_width) {
            const float lines = std::ceil(raw_w / max_width);
            return NveIntrinsicSize{max_width, lines * line_h};
          }
          return NveIntrinsicSize{raw_w, line_h};
        });

    CommandBufferEncoder enc(31, 1);
    enc.create_node(2, NVE_COMP_VIEW); /* Page column: width=300, height=Auto, gap=12 */
    enc.create_node(3, NVE_COMP_VIEW); /* Auto-height Row: width=100%, padding=10, gap=10 */
    enc.create_node(4, NVE_COMP_VIEW); /* Fixed sibling: 100x24, flex_shrink=0 */
    enc.create_node(5, NVE_COMP_TEXT); /* flex_grow=1, flex_shrink=1, 50 chars */
    enc.create_node(6, NVE_COMP_VIEW); /* Sibling below Row: 300x30 */

    enc.set_children(NVE_ROOT_NODE_ID, {2});
    enc.set_children(2, {3, 6});
    enc.set_children(3, {4, 5});
    enc.set_text(5, "01234567890123456789012345678901234567890123456789"); /* 50 chars */

    nve::LayoutProps page{};
    page.width = {1, 300.0f};
    page.gap = 12.0f;
    enc.set_layout(2, page);

    nve::LayoutProps row{};
    row.width = {2, 100.0f};
    row.flex_direction = 1; /* Row */
    row.align_items = 1;    /* FlexStart */
    row.padding = {10.0f, 10.0f, 10.0f, 10.0f};
    row.gap = 10.0f;
    enc.set_layout(3, row);

    nve::LayoutProps fixed{};
    fixed.width = {1, 100.0f};
    fixed.height = {1, 24.0f};
    fixed.flex_shrink = 0.0f;
    enc.set_layout(4, fixed);

    nve::LayoutProps flex_txt{};
    flex_txt.flex_grow = 1.0f;
    flex_txt.flex_shrink = 1.0f;
    enc.set_layout(5, flex_txt);

    nve::LayoutProps below{};
    below.width = {1, 300.0f};
    below.height = {1, 30.0f};
    enc.set_layout(6, below);

    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
    assert(measure_calls >= 1);

    nve::LayoutRect r_page{}, r_row{}, r_fixed{}, r_txt{}, r_below{};
    assert(nve::fabric_host_get_node_layout(host, 2, &r_page));
    assert(nve::fabric_host_get_node_layout(host, 3, &r_row));
    assert(nve::fabric_host_get_node_layout(host, 4, &r_fixed));
    assert(nve::fabric_host_get_node_layout(host, 5, &r_txt));
    assert(nve::fabric_host_get_node_layout(host, 6, &r_below));

    assert(approx_eq(r_fixed.width, 100.0f, 1.0f));
    assert(approx_eq(r_txt.x, 120.0f, 1.0f));       /* 10 pad + 100 fixed + 10 gap */
    assert(approx_eq(r_txt.width, 170.0f, 1.0f));   /* 280 inner - 100 - 10 */
    assert(approx_eq(r_txt.height, 60.0f, 1.0f));   /* 3 wrapped lines @ 20pt */
    assert(approx_eq(r_row.height, 80.0f, 1.0f));   /* 60pt text + 20pt vertical padding */
    assert(approx_eq(r_below.y, 92.0f, 1.0f));      /* 80pt row + 12pt gap (no overlap!) */
    assert(approx_eq(r_page.height, 122.0f, 1.0f)); /* 80 + 12 + 30 */

    nve_fabric_host_destroy(host);
  }

  // (b) Nested flex containers (Row -> flex:1 Column -> Auto-height InnerRow -> [Badge, flex:1 Text])
  //     plus max_width & flex_basis constraints in both Row and Column:
  //     verifies wrapped height propagates cleanly through multiple container levels without clipping.
  {
    NveSurfaceConfig cfg{.surface_id = 32, .width = 300.0f, .height = 600.0f, .scale_factor = 2.0f};
    NveFabricHostHandle host = nve_fabric_host_create(cfg);
    assert(host != nullptr);

    nve::fabric_host_set_text_measurer(
        host,
        [](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
          const float raw_w = static_cast<float>(text.size()) * (font_size * 0.625f);
          const float line_h = font_size * 1.25f;
          if (max_width > 0.0f && raw_w > max_width) {
            return NveIntrinsicSize{max_width, std::ceil(raw_w / max_width) * line_h};
          }
          return NveIntrinsicSize{raw_w, line_h};
        });

    // OuterRow: width=300, height=Auto, gap=10
    //   - LeftAvatar (11): 80x30, flex_shrink=0
    //   - RightCol (12): flex_grow=1, flex_shrink=1, padding=8, gap=6  -> resolved width = 210, inner_w = 194
    //       - InnerRow (13): width=Auto (stretched to 194), height=Auto, gap=4
    //           - Badge (14): 40x20, flex_shrink=0
    //           - LongText (15): flex_grow=1, flex_shrink=1, 45 chars (450pt unwrapped)
    //             -> resolved width = 194 - 40 - 4 = 150 -> ceil(450/150) = 3 lines -> 60pt!
    //       - ConstrainedColText (16): width=Auto, max_width=100, 30 chars (300pt unwrapped)
    //           -> clamped to 100pt -> ceil(300/100) = 3 lines -> 60pt!
    //   - Footer (17): sibling below OuterRow (300x24)
    CommandBufferEncoder enc(32, 1);
    enc.create_node(10, NVE_COMP_VIEW);
    enc.create_node(11, NVE_COMP_VIEW);
    enc.create_node(12, NVE_COMP_VIEW);
    enc.create_node(13, NVE_COMP_VIEW);
    enc.create_node(14, NVE_COMP_VIEW);
    enc.create_node(15, NVE_COMP_TEXT);
    enc.create_node(16, NVE_COMP_TEXT);
    enc.create_node(17, NVE_COMP_VIEW);

    enc.set_children(NVE_ROOT_NODE_ID, {10, 17});
    enc.set_children(10, {11, 12});
    enc.set_children(12, {13, 16});
    enc.set_children(13, {14, 15});
    enc.set_text(15, "012345678901234567890123456789012345678901234"); /* 45 chars */
    enc.set_text(16, "012345678901234567890123456789");                /* 30 chars */

    nve::LayoutProps outer_row{};
    outer_row.width = {1, 300.0f};
    outer_row.flex_direction = 1; /* Row */
    outer_row.align_items = 1;    /* FlexStart */
    outer_row.gap = 10.0f;
    enc.set_layout(10, outer_row);

    nve::LayoutProps left_av{};
    left_av.width = {1, 80.0f};
    left_av.height = {1, 30.0f};
    left_av.flex_shrink = 0.0f;
    enc.set_layout(11, left_av);

    nve::LayoutProps right_col{};
    right_col.flex_grow = 1.0f;
    right_col.flex_shrink = 1.0f;
    right_col.padding = {8.0f, 8.0f, 8.0f, 8.0f};
    right_col.gap = 6.0f;
    enc.set_layout(12, right_col);

    nve::LayoutProps inner_row{};
    inner_row.flex_direction = 1; /* Row */
    inner_row.align_items = 1;    /* FlexStart */
    inner_row.gap = 4.0f;
    enc.set_layout(13, inner_row);

    nve::LayoutProps badge{};
    badge.width = {1, 40.0f};
    badge.height = {1, 20.0f};
    badge.flex_shrink = 0.0f;
    enc.set_layout(14, badge);

    nve::LayoutProps long_txt{};
    long_txt.flex_grow = 1.0f;
    long_txt.flex_shrink = 1.0f;
    enc.set_layout(15, long_txt);

    nve::LayoutProps col_max_txt{};
    col_max_txt.max_width = {1, 100.0f};
    enc.set_layout(16, col_max_txt);

    nve::LayoutProps footer{};
    footer.width = {1, 300.0f};
    footer.height = {1, 24.0f};
    enc.set_layout(17, footer);

    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

    nve::LayoutRect r10{}, r12{}, r13{}, r15{}, r16{}, r17{};
    assert(nve::fabric_host_get_node_layout(host, 10, &r10));
    assert(nve::fabric_host_get_node_layout(host, 12, &r12));
    assert(nve::fabric_host_get_node_layout(host, 13, &r13));
    assert(nve::fabric_host_get_node_layout(host, 15, &r15));
    assert(nve::fabric_host_get_node_layout(host, 16, &r16));
    assert(nve::fabric_host_get_node_layout(host, 17, &r17));

    assert(approx_eq(r12.width, 210.0f, 1.0f));
    assert(approx_eq(r13.width, 194.0f, 1.0f));
    assert(approx_eq(r15.width, 150.0f, 1.0f));
    assert(approx_eq(r15.height, 60.0f, 1.0f));
    assert(approx_eq(r13.height, 60.0f, 1.0f));
    assert(approx_eq(r16.width, 100.0f, 1.0f));
    assert(approx_eq(r16.height, 60.0f, 1.0f));
    assert(approx_eq(r16.y, 8.0f + 60.0f + 6.0f, 1.0f));
    assert(approx_eq(r12.height, 16.0f + 60.0f + 6.0f + 60.0f, 1.0f)); /* 142pt */
    assert(approx_eq(r10.height, 142.0f, 1.0f));
    assert(approx_eq(r17.y, 142.0f, 1.0f)); /* Zero clipping or overlap! */

    // Also test flex_basis + iterative max_width clamping redistribution inside Row.
    CommandBufferEncoder enc_fb(32, 2);
    enc_fb.create_node(20, NVE_COMP_VIEW); /* 300px Auto-height Row */
    enc_fb.create_node(21, NVE_COMP_VIEW); /* flex_basis=50, flex_grow=1, max_width=100 -> clamped to 100 */
    enc_fb.create_node(22, NVE_COMP_TEXT); /* flex_basis=50, flex_grow=1 -> absorbs remaining 200px! */
    enc_fb.set_children(NVE_ROOT_NODE_ID, {20});
    enc_fb.set_children(20, {21, 22});
    enc_fb.set_text(22, "0123456789012345678901234567890123456789"); /* 40 chars = 400pt -> 2 lines at 200pt */

    nve::LayoutProps fb_row{};
    fb_row.width = {1, 300.0f};
    fb_row.flex_direction = 1;
    fb_row.align_items = 1;
    enc_fb.set_layout(20, fb_row);

    nve::LayoutProps fb_c1{};
    fb_c1.height = {1, 20.0f};
    fb_c1.flex_grow = 1.0f;
    fb_c1.max_width = {1, 100.0f};
    enc_fb.set_layout(21, fb_c1);
    enc_fb.set_raw_prop_f64(21, "flexBasis", 50.0);

    nve::LayoutProps fb_c2{};
    fb_c2.flex_grow = 1.0f;
    enc_fb.set_layout(22, fb_c2);
    enc_fb.set_raw_prop_f64(22, "flexBasis", 50.0);

    enc_fb.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc_fb.finish(), nullptr) == 0);

    nve::LayoutRect r20{}, r21{}, r22{};
    assert(nve::fabric_host_get_node_layout(host, 20, &r20));
    assert(nve::fabric_host_get_node_layout(host, 21, &r21));
    assert(nve::fabric_host_get_node_layout(host, 22, &r22));
    assert(approx_eq(r21.width, 100.0f, 1.0f));
    assert(approx_eq(r22.width, 200.0f, 1.0f));
    assert(approx_eq(r22.height, 40.0f, 1.0f)); /* 2 wrapped lines at 200pt */
    assert(approx_eq(r20.height, 40.0f, 1.0f));

    nve_fabric_host_destroy(host);
  }

  // (c) True Baseline alignment (`align_items: Baseline` = 4) on a Row with mixed font sizes
  //     (12px, 24px, 36px), a nested container holding a baseline text child with top padding,
  //     an empty box fallback (baseline == height), and line cross-size expansion from top margin.
  //     Runs identically in BOTH Standalone and RN Yoga builds, asserting <= 1px parity!
  {
    NveSurfaceConfig cfg{.surface_id = 33, .width = 500.0f, .height = 400.0f, .scale_factor = 2.0f};
    NveFabricHostHandle host = nve_fabric_host_create(cfg);
    assert(host != nullptr);

    CommandBufferEncoder enc(33, 1);
    enc.create_node(2, NVE_COMP_VIEW); /* Auto-height Row: align_items=Baseline, padding=10, gap=8 */
    enc.create_node(3, NVE_COMP_TEXT); /* 12px text: baseline = 9.6 */
    enc.create_node(4, NVE_COMP_TEXT); /* 24px text: baseline = 19.2 */
    enc.create_node(5, NVE_COMP_TEXT); /* 36px text: baseline = 28.8 (max baseline!) */
    enc.create_node(6, NVE_COMP_VIEW); /* Nested container: padding.top=8 -> child 7 (12px) => baseline = 17.6 */
    enc.create_node(7, NVE_COMP_TEXT); /* 12px text inside container 6 */
    enc.create_node(8, NVE_COMP_VIEW); /* Empty box 20x25 -> baseline falls back to height=25 */
    enc.create_node(9, NVE_COMP_VIEW); /* Below sibling to verify auto-height expansion */

    enc.set_children(NVE_ROOT_NODE_ID, {2, 9});
    enc.set_children(2, {3, 4, 5, 6, 8});
    enc.set_children(6, {7});

    enc.set_text(3, "Small");
    enc.set_text(4, "Medium");
    enc.set_text(5, "Large");
    enc.set_text(7, "Nested");

    nve::StyleProps s12{};
    s12.font_size = 12.0f;
    nve::StyleProps s24{};
    s24.font_size = 24.0f;
    nve::StyleProps s36{};
    s36.font_size = 36.0f;
    enc.set_style(3, s12);
    enc.set_style(4, s24);
    enc.set_style(5, s36);
    enc.set_style(7, s12);

    nve::LayoutProps brow{};
    brow.width = {1, 480.0f};
    brow.flex_direction = 1; /* Row */
    brow.align_items = 4;    /* Baseline */
    brow.padding = {10.0f, 10.0f, 10.0f, 10.0f};
    brow.gap = 8.0f;
    enc.set_layout(2, brow);

    nve::LayoutProps nested_box{};
    nested_box.padding = {8.0f, 4.0f, 2.0f, 4.0f};
    enc.set_layout(6, nested_box);

    nve::LayoutProps empty_box{};
    empty_box.width = {1, 20.0f};
    empty_box.height = {1, 25.0f};
    enc.set_layout(8, empty_box);

    nve::LayoutProps below{};
    below.width = {1, 480.0f};
    below.height = {1, 20.0f};
    enc.set_layout(9, below);

    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

    nve::LayoutRect r2{}, r3{}, r4{}, r5{}, r6{}, r7{}, r8{}, r9{};
    assert(nve::fabric_host_get_node_layout(host, 2, &r2));
    assert(nve::fabric_host_get_node_layout(host, 3, &r3));
    assert(nve::fabric_host_get_node_layout(host, 4, &r4));
    assert(nve::fabric_host_get_node_layout(host, 5, &r5));
    assert(nve::fabric_host_get_node_layout(host, 6, &r6));
    assert(nve::fabric_host_get_node_layout(host, 7, &r7));
    assert(nve::fabric_host_get_node_layout(host, 8, &r8));
    assert(nve::fabric_host_get_node_layout(host, 9, &r9));

    // Expected shared baseline inside Row 2: pad_t(10) + max_baseline(36 * 0.8 = 28.8) = 38.8!
    const float b3 = r3.y + 12.0f * 0.8f;
    const float b4 = r4.y + 24.0f * 0.8f;
    const float b5 = r5.y + 36.0f * 0.8f;
    const float b7 = r6.y + r7.y + 12.0f * 0.8f;
    const float b8 = r8.y + r8.height; /* empty View baseline == bottom edge */

    assert(approx_eq(b3, 38.8f, 1.0f));
    assert(approx_eq(b4, 38.8f, 1.0f));
    assert(approx_eq(b5, 38.8f, 1.0f));
    assert(approx_eq(b7, 38.8f, 1.0f));
    assert(approx_eq(b8, 38.8f, 1.0f));
    assert(approx_eq(b3, b5, 1.0f));
    assert(approx_eq(b4, b5, 1.0f));
    assert(approx_eq(b7, b5, 1.0f));
    assert(approx_eq(b8, b5, 1.0f));
    // Verify individual y offsets match RN Yoga exact values:
    // r5.y ~= 10.0, r4.y ~= 19.6, r3.y ~= 29.2, r6.y ~= 21.2, r8.y ~= 13.8
    assert(approx_eq(r5.y, 10.0f, 1.0f));
    assert(approx_eq(r4.y, 19.6f, 1.0f));
    assert(approx_eq(r3.y, 29.2f, 1.0f));
    assert(approx_eq(r6.y, 21.2f, 1.0f));
    assert(approx_eq(r8.y, 13.8f, 1.0f));
    // Row height must fit pad_t(10) + 36*1.3(46.8) + pad_b(10) = 66.8
    assert(approx_eq(r2.height, 66.8f, 1.0f));
    assert(approx_eq(r9.y, r2.height, 1.0f));

    // Frame 2: Add margin.top=25 to the 12px text (node 3) -> its ascent becomes 9.6+25=34.6 > 28.8,
    // expanding the Row's cross size to max_ascent(34.6) + max_descent(18.0) = 52.6 (+ 20 pad = 72.6)!
    CommandBufferEncoder enc2(33, 2);
    nve::LayoutProps m3{};
    m3.margin = {25.0f, 0.0f, 0.0f, 0.0f};
    enc2.set_layout(3, m3);
    enc2.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc2.finish(), nullptr) == 0);

    assert(nve::fabric_host_get_node_layout(host, 2, &r2));
    assert(nve::fabric_host_get_node_layout(host, 3, &r3));
    assert(nve::fabric_host_get_node_layout(host, 5, &r5));
    assert(nve::fabric_host_get_node_layout(host, 9, &r9));
    // New shared baseline: 10 + 34.6 = 44.6; r3.y = 35.0; r5.y = 44.6 - 28.8 = 15.8; row.h = 72.6
    assert(approx_eq(r3.y + 12.0f * 0.8f, 44.6f, 1.0f));
    assert(approx_eq(r5.y + 36.0f * 0.8f, 44.6f, 1.0f));
    assert(approx_eq(r2.height, 72.6f, 1.0f));
    assert(approx_eq(r9.y, 72.6f, 1.0f));

    nve_fabric_host_destroy(host);
  }

  // (d) Edge cases: align_self overriding align_items (both into and out of Baseline),
  //     Column Baseline fallback, empty text, zero width, and non-finite surface constraints.
  {
    NveSurfaceConfig cfg{.surface_id = 34, .width = 400.0f, .height = 300.0f, .scale_factor = 2.0f};
    NveFabricHostHandle host = nve_fabric_host_create(cfg);
    assert(host != nullptr);

    CommandBufferEncoder enc(34, 1);
    // Row 10: align_items=FlexStart, height=100; children override with alignSelf:
    //   11: default (FlexStart) -> y=0
    //   12: alignSelf="center"  -> y=(100-20)/2 = 40
    //   13: alignSelf="flex-end"-> y=100-20 = 80
    //   14: alignSelf="stretch" (height=Auto) -> h=100
    enc.create_node(10, NVE_COMP_VIEW);
    enc.create_node(11, NVE_COMP_VIEW);
    enc.create_node(12, NVE_COMP_VIEW);
    enc.create_node(13, NVE_COMP_VIEW);
    enc.create_node(14, NVE_COMP_VIEW);

    // Row 20: align_items=FlexStart, height=Auto; two text items override with alignSelf="baseline"
    //         (12px and 36px), empty text with alignSelf="baseline", and a non-baseline child with
    //         alignSelf="flex-end" (height=10) + alignSelf="stretch" (auto height).
    enc.create_node(20, NVE_COMP_VIEW);
    enc.create_node(21, NVE_COMP_TEXT); /* 12px, alignSelf="baseline" */
    enc.create_node(22, NVE_COMP_TEXT); /* 36px, alignSelf="baseline" */
    enc.create_node(23, NVE_COMP_VIEW); /* 20x10, alignSelf="flex-end" */
    enc.create_node(24, NVE_COMP_TEXT); /* empty text "", alignSelf="baseline" */
    enc.create_node(25, NVE_COMP_VIEW); /* 16xAuto, alignSelf="stretch" -> matches line height */

    // Row 30: zero-width edge case + fixed sibling filling 100% leaving 0px for flex_shrink text
    enc.create_node(30, NVE_COMP_VIEW);
    enc.create_node(31, NVE_COMP_VIEW); /* width=100, flex_shrink=0 */
    enc.create_node(32, NVE_COMP_TEXT); /* flex_shrink=1 -> shrunk to 0px! */
    enc.create_node(33, NVE_COMP_VIEW); /* explicit width=0 container with text child 34 */
    enc.create_node(34, NVE_COMP_TEXT);

    // Column 40: align_items=Baseline (must fall back to FlexStart) + child alignSelf="flex-end"
    enc.create_node(40, NVE_COMP_VIEW);
    enc.create_node(41, NVE_COMP_VIEW); /* 40x20 -> x=0 (FlexStart fallback) */
    enc.create_node(42, NVE_COMP_VIEW); /* 40x20, alignSelf="flex-end" -> x=160 */

    enc.set_children(NVE_ROOT_NODE_ID, {10, 20, 30, 33, 40});
    enc.set_children(10, {11, 12, 13, 14});
    enc.set_children(20, {21, 22, 23, 24, 25});
    enc.set_children(30, {31, 32});
    enc.set_children(33, {34});
    enc.set_children(40, {41, 42});

    enc.set_text(21, "tiny");
    enc.set_text(22, "HUGE");
    enc.set_text(24, "");
    enc.set_text(32, "Shrunk to zero width safely");
    enc.set_text(34, "Inside zero width container");

    nve::StyleProps s12{};
    s12.font_size = 12.0f;
    nve::StyleProps s36{};
    s36.font_size = 36.0f;
    enc.set_style(21, s12);
    enc.set_style(22, s36);

    nve::LayoutProps r10_lp{};
    r10_lp.width = {1, 200.0f};
    r10_lp.height = {1, 100.0f};
    r10_lp.flex_direction = 1; /* Row */
    r10_lp.align_items = 1;    /* FlexStart */
    enc.set_layout(10, r10_lp);

    nve::LayoutProps box20{};
    box20.width = {1, 30.0f};
    box20.height = {1, 20.0f};
    enc.set_layout(11, box20);
    enc.set_layout(12, box20);
    enc.set_layout(13, box20);
    nve::LayoutProps stretch_box{};
    stretch_box.width = {1, 30.0f};
    enc.set_layout(14, stretch_box);

    enc.set_raw_prop_string(12, "alignSelf", "center");
    enc.set_raw_prop_string(13, "alignSelf", "flex-end");
    enc.set_raw_prop_string(14, "alignSelf", "stretch");

    nve::LayoutProps r20_lp{};
    r20_lp.width = {1, 300.0f};
    r20_lp.flex_direction = 1; /* Row */
    r20_lp.align_items = 1;    /* FlexStart */
    enc.set_layout(20, r20_lp);
    enc.set_raw_prop_string(21, "alignSelf", "baseline");
    enc.set_raw_prop_string(22, "alignSelf", "baseline");
    enc.set_raw_prop_string(24, "alignSelf", "baseline");
    nve::LayoutProps end_small{};
    end_small.width = {1, 20.0f};
    end_small.height = {1, 10.0f};
    enc.set_layout(23, end_small);
    enc.set_raw_prop_string(23, "alignSelf", "flex-end");
    nve::LayoutProps stretch_bar{};
    stretch_bar.width = {1, 16.0f};
    enc.set_layout(25, stretch_bar);
    enc.set_raw_prop_string(25, "alignSelf", "stretch");

    nve::LayoutProps r30_lp{};
    r30_lp.width = {1, 100.0f};
    r30_lp.flex_direction = 1;
    r30_lp.align_items = 1;
    enc.set_layout(30, r30_lp);
    nve::LayoutProps full100{};
    full100.width = {1, 100.0f};
    full100.height = {1, 20.0f};
    full100.flex_shrink = 0.0f;
    enc.set_layout(31, full100);
    nve::LayoutProps shrink_zero{};
    shrink_zero.flex_shrink = 1.0f;
    enc.set_layout(32, shrink_zero);

    nve::LayoutProps zero_box{};
    zero_box.width = {1, 0.0f};
    enc.set_layout(33, zero_box);

    nve::LayoutProps col40{};
    col40.width = {1, 200.0f};
    col40.flex_direction = 0; /* Column */
    col40.align_items = 4;    /* Baseline -> FlexStart in Column */
    enc.set_layout(40, col40);
    nve::LayoutProps c40_item{};
    c40_item.width = {1, 40.0f};
    c40_item.height = {1, 20.0f};
    enc.set_layout(41, c40_item);
    enc.set_layout(42, c40_item);
    enc.set_raw_prop_string(42, "alignSelf", "flex-end");

    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

    nve::LayoutRect r11{}, r12{}, r13{}, r14{};
    assert(nve::fabric_host_get_node_layout(host, 11, &r11));
    assert(nve::fabric_host_get_node_layout(host, 12, &r12));
    assert(nve::fabric_host_get_node_layout(host, 13, &r13));
    assert(nve::fabric_host_get_node_layout(host, 14, &r14));
    assert(approx_eq(r11.y, 0.0f, 1.0f));
    assert(approx_eq(r12.y, 40.0f, 1.0f));
    assert(approx_eq(r13.y, 80.0f, 1.0f));
    assert(approx_eq(r14.y, 0.0f, 1.0f) && approx_eq(r14.height, 100.0f, 1.0f));

    nve::LayoutRect r20{}, r21{}, r22{}, r23{}, r24{}, r25{};
    assert(nve::fabric_host_get_node_layout(host, 20, &r20));
    assert(nve::fabric_host_get_node_layout(host, 21, &r21));
    assert(nve::fabric_host_get_node_layout(host, 22, &r22));
    assert(nve::fabric_host_get_node_layout(host, 23, &r23));
    assert(nve::fabric_host_get_node_layout(host, 24, &r24));
    assert(nve::fabric_host_get_node_layout(host, 25, &r25));
    assert(approx_eq(r21.y + 12.0f * 0.8f, r22.y + 36.0f * 0.8f, 1.0f));
    assert(approx_eq(r23.y + r23.height, r20.height, 1.0f));
    assert(std::isfinite(r24.y) && approx_eq(r24.height, 0.0f, 0.5f));
    assert(approx_eq(r25.height, r20.height, 1.0f));

    nve::LayoutRect r32{}, r34{};
    assert(nve::fabric_host_get_node_layout(host, 32, &r32));
    assert(nve::fabric_host_get_node_layout(host, 34, &r34));
    assert(std::isfinite(r32.width) && approx_eq(r32.width, 0.0f, 0.5f));
    assert(std::isfinite(r32.height) && r32.height >= 0.0f);
    assert(std::isfinite(r34.width) && approx_eq(r34.width, 0.0f, 0.5f));
    assert(std::isfinite(r34.height) && r34.height >= 0.0f);

    nve::LayoutRect r41{}, r42{};
    assert(nve::fabric_host_get_node_layout(host, 41, &r41));
    assert(nve::fabric_host_get_node_layout(host, 42, &r42));
    assert(approx_eq(r41.x, 0.0f, 1.0f));
    assert(approx_eq(r42.x, 160.0f, 1.0f));

    // Non-finite surface constraints (NaN and +Infinity) must produce finite, non-negative layouts.
    nve::fabric_host_set_surface_size(
        host,
        std::numeric_limits<float>::quiet_NaN(),
        std::numeric_limits<float>::infinity(),
        2.0f);
    for (const auto& snap : nve::fabric_host_snapshot_nodes(host)) {
      assert(std::isfinite(snap.layout.x) && std::isfinite(snap.layout.y));
      assert(std::isfinite(snap.layout.width) && snap.layout.width >= 0.0f);
      assert(std::isfinite(snap.layout.height) && snap.layout.height >= 0.0f);
    }

    nve::fabric_host_set_surface_size(
        host,
        std::numeric_limits<float>::infinity(),
        -std::numeric_limits<float>::infinity(),
        2.0f);
    for (const auto& snap : nve::fabric_host_snapshot_nodes(host)) {
      assert(std::isfinite(snap.layout.x) && std::isfinite(snap.layout.y));
      assert(std::isfinite(snap.layout.width) && snap.layout.width >= 0.0f);
      assert(std::isfinite(snap.layout.height) && snap.layout.height >= 0.0f);
    }

    nve_fabric_host_destroy(host);
  }
}


} // namespace nve_test
