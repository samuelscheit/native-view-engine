// Canonical 4-byte-aligned `FlatResource` (`nve.res.draw_list`) C++ codec: golden word layouts
// (byte-identical to `packages/modules/canvas/src/flat.rs`), full encode -> decode round trip of
// every opcode, unaligned-pointer safety, and strict rejection of every malformed input.
#include <cmath>
#include <cstdio>
#include <cstring>
#include <limits>
#include <vector>

#include <nve/canvas/canvas.hpp>
#include <nve/canvas/draw_list_encoder.hpp>

namespace {

int g_failures = 0;
#define CHECK(cond)                                                               \
  do {                                                                            \
    if (!(cond)) {                                                                \
      std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
      ++g_failures;                                                               \
    }                                                                             \
  } while (0)

using namespace nve::canvas;

void patch_u32(std::vector<uint8_t>& buf, size_t word_index, uint32_t value) {
  const size_t offset = word_index * 4u;
  for (unsigned i = 0u; i < 4u; ++i) {
    buf[offset + i] = static_cast<uint8_t>((value >> (8u * i)) & 0xFFu);
  }
}

void patch_f32(std::vector<uint8_t>& buf, size_t word_index, float value) {
  uint32_t bits = 0u;
  std::memcpy(&bits, &value, sizeof(uint32_t));
  patch_u32(buf, word_index, bits);
}

struct Recorder {
  std::vector<int> ops;
  DrawStrokePolylineOp poly{};
  std::vector<DrawPoint> points;
  DrawFillRoundedRectOp rounded{};
  void on_clear(const DrawClearOp& op) { ops.push_back(1); CHECK(op.color == DrawColorRgba::rgba(1, 2, 3, 4)); }
  void on_fill_rect(const DrawFillRectOp& op) { ops.push_back(2); CHECK(op.x == 1.0f && op.h == 4.0f); }
  void on_fill_rounded_rect(const DrawFillRoundedRectOp& op) { ops.push_back(3); rounded = op; }
  void on_fill_circle(const DrawFillCircleOp& op) { ops.push_back(4); CHECK(op.radius == 3.0f); }
  void on_stroke_line(const DrawStrokeLineOp& op) { ops.push_back(5); CHECK(op.x1 == 3.0f && op.width == 5.0f); }
  void on_stroke_polyline(const DrawStrokePolylineOp& op) {
    ops.push_back(6);
    poly = op;
    for (const DrawPoint p : op.points) points.push_back(p);
  }
  void on_clip_rect(const DrawClipRectOp& op) { ops.push_back(7); CHECK(op.w == 30.0f); }
  void on_restore_clip(const DrawRestoreClipOp&) { ops.push_back(8); }
};

std::vector<uint8_t> every_op() {
  const DrawPoint pts[] = {{0.0f, 0.0f}, {5.0f, 7.5f}, {-2.0f, 1e6f}};
  DrawListEncoder e;
  e.clear(DrawColorRgba::rgba(1, 2, 3, 4))
      .fill_rect(1.0f, 2.0f, 3.0f, 4.0f, DrawColorRgba::rgb(9, 9, 9))
      .fill_rounded_rect(0.0f, 0.0f, 10.0f, 10.0f, 2.5f, DrawColorRgba::rgb(1, 1, 1))
      .fill_circle(5.0f, 5.0f, 3.0f, DrawColorRgba::rgb(2, 2, 2))
      .stroke_line(0.0f, 0.0f, 3.0f, 3.0f, 5.0f, DrawColorRgba::rgb(3, 3, 3))
      .stroke_polyline(pts, 1.5f, DrawColorRgba::rgb(4, 4, 4))
      .clip_rect(0.0f, 0.0f, 30.0f, 40.0f)
      .restore_clip();
  return e.finish();
}

void test_golden_bytes() {
  // Empty list: single LE u32 op_count = 0.
  const std::vector<uint8_t> empty = DrawListEncoder{}.finish();
  CHECK((empty == std::vector<uint8_t>{0, 0, 0, 0}));

  // 1 Clear op: [op_count=1, OP_CLEAR=1, rgba(1,2,3,4)].
  DrawListEncoder e;
  e.clear(DrawColorRgba::rgba(1, 2, 3, 4));
  const std::vector<uint8_t> got = e.finish();
  const std::vector<uint8_t> want = {1, 0, 0, 0, 1, 0, 0, 0, 1, 2, 3, 4};
  CHECK(got == want);
  CHECK(kDrawListKind.value == nve::host::Key64::of("nve.res.draw_list").value);

  // Word counts match `packages/modules/canvas/src/flat.rs` exactly:
  // 1 (header) + 2 + 6 + 7 + 5 + 7 + (4 + 2*3) + 5 + 1 = 44 words = 176 bytes.
  CHECK(every_op().size() == 44u * 4u);
}

void test_round_trip() {
  const std::vector<uint8_t> bytes = every_op();
  Recorder r;
  const DrawListDecodeResult res = decode_draw_list(bytes, r);
  CHECK(res.ok() && res.ops_decoded == 8u && res.bytes_consumed == bytes.size());
  CHECK((r.ops == std::vector<int>{1, 2, 3, 4, 5, 6, 7, 8}));
  CHECK(r.rounded.radius == 2.5f);
  CHECK(r.points.size() == 3u && r.points[1] == (DrawPoint{5.0f, 7.5f}) && r.points[2].y == 1e6f);
  CHECK(r.poly.width == 1.5f && r.poly.points.size() == 3u);
  CHECK(validate_draw_list(DrawListEncoder{}.finish()).ok());  // empty list

  // Zero-vertex polyline is valid.
  DrawListEncoder zero_poly;
  zero_poly.stroke_polyline(std::span<const DrawPoint>(), 1.0f, DrawColorRgba::rgb(0, 0, 0));
  const std::vector<uint8_t> zp = zero_poly.finish();
  CHECK(zp.size() == 5u * 4u && validate_draw_list(zp).ok());

  // Unaligned base pointer (size still multiple of 4) decodes cleanly under ASan/UBSan.
  std::vector<uint8_t> shifted(bytes.size() + 1u, 0xAAu);
  std::memcpy(shifted.data() + 1u, bytes.data(), bytes.size());
  Recorder shifted_rec;
  const DrawListDecodeResult sres =
      decode_draw_list(std::span<const uint8_t>(shifted.data() + 1u, bytes.size()), shifted_rec);
  CHECK(sres.ok() && sres.ops_decoded == 8u && shifted_rec.points.size() == 3u);

  // Sparkline helper (0, 1, and multi-sample with NaN filtered) produces valid flat streams.
  const float samples[] = {1.0f, std::numeric_limits<float>::quiet_NaN(), 4.0f, 2.0f};
  DrawListEncoder sp0, sp1, spn;
  sp0.sparkline({}, 120.0f, 40.0f, DrawColorRgba::rgb(0, 0, 255), DrawColorRgba::rgb(255, 255, 255));
  sp1.sparkline(std::span<const float>(samples, 1u), 120.0f, 40.0f, DrawColorRgba::rgb(0, 0, 255),
                DrawColorRgba::rgb(255, 255, 255));
  spn.sparkline(samples, 120.0f, 40.0f, DrawColorRgba::rgb(0, 0, 255), DrawColorRgba::rgb(255, 255, 255));
  CHECK(validate_draw_list(sp0.finish()).ok());
  CHECK(validate_draw_list(sp1.finish()).ok());
  CHECK(validate_draw_list(spn.finish()).ok());
}

void test_rejections() {
  const std::vector<uint8_t> bytes = every_op();
  // Every prefix shorter than the full stream is rejected with the exact error.
  for (size_t cut = 0u; cut < bytes.size(); ++cut) {
    const DrawListDecodeResult r = validate_draw_list(std::span<const uint8_t>(bytes.data(), cut));
    CHECK(!r.ok());
    if (cut < kDrawListHeaderSize) {
      CHECK(r.error == DrawListError::Truncated);
    } else if ((cut & 3u) != 0u) {
      CHECK(r.error == DrawListError::Unaligned);
    } else {
      CHECK(r.error == DrawListError::Truncated);
    }
  }

  // Unaligned trailing bytes (1..3 extra bytes) vs aligned trailing word (4 extra bytes).
  for (size_t extra = 1u; extra <= 3u; ++extra) {
    std::vector<uint8_t> unaligned = bytes;
    unaligned.insert(unaligned.end(), extra, 0u);
    CHECK(validate_draw_list(unaligned).error == DrawListError::Unaligned);
  }
  std::vector<uint8_t> v = bytes;
  v.insert(v.end(), 4u, 0u);
  CHECK(validate_draw_list(v).error == DrawListError::TrailingBytes);

  // Unknown opcodes (including 0 and 0x101 to verify full u32 opcode check without u8 truncation).
  for (const uint32_t bad_op : {0u, 9u, 99u, 0x00000101u}) {
    v = bytes;
    patch_u32(v, 1u, bad_op);
    CHECK(validate_draw_list(v).error == DrawListError::UnknownOpcode);
  }

  // Header op count above the limit.
  v = DrawListEncoder{}.finish();
  patch_u32(v, 0u, static_cast<uint32_t>(kMaxDrawOps) + 1u);
  CHECK(validate_draw_list(v).error == DrawListError::OpCountExceedsLimit);

  // Insufficient words for declared op_count.
  v = DrawListEncoder{}.finish();
  patch_u32(v, 0u, 1u);
  CHECK(validate_draw_list(v).error == DrawListError::Truncated);

  // Retired NVE_DRAW1 payload is rejected (no backwards-compatibility shim).
  const std::vector<uint8_t> legacy = {
      'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1', 1, 0, 1, 0, 0, 0, 1, 1, 2, 3, 4};
  CHECK(!validate_draw_list(legacy).ok());

  // Non-finite floats: NaN, +Inf, -Inf in fixed-size op and polyline vertex / width.
  const float nan = std::numeric_limits<float>::quiet_NaN();
  const float inf = std::numeric_limits<float>::infinity();
  for (const float bad_f : {nan, inf, -inf}) {
    DrawListEncoder fe;
    fe.fill_rect(1.0f, 2.0f, 3.0f, 4.0f, DrawColorRgba::rgb(0, 0, 0));
    v = fe.finish();
    patch_f32(v, 2u, bad_f);  // word 0=op_count, 1=opcode, 2=x
    CHECK(validate_draw_list(v).error == DrawListError::NonFiniteFloat);
  }

  const DrawPoint one_pt[] = {{1.0f, 2.0f}};
  DrawListEncoder pe_bad;
  pe_bad.stroke_polyline(one_pt, 1.0f, DrawColorRgba::rgb(0, 0, 0));
  v = pe_bad.finish();
  patch_f32(v, 2u, nan);  // word 2=width
  CHECK(validate_draw_list(v).error == DrawListError::NonFiniteFloat);
  v = pe_bad.finish();
  patch_f32(v, 5u, inf);  // word 5=pt[0].x
  CHECK(validate_draw_list(v).error == DrawListError::NonFiniteFloat);
  v = pe_bad.finish();
  patch_f32(v, 6u, -inf);  // word 6=pt[0].y
  CHECK(validate_draw_list(v).error == DrawListError::NonFiniteFloat);

  // Polyline vertex count above the limit (and u32::MAX overflow guard).
  for (const uint32_t many : {static_cast<uint32_t>(kMaxPolylinePoints) + 1u, 0xFFFFFFFFu}) {
    DrawListEncoder pe;
    pe.stroke_polyline(std::span<const DrawPoint>(), 1.0f, DrawColorRgba::rgb(0, 0, 0));
    v = pe.finish();
    patch_u32(v, 4u, many);  // word 0=op_count, 1=opcode, 2=width, 3=rgba, 4=pt_count
    CHECK(validate_draw_list(v).error == DrawListError::PolylinePointCountExceedsLimit);
  }

  // Encoder sanitizes non-finite and negative radius/width input to 0.
  DrawListEncoder se;
  se.fill_circle(nan, inf, -inf, DrawColorRgba::rgb(0, 0, 0))
      .fill_rounded_rect(0.0f, 0.0f, 10.0f, 10.0f, -3.0f, DrawColorRgba::rgb(0, 0, 0))
      .stroke_line(0.0f, 0.0f, 5.0f, 5.0f, -2.0f, DrawColorRgba::rgb(0, 0, 0))
      .stroke_polyline(one_pt, -1.5f, DrawColorRgba::rgb(0, 0, 0));
  CHECK(validate_draw_list(se.finish()).ok());

  // Negative radius/width rejected with NegativeFloat; -0.0f and negative x/y/w/h accepted.
  {
    DrawListEncoder neg_rr;
    neg_rr.fill_rounded_rect(0.0f, 0.0f, 10.0f, 10.0f, 1.0f, DrawColorRgba::rgb(0, 0, 0));
    v = neg_rr.finish();
    patch_f32(v, 6u, -0.5f);  // radius
    CHECK(validate_draw_list(v).error == DrawListError::NegativeFloat);
    patch_f32(v, 6u, -0.0f);
    CHECK(validate_draw_list(v).ok());

    DrawListEncoder neg_c;
    neg_c.fill_circle(0.0f, 0.0f, 1.0f, DrawColorRgba::rgb(0, 0, 0));
    v = neg_c.finish();
    patch_f32(v, 4u, -1.0f);  // radius
    CHECK(validate_draw_list(v).error == DrawListError::NegativeFloat);

    DrawListEncoder neg_l;
    neg_l.stroke_line(0.0f, 0.0f, 1.0f, 1.0f, 1.0f, DrawColorRgba::rgb(0, 0, 0));
    v = neg_l.finish();
    patch_f32(v, 6u, -0.25f);  // width
    CHECK(validate_draw_list(v).error == DrawListError::NegativeFloat);

    DrawListEncoder neg_p;
    neg_p.stroke_polyline(one_pt, 1.0f, DrawColorRgba::rgb(0, 0, 0));
    v = neg_p.finish();
    patch_f32(v, 2u, -2.0f);  // width
    CHECK(validate_draw_list(v).error == DrawListError::NegativeFloat);

    DrawListEncoder neg_coords;
    neg_coords.fill_rect(-5.0f, -6.0f, -1.0f, -2.0f, DrawColorRgba::rgb(0, 0, 0))
        .clip_rect(-1.0f, -2.0f, -3.0f, -4.0f);
    CHECK(validate_draw_list(neg_coords.finish()).ok());
  }

  // Malformed second op invokes zero visitor callbacks (validate-before-stream).
  {
    DrawListEncoder two;
    two.clear(DrawColorRgba::rgba(1, 2, 3, 4))
        .fill_circle(5.0f, 5.0f, 2.0f, DrawColorRgba::rgb(1, 2, 3));
    v = two.finish();
    patch_f32(v, 6u, nan);  // circle radius word
    Recorder no_calls;
    const DrawListDecodeResult res = decode_draw_list(v, no_calls);
    CHECK(res.error == DrawListError::NonFiniteFloat && no_calls.ops.empty());
  }
}

void test_clip_depth_cap_and_unwind() {
  // 66 nested clips + 2 restores + clear: 2 restores absorb overflow, 64 unwind after clear.
  DrawListEncoder deep;
  for (uint32_t i = 0u; i < kMaxClipDepth + 2u; ++i) {
    deep.clip_rect(0.0f, 0.0f, 30.0f, 30.0f);
  }
  deep.restore_clip().restore_clip().clear(DrawColorRgba::rgba(1, 2, 3, 4));
  Recorder r;
  const DrawListDecodeResult res = decode_draw_list(deep.finish(), r);
  CHECK(res.ok());
  CHECK(r.ops.size() == static_cast<size_t>(kMaxClipDepth) * 2u + 1u);
  CHECK(r.ops[kMaxClipDepth] == 1);  // clear runs while clip #64 is still active
  for (size_t i = 0u; i < kMaxClipDepth; ++i) CHECK(r.ops[i] == 7);
  for (size_t i = kMaxClipDepth + 1u; i < r.ops.size(); ++i) CHECK(r.ops[i] == 8);

  // Unmatched leading restores are dropped; 3rd restore after 66 clips pops live clip #64.
  DrawListEncoder three;
  three.restore_clip();
  for (uint32_t i = 0u; i < kMaxClipDepth + 2u; ++i) {
    three.clip_rect(0.0f, 0.0f, 30.0f, 30.0f);
  }
  three.restore_clip().restore_clip().restore_clip().clear(DrawColorRgba::rgba(1, 2, 3, 4));
  Recorder r3;
  CHECK(decode_draw_list(three.finish(), r3).ok());
  CHECK(r3.ops.size() == static_cast<size_t>(kMaxClipDepth) * 2u + 1u);
  CHECK(r3.ops[kMaxClipDepth] == 8 && r3.ops[kMaxClipDepth + 1u] == 1);

  // Throwing visitor still unwinds all already-opened clips via RAII ScopeGuard.
  struct ThrowingVisitor : Recorder {
    void on_clip_rect(const DrawClipRectOp& op) {
      Recorder::on_clip_rect(op);
      if (ops.size() == 2u) throw 42;
    }
  };
  DrawListEncoder two_clips;
  two_clips.clip_rect(0.0f, 0.0f, 30.0f, 30.0f).clip_rect(0.0f, 0.0f, 30.0f, 30.0f);
  ThrowingVisitor tv;
  bool caught = false;
  try {
    (void)decode_draw_list(two_clips.finish(), tv);
  } catch (int code) {
    caught = (code == 42);
  }
  CHECK(caught && (tv.ops == std::vector<int>{7, 7, 8, 8}));
}

}  // namespace

int main() {
  test_golden_bytes();
  test_round_trip();
  test_rejections();
  test_clip_depth_cap_and_unwind();
  if (g_failures != 0) {
    std::fprintf(stderr, "nve_canvas_draw_list_test: %d failure(s)\n", g_failures);
    return 1;
  }
  std::printf("nve_canvas_draw_list_test: ok\n");
  return 0;
}
