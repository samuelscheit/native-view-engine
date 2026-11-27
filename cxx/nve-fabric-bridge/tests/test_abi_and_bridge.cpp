#include "test_common.hpp"
#include "nve_draw_list.hpp"
#include "nve_host_runtime.hpp"
#include "nve_pair_client.hpp"
#include "nve_scene_abi.generated.h"
#include "nve_snapshot.hpp"

#include <array>
#include <cstddef>
#include <cstring>
#include <limits>

namespace nve_test {

// Compile-time ABI & wire-header layout checks across C11/C++20 structs and vtables.
static_assert(sizeof(NveSurfaceApi) == 4u * sizeof(void*), "NveSurfaceApi vtable size mismatch");
static_assert(offsetof(NveSurfaceApi, dispatch_events) == 3u * sizeof(void*), "NveSurfaceApi::dispatch_events slot mismatch");
static_assert(sizeof(NveLayoutApi) == 2u * sizeof(void*), "NveLayoutApi vtable size mismatch");
static_assert(offsetof(NveLayoutApi, hit_test) == sizeof(void*), "NveLayoutApi::hit_test slot mismatch");
static_assert(sizeof(NveComponentApi) == 2u * sizeof(void*), "NveComponentApi vtable size mismatch");
static_assert(offsetof(NveComponentApi, drain_emitted_events) == sizeof(void*), "NveComponentApi::drain_emitted_events slot mismatch");
static_assert(sizeof(NveDevApi) == 3u * sizeof(void*), "NveDevApi vtable size mismatch");
static_assert(offsetof(NveDevApi, restore_state) == 2u * sizeof(void*), "NveDevApi::restore_state slot mismatch");
static_assert(sizeof(NveHostApi) == 2u * sizeof(void*), "NveHostApi vtable size mismatch");
static_assert(offsetof(NveHostApi, dispatch_host_events) == sizeof(void*), "NveHostApi::dispatch_host_events slot mismatch");
static_assert(sizeof(NveHostCallbacks) == 3u * sizeof(void*), "NveHostCallbacks size mismatch");
static_assert(offsetof(NveHostCallbacks, channel_publish) == 2u * sizeof(void*), "NveHostCallbacks::channel_publish offset mismatch");
static_assert(sizeof(NveDirectEvent) == offsetof(NveDirectEvent, value) + sizeof(NveValue), "NveDirectEvent size must equal offsetof(value) + sizeof(NveValue)");
static_assert(sizeof(NveDirectEvent) == (sizeof(void*) == 8u ? 96u : (offsetof(NveValue, data) == 8u ? 72u : 68u)), "NveDirectEvent size must not grow");
static_assert(offsetof(NveDirectEvent, modifiers) == 18u, "NveDirectEvent::modifiers offset mismatch");
static_assert(offsetof(NveDirectEvent, event_flags) == 19u, "NveDirectEvent::event_flags offset mismatch");
static_assert(offsetof(NveDirectEvent, key_code) == 20u, "NveDirectEvent::key_code offset mismatch");
static_assert(offsetof(NveDirectEvent, name) == 24u, "NveDirectEvent::name offset mismatch");
static_assert(sizeof(NveEventBatch) == (sizeof(void*) == 8u ? 16u : 12u), "NveEventBatch size mismatch");
static_assert(sizeof(NveNodeMeta) == NVE_NODE_META_SIZE && NVE_NODE_META_SIZE == 64u, "NveNodeMeta size mismatch");
static_assert(sizeof(NveRenderNode) == NVE_RENDER_NODE_SIZE, "NveRenderNode size mismatch");
static_assert(offsetof(NveSceneView, removed_ids) == 40u + 4u * sizeof(void*), "NveSceneView::removed_ids offset mismatch");
static_assert(offsetof(NveSceneView, tables) == 40u + 5u * sizeof(void*), "NveSceneView::tables offset mismatch");
static_assert(sizeof(NveTableView) == (sizeof(void*) == 8u ? 48u : 36u), "NveTableView size mismatch");
static_assert(nve::kRelocHeaderSize == 64u, "NVERELOC header wire size mismatch");
static_assert(offsetof(nve::RelocHeader, flags) == nve::kSnapshotFlagsOffset, "RelocHeader::flags offset mismatch");
static_assert(nve::kWireHeaderSize == 16u, "NveWireFrame v2 header wire size mismatch");
static_assert(nve::kWireTrailerSize == 8u, "NveWireFrame v2 XXH64 trailer size mismatch");

// 1. Test all 10 NveOpcode variants + custom component hooks + snapshot inspection.
void test_all_opcodes_and_custom_components() {
  // Verify NVERELOC and NveWireFrame v2 headers encode and round-trip cleanly.
  {
    const uint8_t f0[4] = {0x4E, 0x56, 0x45, 0x31};
    const auto snap = nve::encode_snapshot(
        7u,
        3u,
        NveBufferSlice{f0, sizeof(f0)},
        NveBufferSlice{nullptr, 0u});
    assert(snap.size() >= nve::kRelocBundleMinSize);
    assert(std::memcmp(snap.data() + nve::kSnapshotMagicOffset, nve::kRelocMagic, 8u) == 0);
    nve::MmapSnapshot parsed;
    assert(nve::MmapSnapshot::from_bytes(snap, parsed));
    assert(parsed.header().app_schema_version == 7u);
    assert(parsed.header().flags == 3u);
    assert(parsed.bundle_root().frame0.offset == nve::kRelocBundleMinSize);
    assert(parsed.bundle_root().frame0.len == sizeof(f0));

    nve::WireHello hello{};
    hello.schema_version = 2u;
    hello.capabilities = 0x0Fu;
    hello.bearer_token = "pair-secret";
    hello.client_name = "abi-test";
    const auto hello_wire = nve::wire_v2::encode_hello(hello);
    nve::WireHello decoded_hello{};
    assert(nve::wire_v2::decode_hello(hello_wire, decoded_hello));
    assert(decoded_hello.bearer_token == hello.bearer_token);
    assert(decoded_hello.client_name == hello.client_name);

    nve::WireHelloAck ack{};
    ack.session_id = 42u;
    ack.schema_version = 2u;
    ack.server_capabilities = 0x07u;
    const auto ack_wire = nve::wire_v2::encode_hello_ack(ack);
    assert(ack_wire.size() == 12u);
    nve::WireHelloAck decoded_ack{};
    assert(nve::wire_v2::decode_hello_ack(ack_wire, decoded_ack));
    assert(decoded_ack.session_id == 42u);

    const std::array<uint8_t, 3> delta_body = {1u, 2u, 3u};
    const auto frame_wire = nve::wire_v2::encode_frame(
        nve::WireChannel::UiScene, nve::WireKind::SceneKeyframe, 10u, delta_body);
    assert(frame_wire.size() == nve::kWireHeaderSize + delta_body.size() + nve::kWireTrailerSize);
    nve::WireFrameView frame_view{};
    assert(nve::wire_v2::decode_frame(frame_wire, frame_view));
    assert(frame_view.header.sequence == 10u);
    assert(frame_view.payload.size() == delta_body.size());
  }
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

  // 1b. Shared zero-alloc NVE_DRAW1 binary decoder + FabricHost -> NveSceneView text_slab roundtrip.
  {
    nve::DrawListEncoder draw_enc;
    const std::array<nve::DrawPoint, 4> poly_pts = {
        nve::DrawPoint{12.0f, 50.0f},
        nve::DrawPoint{60.0f, 32.0f},
        nve::DrawPoint{120.0f, 42.0f},
        nve::DrawPoint{188.0f, 18.0f},
    };
    draw_enc.clear(nve::DrawColorRgba::rgba(11, 17, 32, 255))
        .clip_rect(4.0f, 4.0f, 200.0f, 80.0f)
        .fill_rect(8.0f, 12.0f, 40.0f, 24.0f, nve::DrawColorRgba::rgb(30, 41, 59))
        .fill_rounded_rect(10.0f, 10.0f, 180.0f, 60.0f, 12.0f, nve::DrawColorRgba::rgb(15, 23, 42))
        .stroke_line(12.0f, 40.0f, 188.0f, 40.0f, 1.5f, nve::DrawColorRgba::rgba(148, 163, 184, 128))
        .stroke_polyline(poly_pts, 2.5f, nve::DrawColorRgba::rgb(56, 189, 248))
        .fill_circle(188.0f, 18.0f, 4.0f, nve::DrawColorRgba::rgb(34, 197, 94))
        .restore_clip();
    assert(draw_enc.op_count() == 8u);
    const std::vector<uint8_t> valid_bytes = draw_enc.finish();
    // Exact byte-for-byte wire length matching Rust `canvas_ir.rs`:
    // 15 (hdr) + 5 (Clear) + 17 (ClipRect) + 21 (FillRect) + 25 (FillRoundedRect)
    // + 25 (StrokeLine) + 45 (StrokePolyline: 1+4+4+4+4*8) + 17 (FillCircle) + 1 (RestoreClip) = 171
    assert(valid_bytes.size() == 171u);
    assert(std::memcmp(valid_bytes.data(), "NVE_DRAW1", 9u) == 0);

    // Decode from an intentionally unaligned offset (+1 byte) to verify unaligned-safe memcpy reads.
    std::vector<uint8_t> unaligned(valid_bytes.size() + 1u, 0xCCu);
    std::memcpy(unaligned.data() + 1u, valid_bytes.data(), valid_bytes.size());
    const std::span<const uint8_t> unaligned_span(unaligned.data() + 1u, valid_bytes.size());

    nve::DrawListHeader hdr{};
    assert(nve::decode_draw_list_header(unaligned_span, hdr) == nve::DrawListError::Ok);
    assert(hdr.version == nve::kDrawListVersion);
    assert(hdr.op_count == 8u);

    struct RecordingVisitor {
      std::vector<uint8_t> order;
      nve::DrawClearOp clear_op{};
      nve::DrawClipRectOp clip_op{};
      nve::DrawFillRectOp fill_rect_op{};
      nve::DrawFillRoundedRectOp rounded_op{};
      nve::DrawStrokeLineOp line_op{};
      float poly_width{0.0f};
      nve::DrawColorRgba poly_color{};
      std::vector<nve::DrawPoint> poly_points;
      nve::DrawFillCircleOp circle_op{};

      void on_clear(const nve::DrawClearOp& op) {
        order.push_back(1u);
        clear_op = op;
      }
      void on_fill_rect(const nve::DrawFillRectOp& op) {
        order.push_back(2u);
        fill_rect_op = op;
      }
      void on_fill_rounded_rect(const nve::DrawFillRoundedRectOp& op) {
        order.push_back(3u);
        rounded_op = op;
      }
      void on_fill_circle(const nve::DrawFillCircleOp& op) {
        order.push_back(4u);
        circle_op = op;
      }
      void on_stroke_line(const nve::DrawStrokeLineOp& op) {
        order.push_back(5u);
        line_op = op;
      }
      void on_stroke_polyline(const nve::DrawStrokePolylineOp& op) {
        order.push_back(6u);
        poly_width = op.width;
        poly_color = op.color;
        poly_points.clear();
        for (const nve::DrawPoint pt : op.points) {
          poly_points.push_back(pt);
        }
        assert(op.points.size() == poly_points.size());
        for (size_t i = 0u; i < poly_points.size(); ++i) {
          assert(op.points[i] == poly_points[i]);
        }
      }
      void on_clip_rect(const nve::DrawClipRectOp& op) {
        order.push_back(7u);
        clip_op = op;
      }
      void on_restore_clip(const nve::DrawRestoreClipOp&) { order.push_back(8u); }
    };

    RecordingVisitor rec{};
    const auto dec = nve::decode_draw_list(unaligned_span, rec);
    assert(dec.ok());
    assert(dec.ops_decoded == 8u);
    assert(dec.bytes_consumed == valid_bytes.size());
    assert((rec.order == std::vector<uint8_t>{1u, 7u, 2u, 3u, 5u, 6u, 4u, 8u}));
    assert(rec.clear_op.color == nve::DrawColorRgba::rgba(11, 17, 32, 255));
    assert(approx_eq(rec.clip_op.x, 4.0f) && approx_eq(rec.clip_op.w, 200.0f));
    assert(approx_eq(rec.fill_rect_op.x, 8.0f) && approx_eq(rec.fill_rect_op.h, 24.0f));
    assert(rec.fill_rect_op.color == nve::DrawColorRgba::rgb(30, 41, 59));
    assert(approx_eq(rec.rounded_op.radius, 12.0f));
    assert(approx_eq(rec.line_op.width, 1.5f));
    assert(rec.line_op.color == nve::DrawColorRgba::rgba(148, 163, 184, 128));
    assert(approx_eq(rec.poly_width, 2.5f));
    assert(rec.poly_color == nve::DrawColorRgba::rgb(56, 189, 248));
    assert(rec.poly_points.size() == 4u);
    assert(approx_eq(rec.poly_points[0].x, 12.0f) && approx_eq(rec.poly_points[0].y, 50.0f));
    assert(approx_eq(rec.poly_points[3].x, 188.0f) && approx_eq(rec.poly_points[3].y, 18.0f));
    assert(approx_eq(rec.circle_op.cx, 188.0f) && approx_eq(rec.circle_op.radius, 4.0f));
    assert(rec.circle_op.color == nve::DrawColorRgba::rgb(34, 197, 94));

    // Base64 attr string roundtrip (`nve-draw1;b64,...` and bare base64).
    const std::string attr_b64 = nve::encode_draw_list_attr_base64(valid_bytes);
    assert(attr_b64.rfind("nve-draw1;b64,", 0) == 0);
    std::vector<uint8_t> decoded_b64;
    assert(nve::decode_draw_list_attr_bytes(attr_b64, decoded_b64) == nve::DrawListError::Ok);
    assert(decoded_b64 == valid_bytes);
    const std::string_view bare_b64 =
        std::string_view(attr_b64).substr(nve::kDrawListAttrPrefix.size());
    assert(nve::decode_draw_list_attr_bytes(bare_b64, decoded_b64) == nve::DrawListError::Ok);
    assert(decoded_b64 == valid_bytes);
    assert(nve::decode_draw_list_attr_bytes("nve-draw1;b64,!!!", decoded_b64) ==
           nve::DrawListError::InvalidBase64);
    assert(nve::decode_draw_list_attr_bytes("nve-draw1;b64,AAAA", decoded_b64) ==
           nve::DrawListError::Truncated);

    // Every single truncation offset must fail with `Truncated`.
    for (size_t cut = 0u; cut < valid_bytes.size(); ++cut) {
      const auto r = nve::validate_draw_list(std::span<const uint8_t>(valid_bytes.data(), cut));
      assert(r.error == nve::DrawListError::Truncated);
    }

    // Trailing byte rejected.
    {
      std::vector<uint8_t> trailing = valid_bytes;
      trailing.push_back(0xAAu);
      assert(nve::validate_draw_list(trailing).error == nve::DrawListError::TrailingBytes);
    }

    // Bad magic & unsupported version rejected.
    {
      std::vector<uint8_t> bad_magic = valid_bytes;
      bad_magic[0] = 'X';
      assert(nve::validate_draw_list(bad_magic).error == nve::DrawListError::BadMagic);

      std::vector<uint8_t> bad_ver = valid_bytes;
      bad_ver[9] = 99u;
      bad_ver[10] = 0u;
      assert(nve::validate_draw_list(bad_ver).error == nve::DrawListError::UnsupportedVersion);
    }

    // Oversized op_count (> kMaxDrawOps) and lying op_count rejected without allocating.
    {
      std::vector<uint8_t> huge_ops(valid_bytes.begin(), valid_bytes.begin() + 15);
      const uint32_t over = static_cast<uint32_t>(nve::kMaxDrawOps + 1u);
      std::memcpy(huge_ops.data() + 11u, &over, sizeof(uint32_t));
      assert(nve::validate_draw_list(huge_ops).error == nve::DrawListError::OpCountExceedsLimit);

      const uint32_t lying = static_cast<uint32_t>(nve::kMaxDrawOps);
      std::memcpy(huge_ops.data() + 11u, &lying, sizeof(uint32_t));
      assert(nve::validate_draw_list(huge_ops).error == nve::DrawListError::Truncated);
    }

    // Oversized polyline vertex count rejected without allocating.
    {
      std::vector<uint8_t> huge_poly(valid_bytes.begin(), valid_bytes.begin() + 15);
      const uint32_t one = 1u;
      std::memcpy(huge_poly.data() + 11u, &one, sizeof(uint32_t));
      huge_poly.push_back(static_cast<uint8_t>(nve::DrawOpcode::StrokePolyline));
      const float w = 2.0f;
      const uint32_t c = 0xFFFFFFFFu;
      const uint32_t pts = static_cast<uint32_t>(nve::kMaxPolylinePoints + 1u);
      const uint8_t* pw = reinterpret_cast<const uint8_t*>(&w);
      const uint8_t* pc = reinterpret_cast<const uint8_t*>(&c);
      const uint8_t* pp = reinterpret_cast<const uint8_t*>(&pts);
      huge_poly.insert(huge_poly.end(), pw, pw + 4u);
      huge_poly.insert(huge_poly.end(), pc, pc + 4u);
      huge_poly.insert(huge_poly.end(), pp, pp + 4u);
      assert(nve::validate_draw_list(huge_poly).error ==
             nve::DrawListError::PolylinePointCountExceedsLimit);
    }

    // Unknown opcode rejected.
    {
      std::vector<uint8_t> bad_op(valid_bytes.begin(), valid_bytes.begin() + 15);
      const uint32_t one = 1u;
      std::memcpy(bad_op.data() + 11u, &one, sizeof(uint32_t));
      bad_op.push_back(0xFEu);
      assert(nve::validate_draw_list(bad_op).error == nve::DrawListError::UnknownOpcode);
    }

    // Non-finite float (NaN, +Inf, -Inf) rejected in scalar op AND inside StrokePolyline vertex.
    const std::array<float, 3> bad_floats = {
        std::numeric_limits<float>::quiet_NaN(),
        std::numeric_limits<float>::infinity(),
        -std::numeric_limits<float>::infinity(),
    };
    for (float bf : bad_floats) {
      nve::DrawListEncoder circle_enc;
      circle_enc.fill_circle(10.0f, 10.0f, 5.0f, nve::DrawColorRgba::rgb(255, 255, 255));
      std::vector<uint8_t> buf = circle_enc.finish();
      std::memcpy(buf.data() + 16u, &bf, sizeof(float));
      assert(nve::validate_draw_list(buf).error == nve::DrawListError::NonFiniteFloat);

      nve::DrawListEncoder poly_enc;
      const std::array<nve::DrawPoint, 2> pts = {
          nve::DrawPoint{1.0f, 2.0f},
          nve::DrawPoint{3.0f, 4.0f},
      };
      poly_enc.stroke_polyline(pts, 2.0f, nve::DrawColorRgba::rgb(255, 255, 255));
      std::vector<uint8_t> pbuf = poly_enc.finish();
      // Header(15) + tag(1) + width(4) + color(4) + count(4) + p0(8) = offset 36 (p1.x)
      std::memcpy(pbuf.data() + 36u, &bf, sizeof(float));
      assert(nve::validate_draw_list(pbuf).error == nve::DrawListError::NonFiniteFloat);
    }

    // Deterministic xorshift64 fuzzing across 1,000 mutated buffers (zero crashes / UB).
    uint64_t rng = 0x5EEDCAFEBABE1234ULL;
    auto next_u64 = [&]() -> uint64_t {
      rng ^= rng << 13u;
      rng ^= rng >> 7u;
      rng ^= rng << 17u;
      return rng;
    };
    for (size_t iter = 0u; iter < 1000u; ++iter) {
      std::vector<uint8_t> m = valid_bytes;
      const size_t flips = static_cast<size_t>(next_u64() % 4u + 1u);
      for (size_t f = 0u; f < flips; ++f) {
        m[static_cast<size_t>(next_u64() % m.size())] ^=
            static_cast<uint8_t>(next_u64() & 0xFFu);
      }
      m.resize(static_cast<size_t>(next_u64() % (m.size() + 1u)));
      (void)nve::validate_draw_list(m);
    }

    // FabricHost -> NveSceneView text_slab roundtrip for NVE_COMP_SKIA_CANVAS (zero ABI break).
    NveSurfaceConfig scfg{
        .surface_id = 77u, .width = 320.0f, .height = 160.0f, .scale_factor = 2.0f};
    NveFabricHostHandle canvas_host = nve_fabric_host_create(scfg);
    assert(canvas_host != nullptr);

    nve::DrawListEncoder spark_enc;
    const std::array<float, 6> samples = {12.0f, 28.0f, 19.0f, 45.0f, 36.0f, 62.0f};
    spark_enc.sparkline(
        samples,
        240.0f,
        80.0f,
        nve::DrawColorRgba::rgb(56, 189, 248),
        nve::DrawColorRgba::rgb(15, 23, 42));
    const std::vector<uint8_t> spark_bytes = spark_enc.finish();

    // Frame 1: raw binary `drawList` via RawPropValue::Bytes.
    CommandBufferEncoder f1(77u, 1u);
    f1.create_node(2u, NVE_COMP_SKIA_CANVAS);
    nve::LayoutProps cl{};
    cl.width = {1u, 240.0f};
    cl.height = {1u, 80.0f};
    f1.set_layout(2u, cl);
    f1.set_raw_prop_bytes(2u, "drawList", spark_bytes);
    f1.set_children(NVE_ROOT_NODE_ID, {2u});
    f1.commit(true);
    assert(nve_fabric_host_submit_frame(canvas_host, f1.finish(), nullptr) == 0);

    const NveSceneView sv1 = nve::fabric_host_scene_view(canvas_host);
    assert(sv1.schema_hash == NVE_SCENE_SCHEMA_HASH);
    const NveRenderNode* canvas_node1 = nullptr;
    for (uint32_t i = 0u; i < sv1.node_count; ++i) {
      if (sv1.nodes[i].node_id == 2u) {
        canvas_node1 = &sv1.nodes[i];
        break;
      }
    }
    assert(canvas_node1 != nullptr);
    assert(canvas_node1->kind == NVE_NODE_KIND_SKIA_CANVAS);
    const std::string_view slab_sv1 = nve::scene_node_text(sv1, *canvas_node1);
    assert(slab_sv1.size() == spark_bytes.size());
    assert(std::memcmp(slab_sv1.data(), spark_bytes.data(), spark_bytes.size()) == 0);
    assert(nve::validate_draw_list(
               std::span<const uint8_t>(
                   reinterpret_cast<const uint8_t*>(slab_sv1.data()), slab_sv1.size()))
               .ok());

    // Frame 2: identical `drawList` bytes must NOT dirty the scene node.
    CommandBufferEncoder f2(77u, 2u);
    f2.set_raw_prop_bytes(2u, "drawList", spark_bytes);
    f2.commit(true);
    assert(nve_fabric_host_submit_frame(canvas_host, f2.finish(), nullptr) == 0);
    const NveSceneView sv2 = nve::fabric_host_scene_view(canvas_host);
    assert(sv2.dirty_count == 0u);

    // Frame 3: update via `draw_list` snake_case key with `nve-draw1;b64,` base64 string fallback.
    CommandBufferEncoder f3(77u, 3u);
    f3.set_raw_prop_string(2u, "draw_list", attr_b64);
    f3.commit(true);
    assert(nve_fabric_host_submit_frame(canvas_host, f3.finish(), nullptr) == 0);
    const NveSceneView sv3 = nve::fabric_host_scene_view(canvas_host);
    assert(sv3.dirty_count == 1u);
    const NveRenderNode& dirty_node3 = sv3.nodes[sv3.dirty_indices[0]];
    assert(dirty_node3.node_id == 2u);
    const std::string_view slab_sv3 = nve::scene_node_text(sv3, dirty_node3);
    assert(slab_sv3.size() == valid_bytes.size());
    assert(std::memcmp(slab_sv3.data(), valid_bytes.data(), valid_bytes.size()) == 0);

    // Verify Frame-0 serialization preserves `draw_list` across replay into a fresh host.
    const std::vector<uint8_t> frame0_buf = nve::fabric_host_serialize_frame0(canvas_host, 3u);
    NveFabricHostHandle replay_host = nve_fabric_host_create(scfg);
    assert(nve_fabric_host_submit_frame(
               replay_host, NveBufferSlice{frame0_buf.data(), frame0_buf.size()}, nullptr) == 0);
    const NveSceneView rsv = nve::fabric_host_scene_view(replay_host);
    bool found_replayed_canvas = false;
    for (uint32_t i = 0u; i < rsv.node_count; ++i) {
      if (rsv.nodes[i].node_id == 2u) {
        const std::string_view rslab = nve::scene_node_text(rsv, rsv.nodes[i]);
        assert(rslab.size() == valid_bytes.size());
        assert(std::memcmp(rslab.data(), valid_bytes.data(), valid_bytes.size()) == 0);
        found_replayed_canvas = true;
      }
    }
    assert(found_replayed_canvas);
    nve_fabric_host_destroy(replay_host);

    // Frame 4: clear `drawList` via null -> text_slab span becomes empty and marks node dirty.
    CommandBufferEncoder f4(77u, 4u);
    f4.set_raw_prop_null(2u, "drawList");
    f4.commit(true);
    assert(nve_fabric_host_submit_frame(canvas_host, f4.finish(), nullptr) == 0);
    const NveSceneView sv4 = nve::fabric_host_scene_view(canvas_host);
    assert(sv4.dirty_count == 1u);
    assert(sv4.nodes[sv4.dirty_indices[0]].text_len == 0u);

    nve_fabric_host_destroy(canvas_host);
  }

  // 1c. EventRingWriter hardening checks (null+nonzero len, narrowing guards, unknown payload_tag).
  {
    nve::EventRingWriter ring;
    ring.clear();

    // nullptr with nonzero length MUST be rejected without corrupting framing or count!
    assert(!ring.push_custom(1u, 10u, 100u, "badNull", nullptr, 32u));
    assert(ring.count() == 0u);
    assert(ring.finalize().size() == 4u);

    // nullptr with zero length is valid and roundtrips cleanly with empty payload.
    assert(ring.push_custom(1u, 10u, 101u, "emptyOk", nullptr, 0u));
    assert(ring.count() == 1u);

    // Name exceeding uint16_t max MUST be rejected cleanly without mutating ring.
    const std::string oversized_name(70000u, 'x');
    const uint8_t one_byte = 0x42u;
    assert(!ring.push_custom(1u, 10u, 102u, oversized_name, &one_byte, 1u));
    assert(!ring.push_custom_value(1u, 10u, 103u, oversized_name, nve::make_bool_value(true)));
    assert(ring.count() == 1u);

    // Unknown payload_tag in push_direct_event MUST be rejected (not silently mapped to Custom).
    for (uint8_t bad_tag : {uint8_t{9u}, uint8_t{42u}, uint8_t{255u}}) {
      NveDirectEvent bad_ev{};
      bad_ev.surface_id = 1u;
      bad_ev.node_id = 10u;
      bad_ev.timestamp_us = 104u;
      bad_ev.kind = static_cast<uint8_t>(nve::EventKind::Custom);
      bad_ev.payload_tag = bad_tag;
      bad_ev.name = nve::make_str_slice("shouldReject");
      bad_ev.value = nve::make_i64_value(7);
      assert(!ring.push_direct_event(bad_ev));
    }
    assert(ring.count() == 1u);

    // Valid direct events across tags 0..5 roundtrip accurately via templated write_le<T>.
    NveDirectEvent ev_press{};
    ev_press.surface_id = 1u;
    ev_press.node_id = 11u;
    ev_press.timestamp_us = 200u;
    ev_press.kind = static_cast<uint8_t>(nve::EventKind::Press);
    ev_press.payload_tag = 1u;
    ev_press.x = 12.5f;
    ev_press.y = 34.25f;
    assert(ring.push_direct_event(ev_press));

    NveDirectEvent ev_text{};
    ev_text.surface_id = 1u;
    ev_text.node_id = 12u;
    ev_text.timestamp_us = 201u;
    ev_text.kind = static_cast<uint8_t>(nve::EventKind::TextChange);
    ev_text.payload_tag = 3u;
    ev_text.text = nve::make_str_slice("hello-hardened");
    assert(ring.push_direct_event(ev_text));

    NveDirectEvent ev_custom{};
    ev_custom.surface_id = 1u;
    ev_custom.node_id = 13u;
    ev_custom.timestamp_us = 202u;
    ev_custom.kind = static_cast<uint8_t>(nve::EventKind::Custom);
    ev_custom.payload_tag = 5u;
    ev_custom.name = nve::make_str_slice("onMetric");
    ev_custom.value = nve::make_i64_value(9001);
    assert(ring.push_direct_event(ev_custom));

    assert(ring.count() == 4u);
    const auto& bytes = ring.finalize();
    std::vector<nve::DecodedEvent> decoded;
    assert(nve::decode_event_ring(NveBufferSlice{bytes.data(), bytes.size()}, decoded));
    assert(decoded.size() == 4u);
    assert(decoded[0].kind == nve::EventKind::Custom && decoded[0].name == "emptyOk" &&
           decoded[0].payload_bytes.empty());
    assert(decoded[1].kind == nve::EventKind::Press && decoded[1].node_id == 11u &&
           approx_eq(decoded[1].x, 12.5f, 1e-4f) && approx_eq(decoded[1].y, 34.25f, 1e-4f));
    assert(decoded[2].kind == nve::EventKind::TextChange && decoded[2].text == "hello-hardened");
    assert(decoded[3].kind == nve::EventKind::Custom && decoded[3].name == "onMetric");
    const auto v3 = nve::BinValueView::parse(
        decoded[3].payload_bytes.data(), decoded[3].payload_bytes.size());
    assert(v3.has_value() && v3->as_i64() == 9001);

    // Typed KeyDown, PointerDown, and Wheel direct events roundtrip through EventRingWriter & decode_event_ring.
    nve::EventRingWriter typed_ring;
    typed_ring.clear();
    NveDirectEvent ev_key{.surface_id = 1u, .node_id = 14u, .timestamp_us = 300u,
        .kind = NVE_EVENT_KEY_DOWN, .payload_tag = NVE_EVENT_PAYLOAD_KEY,
        .modifiers = NVE_KEY_MOD_META | NVE_KEY_MOD_SHIFT, .event_flags = NVE_EVENT_FLAG_COMPOSING,
        .key_code = 75u, .name = nve::make_str_slice("KeyK"), .text = nve::make_str_slice("k")};
    NveDirectEvent ev_ptr{.surface_id = 1u, .node_id = 15u, .timestamp_us = 301u,
        .kind = NVE_EVENT_POINTER_DOWN, .payload_tag = NVE_EVENT_PAYLOAD_POINTER,
        .modifiers = NVE_KEY_MOD_CTRL, .event_flags = 1u | NVE_EVENT_FLAG_CAPTURE,
        .x = 120.5f, .y = 48.25f};
    NveDirectEvent ev_wheel{.surface_id = 1u, .node_id = 16u, .timestamp_us = 302u,
        .kind = NVE_EVENT_WHEEL, .payload_tag = NVE_EVENT_PAYLOAD_WHEEL,
        .modifiers = NVE_KEY_MOD_ALT, .x = -4.0f, .y = 24.5f};
    assert(typed_ring.push_direct_event(ev_key) && typed_ring.push_direct_event(ev_ptr) && typed_ring.push_direct_event(ev_wheel));
    const auto& typed_bytes = typed_ring.finalize();
    std::vector<nve::DecodedEvent> td;
    assert(nve::decode_event_ring(NveBufferSlice{typed_bytes.data(), typed_bytes.size()}, td) && td.size() == 3u);
    assert(td[0].kind == nve::EventKind::KeyDown && td[0].text == "k" && td[0].code == "KeyK" &&
           td[0].key_code == 75u && td[0].modifiers == (NVE_KEY_MOD_META | NVE_KEY_MOD_SHIFT) &&
           td[0].event_flags == NVE_EVENT_FLAG_COMPOSING);
    assert(td[1].kind == nve::EventKind::PointerDown && approx_eq(td[1].x, 120.5f, 1e-4f) &&
           approx_eq(td[1].y, 48.25f, 1e-4f) && td[1].modifiers == NVE_KEY_MOD_CTRL &&
           td[1].event_flags == (1u | NVE_EVENT_FLAG_CAPTURE));
    assert(td[2].kind == nve::EventKind::Wheel && approx_eq(td[2].x, -4.0f, 1e-4f) &&
           approx_eq(td[2].y, 24.5f, 1e-4f) && td[2].modifiers == NVE_KEY_MOD_ALT);
  }
}

} // namespace nve_test
