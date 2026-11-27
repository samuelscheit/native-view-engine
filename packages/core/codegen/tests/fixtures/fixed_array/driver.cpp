// `[T; N]` over the generated C++20 `NveBinValue` codec: lossless round trip, and any other
// element count fails the reader (`Truncated` when shorter, `TooLarge` when longer).
#include "FixedCodecHybridSpec.generated.hpp"

#include <cstdio>
#include <vector>

using nve::host::BinError;
using nve::host::BinReader;
using nve::host::BinWriter;
using nve::hybrid::FixedBundle;
using nve::hybrid::FixedPoint;

#define REQUIRE(cond)                                                    \
  do {                                                                   \
    if (!(cond)) {                                                       \
      std::fprintf(stderr, "FAILED line %d: %s\n", __LINE__, #cond);     \
      return 1;                                                          \
    }                                                                    \
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

int main() {
  FixedBundle in{};
  in.label = "fixed";
  in.coords = {1u, 0xFFFFFFFFu, 0u, 42u};
  in.points = {FixedPoint{-7, 8}, FixedPoint{2147483647, -2147483647 - 1}};
  in.tag = std::array<uint8_t, 3>{9u, 0u, 255u};

  BinWriter w;
  in.encode_bin(w);
  REQUIRE(w.ok());
  const auto encoded = w.bytes();
  const std::vector<uint8_t> bytes(encoded.begin(), encoded.end());
  REQUIRE(BinReader::validate(bytes) == BinError::None);

  BinReader r(bytes);
  const FixedBundle out = FixedBundle::decode_bin(r);
  REQUIRE(r.ok() && r.at_end());
  REQUIRE(out.label == "fixed");
  REQUIRE(out.coords == in.coords);
  REQUIRE(out.points[0].x == -7 && out.points[0].y == 8);
  REQUIRE(out.points[1].x == 2147483647 && out.points[1].y == -2147483647 - 1);
  REQUIRE(out.tag.has_value() && *out.tag == *in.tag);

  FixedBundle no_tag = in;
  no_tag.tag = std::nullopt;
  BinWriter w2;
  no_tag.encode_bin(w2);
  const auto encoded2 = w2.bytes();
  BinReader r2(encoded2);
  REQUIRE(!FixedBundle::decode_bin(r2).tag.has_value() && r2.ok() && r2.at_end());

  // Exact length decodes; every other count fails the reader.
  REQUIRE(decode_error(bundle_with("coords", 4u, false)) == BinError::None);
  REQUIRE(decode_error(bundle_with("coords", 3u, false)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("coords", 0u, false)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("coords", 5u, false)) == BinError::TooLarge);
  REQUIRE(decode_error(bundle_with("points", 2u, true)) == BinError::None);
  REQUIRE(decode_error(bundle_with("points", 1u, true)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("points", 3u, true)) == BinError::TooLarge);
  REQUIRE(decode_error(bundle_with("tag", 2u, false)) == BinError::Truncated);
  REQUIRE(decode_error(bundle_with("tag", 4u, false)) == BinError::TooLarge);

  std::puts("cpp fixed array ok");
  return 0;
}
