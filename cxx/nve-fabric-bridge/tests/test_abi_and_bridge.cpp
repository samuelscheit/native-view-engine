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
static_assert(sizeof(NveSurfaceApi) == 5u * sizeof(void*), "NveSurfaceApi vtable size mismatch");
static_assert(offsetof(NveSurfaceApi, dispatch_events_direct) == 4u * sizeof(void*), "NveSurfaceApi::dispatch_events_direct slot mismatch");
static_assert(sizeof(NveLayoutApi) == 2u * sizeof(void*), "NveLayoutApi vtable size mismatch");
static_assert(offsetof(NveLayoutApi, hit_test) == sizeof(void*), "NveLayoutApi::hit_test slot mismatch");
static_assert(sizeof(NveComponentApi) == 3u * sizeof(void*), "NveComponentApi vtable size mismatch");
static_assert(offsetof(NveComponentApi, set_props_direct) == 2u * sizeof(void*), "NveComponentApi::set_props_direct slot mismatch");
static_assert(sizeof(NveDevApi) == 3u * sizeof(void*), "NveDevApi vtable size mismatch");
static_assert(offsetof(NveDevApi, restore_state) == 2u * sizeof(void*), "NveDevApi::restore_state slot mismatch");
static_assert(sizeof(NveHostApi) == 2u * sizeof(void*), "NveHostApi vtable size mismatch");
static_assert(offsetof(NveHostApi, dispatch_host_events) == sizeof(void*), "NveHostApi::dispatch_host_events slot mismatch");
static_assert(sizeof(NveHostCallbacks) == 3u * sizeof(void*), "NveHostCallbacks size mismatch");
static_assert(offsetof(NveHostCallbacks, channel_publish) == 2u * sizeof(void*), "NveHostCallbacks::channel_publish offset mismatch");
static_assert(sizeof(NveRenderNode) == NVE_RENDER_NODE_SIZE, "NveRenderNode size mismatch");
static_assert(offsetof(NveSceneView, removed_ids) == 40u + 4u * sizeof(void*), "NveSceneView::removed_ids offset mismatch");
static_assert(nve::kRelocHeaderSize == 64u, "NVERELOC header wire size mismatch");
static_assert(offsetof(nve::RelocHeader, flags) == nve::kSnapshotFlagsOffset, "RelocHeader::flags offset mismatch");
static_assert(sizeof(nve::HelloPayload) == nve::kHelloPayloadWireSize, "HelloPayload size mismatch");
static_assert(offsetof(nve::HelloPayload, token_proof) == 16u, "HelloPayload::token_proof offset mismatch");
static_assert(offsetof(nve::HelloAckPayload, resync_required) == 16u, "HelloAckPayload::resync_required offset mismatch");
static_assert(offsetof(nve::DeltaPacketHeader, payload_len) == 16u, "DeltaPacketHeader::payload_len offset mismatch");

// 1. Test all 10 NveOpcode variants + custom component hooks + snapshot inspection.
void test_all_opcodes_and_custom_components() {
  // Verify NVERELOC and pair/NVD1 wire headers encode at their exact asserted offsets.
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

    nve::HelloPayload hello{};
    hello.schema_version = 2u;
    hello.max_window = 32u;
    hello.client_last_seq = 9u;
    hello.client_nonce = 0x1122334455667788ULL;
    const auto hello_wire = nve::pair_wire::encode_hello_payload(hello);
    assert(hello_wire.size() == nve::kHelloPayloadWireSize);
    nve::HelloPayload decoded_hello{};
    assert(nve::pair_wire::decode_hello_payload(hello_wire, decoded_hello));
    assert(decoded_hello.client_nonce == hello.client_nonce);

    nve::HelloAckPayload ack{};
    ack.schema_version = 2u;
    ack.server_max_window = 16u;
    ack.server_seq = 5u;
    ack.server_nonce = 0x8877665544332211ULL;
    ack.resync_required = true;
    const auto ack_wire = nve::pair_wire::encode_hello_ack_payload(ack);
    assert(ack_wire.size() == nve::kHelloAckPayloadWireSize);
    nve::HelloAckPayload decoded_ack{};
    assert(nve::pair_wire::decode_hello_ack_payload(ack_wire, decoded_ack));
    assert(decoded_ack.resync_required);

    const std::array<uint8_t, 3> delta_body = {1u, 2u, 3u};
    const auto nvd_wire = nve::pair_wire::encode_nvd_packet(
        nve::DeltaPacketKind::Keyframe, 0u, 10u, 0u, 0u, delta_body);
    assert(nvd_wire.size() == nve::kDeltaHeaderSize + delta_body.size());
    nve::DeltaPacketView nvd_view{};
    assert(nve::pair_wire::parse_nvd_packet(nvd_wire, nvd_view));
    assert(nvd_view.header.seq == 10u);
    assert(nvd_view.header.payload_len == delta_body.size());
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
    for (uint8_t bad_tag : {uint8_t{6u}, uint8_t{42u}, uint8_t{255u}}) {
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
  }
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

} // namespace nve_test
