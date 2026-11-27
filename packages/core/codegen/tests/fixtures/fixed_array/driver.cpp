// `[T; N]` over the generated C++20 bindings, against the live Rust provider: the `NveBinValue`
// codec (lossless; any other element count fails the reader, `Truncated` when shorter, `TooLarge`
// when longer), POD C arrays (nested too) in `to_abi` / `from_abi`, `Option<[T; N]>` and
// `Result<Option<[T; N]>, String>` returns, and `[T; N]` view props.
#include "FixedCodecHybridSpec.generated.hpp"

#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>

using namespace nve::hybrid;
using nve::host::BinError;
using nve::host::BinReader;
using nve::host::BinWriter;

#define REQUIRE(cond)                                                \
  do {                                                               \
    if (!(cond)) {                                                   \
      std::fprintf(stderr, "FAILED line %d: %s\n", __LINE__, #cond); \
      return 1;                                                      \
    }                                                                \
  } while (0)

// `{"<key>": [count x value]}`: a bundle whose `key` array has `count` entries.
static std::vector<uint8_t> bundle_with(const char* key, uint32_t count, bool points) {
  BinWriter w;
  const auto map = w.begin_map(1u);
  w.write_map_key(key);
  const auto arr = w.begin_array(count);
  for (uint32_t i = 0; i < count; ++i) {
    if (points) {
      FixedPoint{static_cast<int32_t>(i), -1}.encode_bin(w);
    } else {
      w.write_i64(i);
    }
  }
  w.end(arr);
  w.end(map);
  const auto bytes = w.bytes();
  return {bytes.begin(), bytes.end()};
}

static BinError decode_error(const std::vector<uint8_t>& bytes) {
  BinReader r(bytes);
  (void)FixedBundle::decode_bin(r);
  return r.error();
}

static FixedBundle sample_bundle() {
  FixedBundle b{};
  b.label = "fixed";
  b.coords = {1u, 0xFFFFFFFFu, 0u, 42u};
  b.points = {FixedPoint{-7, 8}, FixedPoint{2147483647, -2147483647 - 1}};
  b.matrix = {{{1u, 2u}, {3u, 4u}, {5u, 6u}}};
  b.tag = std::array<uint8_t, 3>{9u, 0u, 255u};
  return b;
}

static FixedPod sample_pod() {
  FixedPod p{};
  p.v = {1.5f, -2.0f, 0.0f, 3.25f};
  p.points = {FixedPoint{1, 2}, FixedPoint{3, 4}};
  p.kinds = {FixedKind::Gamma, FixedKind::Beta};
  p.grid = {{{10u, 11u}, {12u, 13u}, {14u, 15u}}};
  p.flags[0][0] = true;
  p.flags[0][32] = true;
  p.flags[1][5] = true;
  return p;
}

static int codec_checks() {
  const FixedBundle in = sample_bundle();
  BinWriter w;
  in.encode_bin(w);
  REQUIRE(w.ok());
  const auto encoded = w.bytes();
  const std::vector<uint8_t> bytes(encoded.begin(), encoded.end());
  REQUIRE(BinReader::validate(bytes) == BinError::None);
  BinReader r(bytes);
  const FixedBundle out = FixedBundle::decode_bin(r);
  REQUIRE(r.ok() && r.at_end());
  REQUIRE(out.label == "fixed" && out.coords == in.coords && out.matrix == in.matrix);
  REQUIRE(out.points[1].x == 2147483647 && out.points[1].y == -2147483647 - 1);
  REQUIRE(out.tag.has_value() && *out.tag == *in.tag);

  REQUIRE(decode_error(bundle_with("coords", 4u, false)) == BinError::None);
  REQUIRE(decode_error(bundle_with("coords", 3u, false)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("coords", 0u, false)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("coords", 5u, false)) == BinError::TooLarge);
  REQUIRE(decode_error(bundle_with("points", 2u, true)) == BinError::None);
  REQUIRE(decode_error(bundle_with("points", 1u, true)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("points", 3u, true)) == BinError::TooLarge);
  REQUIRE(decode_error(bundle_with("matrix", 2u, false)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("tag", 2u, false)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("tag", 4u, false)) == BinError::TooLarge);

  // POD C arrays convert element by element, nested dimensions included.
  const FixedPod pod = sample_pod();
  const NveHybridFixedPod raw = pod.to_abi();
  REQUIRE(raw.v[3] == 3.25f && raw.points[1].y == 4 && raw.grid[2][1] == 15u);
  REQUIRE(raw.kinds[0] == static_cast<uint32_t>(FixedKind::Gamma));
  REQUIRE(raw.flags[0][32] && raw.flags[1][5] && !raw.flags[1][4]);
  const FixedPod back = FixedPod::from_abi(raw);
  REQUIRE(back.v == pod.v && back.grid == pod.grid && back.flags == pod.flags);
  REQUIRE(back.kinds == pod.kinds && back.points[0].x == 1 && back.points[1].y == 4);
  return 0;
}

static int live_checks() {
  auto codec = FixedCodecClient::resolve(&nve_get_interface);
  const FixedBundle echoed = codec.echo(sample_bundle());
  REQUIRE(echoed.label == "fixed!");
  REQUIRE((echoed.coords == std::array<uint32_t, 4>{42u, 0u, 0xFFFFFFFFu, 1u}));
  REQUIRE(echoed.points[0].x == 2147483647 && echoed.points[1].x == -7);
  REQUIRE((echoed.matrix[2] == std::array<uint32_t, 2>{6u, 5u}));
  REQUIRE((echoed.tag == std::array<uint8_t, 3>{255u, 0u, 9u}));
  REQUIRE((codec.swap({1u, 2u, 3u, 4u}) == std::array<uint32_t, 4>{4u, 3u, 2u, 1u}));

  REQUIRE((codec.tag(true) == std::array<uint8_t, 3>{7u, 8u, 255u}));
  REQUIRE(!codec.tag(false).has_value());
  REQUIRE((codec.tryTag(true) == std::array<uint8_t, 3>{1u, 2u, 3u}));
  bool threw = false;
  try {
    (void)codec.tryTag(false);
  } catch (const std::exception& e) {
    threw = std::string(e.what()).find("no tag") != std::string::npos;
  }
  REQUIRE(threw);

  const FixedPod mirrored = codec.mirror(sample_pod());
  REQUIRE((mirrored.v == std::array<float, 4>{-1.5f, 2.0f, -0.0f, -3.25f}));
  REQUIRE(mirrored.points[0].x == 3 && mirrored.points[1].y == 2);
  REQUIRE((mirrored.kinds == std::array<FixedKind, 2>{FixedKind::Beta, FixedKind::Gamma}));
  REQUIRE((mirrored.grid[0] == std::array<uint32_t, 2>{11u, 12u}));
  REQUIRE(!mirrored.flags[0][0] && !mirrored.flags[0][32] && mirrored.flags[0][1] && !mirrored.flags[1][5]);

  auto view = FixedViewClient::resolve(&nve_get_interface);
  FixedViewProps props{};
  props.coords = {5u, 6u, 7u, 8u};
  props.tag = std::array<uint8_t, 3>{4u, 5u, 6u};
  props.pod = sample_pod();
  view.setProps(props);
  const FixedBundle summary = view.summary();
  REQUIRE(summary.label == "view:3" && summary.coords == props.coords);
  REQUIRE(summary.matrix == props.pod.grid && summary.points[1].x == 3);
  REQUIRE(summary.tag == props.tag);
  props.tag = std::nullopt;
  view.setProps(props);
  REQUIRE(!view.summary().tag.has_value());
  return 0;
}

int main() {
  if (codec_checks() != 0 || live_checks() != 0) return 1;
  std::puts("cpp fixed array ok");
  return 0;
}
