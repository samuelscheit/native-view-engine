// Generated flat `u32`-word `nve.res.draw_list` codec (`nve_draw_list.generated.hpp`) test suite:
// golden word layout, all 8 opcodes, unaligned pointer safety, strict rejection of malformed
// payloads (including NegativeFloat), and `kMaxClipDepth = 64` overflow balancing + RAII unwind.
#include <cstdio>
#include <cstring>
#include <limits>
#include <vector>

#include <nve/generated/nve_draw_list.generated.hpp>
#include <nve/host/xxh64.hpp>

namespace {

int g_failures = 0;
#define CHECK(cond)                                                               \
  do {                                                                            \
    if (!(cond)) {                                                                \
      std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
      ++g_failures;                                                               \
    }                                                                             \
  } while (0)

namespace dl = nve::generated::draw_list;

void push_u32(std::vector<uint8_t>& out, uint32_t v) {
  out.push_back(static_cast<uint8_t>(v & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 16u) & 0xFFu));
  out.push_back(static_cast<uint8_t>((v >> 24u) & 0xFFu));
}

void push_f32(std::vector<uint8_t>& out, float v) {
  uint32_t bits = 0u;
  std::memcpy(&bits, &v, sizeof(uint32_t));
  push_u32(out, bits);
}

std::vector<uint8_t> words_to_bytes(std::initializer_list<uint32_t> words) {
  std::vector<uint8_t> out;
  out.reserve(words.size() * 4u);
  for (const uint32_t w : words) push_u32(out, w);
  return out;
}

uint32_t fbits(float v) {
  uint32_t bits = 0u;
  std::memcpy(&bits, &v, sizeof(uint32_t));
  return bits;
}

struct Recorder {
  std::vector<uint32_t> ops;
  std::vector<dl::Point2> points;
  float last_radius = -1.0f;
  void on_clear(const dl::ClearOp& op) {
    ops.push_back(1u);
    CHECK(op.color == dl::ColorRgba::rgba(1u, 2u, 3u, 4u));
  }
  void on_fill_rect(const dl::FillRectOp& op) {
    ops.push_back(2u);
    CHECK(op.x == 1.0f && op.h == 4.0f);
  }
  void on_fill_rounded_rect(const dl::FillRoundedRectOp& op) {
    ops.push_back(3u);
    last_radius = op.radius;
  }
  void on_fill_circle(const dl::FillCircleOp& op) {
    ops.push_back(4u);
    CHECK(op.radius == 3.0f);
  }
  void on_stroke_line(const dl::StrokeLineOp& op) {
    ops.push_back(5u);
    CHECK(op.width == 5.0f);
  }
  void on_stroke_polyline(const dl::StrokePolylineOp& op) {
    ops.push_back(6u);
    for (const dl::Point2 pt : op.points) points.push_back(pt);
  }
  void on_clip_rect(const dl::ClipRectOp&) { ops.push_back(7u); }
  void on_restore_clip(const dl::RestoreClipOp&) { ops.push_back(8u); }
};

std::vector<uint8_t> build_all_8_ops() {
  std::vector<uint8_t> b;
  push_u32(b, 8u);
  // 1. Clear
  push_u32(b, 1u);
  push_u32(b, dl::ColorRgba::rgba(1u, 2u, 3u, 4u).to_u32());
  // 2. FillRect
  push_u32(b, 2u);
  push_f32(b, 1.0f);
  push_f32(b, 2.0f);
  push_f32(b, 3.0f);
  push_f32(b, 4.0f);
  push_u32(b, dl::ColorRgba::rgb(9u, 9u, 9u).to_u32());
  // 3. FillRoundedRect
  push_u32(b, 3u);
  push_f32(b, 0.0f);
  push_f32(b, 0.0f);
  push_f32(b, 10.0f);
  push_f32(b, 10.0f);
  push_f32(b, 2.5f);
  push_u32(b, dl::ColorRgba::rgb(1u, 1u, 1u).to_u32());
  // 4. FillCircle
  push_u32(b, 4u);
  push_f32(b, 5.0f);
  push_f32(b, 5.0f);
  push_f32(b, 3.0f);
  push_u32(b, dl::ColorRgba::rgb(2u, 2u, 2u).to_u32());
  // 5. StrokeLine
  push_u32(b, 5u);
  push_f32(b, 0.0f);
  push_f32(b, 0.0f);
  push_f32(b, 3.0f);
  push_f32(b, 3.0f);
  push_f32(b, 5.0f);
  push_u32(b, dl::ColorRgba::rgb(3u, 3u, 3u).to_u32());
  // 6. StrokePolyline (3 pts)
  push_u32(b, 6u);
  push_f32(b, 1.5f);
  push_u32(b, dl::ColorRgba::rgb(4u, 4u, 4u).to_u32());
  push_u32(b, 3u);
  push_f32(b, 0.0f);
  push_f32(b, 0.0f);
  push_f32(b, 5.0f);
  push_f32(b, 7.5f);
  push_f32(b, -2.0f);
  push_f32(b, 1e6f);
  // 7. ClipRect
  push_u32(b, 7u);
  push_f32(b, 0.0f);
  push_f32(b, 0.0f);
  push_f32(b, 30.0f);
  push_f32(b, 40.0f);
  // 8. RestoreClip
  push_u32(b, 8u);
  return b;
}

void test_golden_vectors() {
  CHECK(dl::kKind == nve::host::Key64::of("nve.res.draw_list").value);
  CHECK(dl::kHeaderSize == 4u);
  CHECK(dl::kMaxDrawOps == 65536u);
  CHECK(dl::kMaxPolylinePoints == 16384u);
  CHECK(dl::kMaxClipDepth == 64u);

  const std::vector<uint8_t> all8 = build_all_8_ops();
  CHECK(all8.size() == 44u * 4u);

  Recorder rec;
  const dl::DecodeResult res = dl::decode(all8, rec);
  CHECK(res.ok() && res.ops_decoded == 8u && res.bytes_consumed == all8.size());
  CHECK((rec.ops == std::vector<uint32_t>{1u, 2u, 3u, 4u, 5u, 6u, 7u, 8u}));
  CHECK(rec.last_radius == 2.5f && rec.points.size() == 3u && rec.points[1].y == 7.5f);

  // Unaligned base pointer with valid 4-byte-multiple size.
  std::vector<uint8_t> shifted(all8.size() + 3u, 0xCCu);
  std::memcpy(shifted.data() + 3u, all8.data(), all8.size());
  Recorder shifted_rec;
  CHECK(dl::decode(std::span<const uint8_t>(shifted.data() + 3u, all8.size()), shifted_rec).ok());
  CHECK(shifted_rec.points.size() == 3u);

  // Empty valid stream and 0-point polyline with -0.0f width.
  CHECK(dl::validate(words_to_bytes({0u})).ok());
  CHECK(dl::validate(words_to_bytes({1u, 6u, fbits(-0.0f), 0xFFFFFFFFu, 0u})).ok());

  // Malformed vectors.
  CHECK(dl::validate(std::span<const uint8_t>{}).error == dl::Error::Truncated);
  CHECK(dl::validate(std::vector<uint8_t>{1u, 0u, 0u}).error == dl::Error::Truncated);
  CHECK(dl::validate(std::vector<uint8_t>{0u, 0u, 0u, 0u, 1u}).error == dl::Error::Unaligned);
  CHECK(dl::validate(words_to_bytes({0u, 0u})).error == dl::Error::TrailingBytes);
  CHECK(dl::validate(words_to_bytes({65537u})).error == dl::Error::OpCountExceedsLimit);
  CHECK(dl::validate(words_to_bytes({1u, 0u})).error == dl::Error::UnknownOpcode);
  CHECK(dl::validate(words_to_bytes({1u, 9u})).error == dl::Error::UnknownOpcode);
  CHECK(dl::validate(words_to_bytes({1u, 6u, fbits(1.0f), 0u, 16385u})).error ==
        dl::Error::SliceLenExceedsLimit);

  const float nan = std::numeric_limits<float>::quiet_NaN();
  const float inf = std::numeric_limits<float>::infinity();
  CHECK(dl::validate(words_to_bytes({1u, 2u, fbits(nan), 0u, 0u, 0u, 0u})).error ==
        dl::Error::NonFiniteFloat);
  CHECK(dl::validate(words_to_bytes({1u, 4u, 0u, 0u, fbits(-inf), 0u})).error ==
        dl::Error::NonFiniteFloat);
  CHECK(dl::validate(words_to_bytes({1u, 4u, 0u, 0u, fbits(-0.1f), 0u})).error ==
        dl::Error::NegativeFloat);
  CHECK(dl::validate(words_to_bytes({1u, 3u, 0u, 0u, fbits(1.0f), fbits(1.0f), fbits(-1.0f), 0u}))
            .error == dl::Error::NegativeFloat);
  CHECK(dl::validate(words_to_bytes({1u, 5u, 0u, 0u, 0u, 0u, fbits(-1.0f), 0u})).error ==
        dl::Error::NegativeFloat);
  CHECK(dl::validate(words_to_bytes({1u, 6u, fbits(-1.0f), 0u, 0u})).error ==
        dl::Error::NegativeFloat);

  // Polyline bad point reports exact failing point byte offset (pos + p * 8 + 8), not total slice end.
  const dl::DecodeResult bad_pt = dl::validate(words_to_bytes({
      1u,
      6u,
      fbits(1.0f),
      0u,
      3u,
      fbits(0.0f),
      fbits(0.0f),
      fbits(nan),
      fbits(1.0f),
      fbits(2.0f),
      fbits(3.0f),
  }));
  CHECK(bad_pt.error == dl::Error::NonFiniteFloat && bad_pt.bytes_consumed == 20u + 2u * 8u);
}

void test_clip_balance_66_clips_2_restores() {
  std::vector<uint8_t> b;
  const uint32_t total_ops = dl::kMaxClipDepth + 2u + 2u + 1u;
  push_u32(b, total_ops);
  for (uint32_t i = 0u; i < dl::kMaxClipDepth + 2u; ++i) {
    push_u32(b, 7u);
    push_f32(b, 0.0f);
    push_f32(b, 0.0f);
    push_f32(b, 10.0f);
    push_f32(b, 10.0f);
  }
  push_u32(b, 8u);
  push_u32(b, 8u);
  push_u32(b, 1u);
  push_u32(b, dl::ColorRgba::rgba(1u, 2u, 3u, 4u).to_u32());

  Recorder r;
  CHECK(dl::decode(b, r).ok());
  CHECK(r.ops.size() == static_cast<size_t>(dl::kMaxClipDepth) * 2u + 1u);
  CHECK(r.ops[dl::kMaxClipDepth] == 1u);
  for (size_t i = 0u; i < dl::kMaxClipDepth; ++i) CHECK(r.ops[i] == 7u);
  for (size_t i = dl::kMaxClipDepth + 1u; i < r.ops.size(); ++i) CHECK(r.ops[i] == 8u);

  // Throw while overflowed (depth == 64, overflow == 2): exactly 64 restores unwind, NOT 66.
  std::vector<uint8_t> overflow_buf;
  push_u32(overflow_buf, dl::kMaxClipDepth + 2u + 1u);
  for (uint32_t i = 0u; i < dl::kMaxClipDepth + 2u; ++i) {
    push_u32(overflow_buf, 7u);
    push_f32(overflow_buf, 0.0f);
    push_f32(overflow_buf, 0.0f);
    push_f32(overflow_buf, 10.0f);
    push_f32(overflow_buf, 10.0f);
  }
  push_u32(overflow_buf, 1u);
  push_u32(overflow_buf, dl::ColorRgba::rgba(1u, 2u, 3u, 4u).to_u32());

  struct OverflowThrowingRecorder : Recorder {
    void on_clear(const dl::ClearOp& op) {
      Recorder::on_clear(op);
      throw 99;
    }
  };
  OverflowThrowingRecorder throwing;
  bool caught = false;
  try {
    (void)dl::decode(overflow_buf, throwing);
  } catch (int code) {
    caught = (code == 99);
  }
  CHECK(caught);
  CHECK(throwing.ops.size() == static_cast<size_t>(dl::kMaxClipDepth) * 2u + 1u);
  CHECK(throwing.ops[dl::kMaxClipDepth] == 1u);
  for (size_t i = 0u; i < dl::kMaxClipDepth; ++i) CHECK(throwing.ops[i] == 7u);
  for (size_t i = dl::kMaxClipDepth + 1u; i < throwing.ops.size(); ++i) CHECK(throwing.ops[i] == 8u);
}

}  // namespace

int main() {
  test_golden_vectors();
  test_clip_balance_66_clips_2_restores();
  if (g_failures != 0) {
    std::fprintf(stderr, "nve_host_draw_list_test: %d failure(s)\n", g_failures);
    return 1;
  }
  std::printf("nve_host_draw_list_test: ok\n");
  return 0;
}
