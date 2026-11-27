// C++20 suite: constexpr XXH64 / Key64 (compile-time and runtime parity with Rust `nve-hash`).
#include <array>
#include <cstdint>
#include <span>
#include <string_view>
#include <vector>

#include "nve/host/xxh64.hpp"
#include "vectors.h"

using nve::host::const_id;
using nve::host::Key64;
using nve::host::Xxh64Hasher;
using nve::host::xxh64;

namespace {

template <size_t N>
constexpr std::array<uint8_t, N> seq() {
  std::array<uint8_t, N> out{};
  for (size_t i = 0; i < N; ++i) out[i] = static_cast<uint8_t>((i * 31u + 7u) & 0xFFu);
  return out;
}

template <size_t N>
constexpr uint64_t streamed(const std::array<uint8_t, N>& bytes, uint64_t seed, size_t step) {
  Xxh64Hasher h(seed);
  for (size_t at = 0; at < N; at += step) h.update(std::span<const uint8_t>(bytes).subspan(at, N - at < step ? N - at : step));
  return h.digest();
}

constexpr auto kSeq101 = seq<101>();
constexpr auto kSeq37 = seq<37>();

// Compile-time evaluation is part of the contract: table keys and schema hashes bake into binaries.
static_assert(xxh64(std::string_view{}) == 0xEF46DB3751D8E999ULL);
static_assert(const_id("abc") == 0x44BC2CF5AD770999ULL);
static_assert(Key64::of("nve.visual").value == 0x33E73115985A4FD6ULL);
static_assert(Key64::of("nve.visual").lo() == 0x985A4FD6u && Key64::of("nve.visual").hi() == 0x33E73115u);
static_assert(Key64::from_halves(0x985A4FD6u, 0x33E73115u) == Key64::of("nve.visual"));
static_assert(xxh64(std::string_view("h\xC3\xA9llo w\xC3\xB6rld \xE2\x80\x94 \xE6\x97\xA5\xE6\x9C\xAC\xE8\xAA\x9E \xF0\x9F\x9A\x80")) ==
              0x72AC442FF213F592ULL);
static_assert(xxh64(kSeq101) == 0xFBFDF3FA1A53BC7FULL);
static_assert(xxh64(kSeq101, WIRE_SEED) == 0x277E092A8C3DB1BCULL);
static_assert(xxh64(kSeq37, NVE_XXH64_PRIME1) == 0x33DDB9A885FC58A4ULL);
static_assert(streamed(kSeq101, WIRE_SEED, 1) == 0x277E092A8C3DB1BCULL);
static_assert(streamed(kSeq101, WIRE_SEED, 33) == 0x277E092A8C3DB1BCULL);
static_assert(Xxh64Hasher(7).update_u32(0x04030201u).update_u64(0x0c0b0a0908070605ULL).digest() ==
              xxh64(std::array<uint8_t, 12>{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12}, 7));

void test_runtime_vectors() {
  std::vector<uint8_t> buf(MAX_VECTOR_LEN);
  for (const HashVector& v : kVectors) {
    vector_bytes(&v, buf.data());
    const std::span<const uint8_t> bytes(buf.data(), v.len);
    CHECK(xxh64(bytes, v.seed) == v.expected);
    for (size_t step : {size_t{1}, size_t{7}, size_t{32}, size_t{33}, size_t{100}}) {
      Xxh64Hasher h(v.seed);
      for (size_t at = 0; at < v.len; at += step) h.update(bytes.subspan(at, v.len - at < step ? v.len - at : step));
      CHECK(h.digest() == v.expected);
    }
    if (v.input == INPUT_TEXT) {
      CHECK(xxh64(std::string_view(v.text, v.len), v.seed) == v.expected);
    }
  }
}

void test_runtime_string_view_matches_constexpr() {
  // Same text hashed at compile time (chunked copy) and at runtime (direct reinterpret).
  constexpr std::string_view text = "0123456789abcdef0123456789abcdef0123456789";
  constexpr uint64_t baked = xxh64(text, NVE_XXH64_PRIME1);
  volatile size_t len = text.size();  // defeat constant folding
  CHECK(xxh64(std::string_view(text.data(), len), NVE_XXH64_PRIME1) == baked);
  CHECK(nve_key64_of(text.data(), text.size()).value == const_id(text));
  CHECK(Key64::from_c(Key64::of("x").to_c()) == Key64::of("x"));
}

}  // namespace

int main() {
  test_runtime_vectors();
  test_runtime_string_view_matches_constexpr();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_xxh64_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_xxh64_test: ok\n");
  return 0;
}
