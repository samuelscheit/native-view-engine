// `NVE_DRAW1` C++ codec: golden bytes (shared with `packages/modules/canvas/src/codec.rs`), full
// encode -> decode round trip of every opcode, and strict rejection of every malformed input.
#include <cmath>
#include <cstdio>
#include <limits>
#include <vector>

#include <nve/canvas/canvas.hpp>
#include <nve/canvas/draw_list_encoder.hpp>

namespace {

int g_failures = 0;
#define CHECK(cond)                                                          \
  do {                                                                       \
    if (!(cond)) {                                                           \
      std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
      ++g_failures;                                                          \
    }                                                                        \
  } while (0)

using namespace nve::canvas;

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
  DrawListEncoder e;
  e.clear(DrawColorRgba::rgba(1, 2, 3, 4));
  const std::vector<uint8_t> got = e.finish();
  const std::vector<uint8_t> want = {'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1', 1, 0, 1, 0, 0, 0, 1, 1, 2, 3, 4};
  CHECK(got == want);
  CHECK(kDrawListKind.value == nve::host::Key64::of("nve.res.draw_list").value);
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
}

void test_rejections() {
  const std::vector<uint8_t> bytes = every_op();
  for (size_t cut = 0; cut < bytes.size(); ++cut) {
    CHECK(!validate_draw_list(std::span<const uint8_t>(bytes.data(), cut)).ok());
  }
  std::vector<uint8_t> v = bytes;
  v[0] = 'X';
  CHECK(validate_draw_list(v).error == DrawListError::BadMagic);
  v = bytes;
  v[9] = 2;
  CHECK(validate_draw_list(v).error == DrawListError::UnsupportedVersion);
  v = bytes;
  v.push_back(0);
  CHECK(validate_draw_list(v).error == DrawListError::TrailingBytes);
  v = bytes;
  v[15] = 99;  // first opcode
  CHECK(validate_draw_list(v).error == DrawListError::UnknownOpcode);

  // Header op count above the limit.
  v = DrawListEncoder{}.finish();
  const uint32_t too_many = static_cast<uint32_t>(kMaxDrawOps) + 1u;
  for (unsigned i = 0; i < 4u; ++i) v[11u + i] = static_cast<uint8_t>(too_many >> (8u * i));
  CHECK(validate_draw_list(v).error == DrawListError::OpCountExceedsLimit);

  // NaN coordinate (the encoder sanitizes, so patch the bytes): FillRect x of a 1-op list.
  DrawListEncoder fe;
  fe.fill_rect(1.0f, 2.0f, 3.0f, 4.0f, DrawColorRgba::rgb(0, 0, 0));
  v = fe.finish();
  const float nan = std::numeric_limits<float>::quiet_NaN();
  uint32_t bits = 0u;
  std::memcpy(&bits, &nan, 4);
  for (unsigned i = 0; i < 4u; ++i) v[16u + i] = static_cast<uint8_t>(bits >> (8u * i));
  CHECK(validate_draw_list(v).error == DrawListError::NonFiniteFloat);

  // Polyline vertex count above the limit.
  DrawListEncoder pe;
  pe.stroke_polyline(std::span<const DrawPoint>(), 1.0f, DrawColorRgba::rgb(0, 0, 0));
  v = pe.finish();
  const uint32_t many = static_cast<uint32_t>(kMaxPolylinePoints) + 1u;
  for (unsigned i = 0; i < 4u; ++i) v[15u + 1u + 8u + i] = static_cast<uint8_t>(many >> (8u * i));
  CHECK(validate_draw_list(v).error == DrawListError::PolylinePointCountExceedsLimit);

  // Encoder sanitizes non-finite input to 0 (matches `sanitize_finite`).
  DrawListEncoder se;
  se.fill_circle(nan, 1.0f, 1.0f, DrawColorRgba::rgb(0, 0, 0));
  CHECK(validate_draw_list(se.finish()).ok());
}

}  // namespace

int main() {
  test_golden_bytes();
  test_round_trip();
  test_rejections();
  if (g_failures != 0) {
    std::fprintf(stderr, "nve_canvas_draw_list_test: %d failure(s)\n", g_failures);
    return 1;
  }
  std::printf("nve_canvas_draw_list_test: ok\n");
  return 0;
}
