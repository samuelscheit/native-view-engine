// C++20 suite: BinReader / BinWriter primitives, NveBinValue golden bytes, bounds, fuzz, and
// zero-allocation reads (global operator new is counted).
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <new>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "nve/host/bin_codec.hpp"
#include "nve/host/hybrid_runtime.hpp"
#include "check.h"

static size_t g_allocations = 0;

void* operator new(size_t n) {
  ++g_allocations;
  if (void* p = std::malloc(n == 0 ? 1 : n)) return p;
  throw std::bad_alloc();
}
void operator delete(void* p) noexcept { std::free(p); }
void operator delete(void* p, size_t) noexcept { std::free(p); }

using nve::host::BinError;
using nve::host::BinReader;
using nve::host::BinTag;
using nve::host::BinWriter;
using nve::host::DynamicValue;

namespace {

// Validated by Rust `BinValueView::parse`: {"k": [1, "é", null, true, 1.5, b"\xFF"]}.
constexpr std::string_view kGoldenHex =
    "07 34000000 01000000 01000000 6b 06 26000000 06000000 02 0100000000000000 04 02000000 c3a9 00 0101 03 "
    "000000000000f83f 05 01000000 ff";

std::vector<uint8_t> from_hex(std::string_view hex) {
  std::vector<uint8_t> out;
  int hi = -1;
  for (const char c : hex) {
    if (c == ' ') continue;
    const int nib = c <= '9' ? c - '0' : c - 'a' + 10;
    if (hi < 0) {
      hi = nib;
    } else {
      out.push_back(static_cast<uint8_t>(hi * 16 + nib));
      hi = -1;
    }
  }
  return out;
}

std::vector<uint8_t> encode_golden() {
  BinWriter w;
  const BinWriter::Mark map = w.begin_map(1);
  w.write_map_key("k");
  const BinWriter::Mark arr = w.begin_array(6);
  w.write_i64(1);
  w.write_str("\xC3\xA9");
  w.write_null();
  w.write_bool(true);
  w.write_f64(1.5);
  const uint8_t ff = 0xFF;
  w.write_bytes(std::span<const uint8_t>(&ff, 1));
  w.end(arr);
  w.end(map);
  CHECK(w.ok());
  return std::move(w).take();
}

void test_golden_round_trip() {
  const std::vector<uint8_t> golden = from_hex(kGoldenHex);
  CHECK(encode_golden() == golden);
  CHECK(BinReader::validate(golden) == BinError::None);

  const size_t before = g_allocations;
  BinReader r(golden);
  CHECK(r.begin_map() == 1u);
  CHECK(r.read_map_key() == "k");
  CHECK(r.begin_array() == 6u);
  CHECK(r.read_i64() == 1);
  CHECK(r.read_str() == "\xC3\xA9");
  CHECK(r.read_null());
  CHECK(!r.read_null());  // `true` stays in place
  CHECK(r.read_bool());
  CHECK(r.read_f64() == 1.5);
  const std::span<const uint8_t> b = r.read_bytes();
  CHECK(b.size() == 1u && b[0] == 0xFF);
  CHECK(r.ok() && r.at_end());
  CHECK(g_allocations == before);  // reads borrow, never allocate
}

void test_primitives_unaligned() {
  BinWriter w;
  w.u8(0xAA);  // shift everything off alignment
  w.u16(0xBEEF);
  w.u32(0xDEADBEEFu);
  w.u64(0x0123456789ABCDEFULL);
  w.i32(-2);
  w.i64(std::numeric_limits<int64_t>::min());
  w.f32(-0.0f);
  w.f64(std::numeric_limits<double>::infinity());
  BinReader r(w.bytes());
  CHECK(r.u8() == 0xAA);
  CHECK(r.u16() == 0xBEEF);
  CHECK(r.u32() == 0xDEADBEEFu);
  CHECK(r.u64() == 0x0123456789ABCDEFULL);
  CHECK(r.i32() == -2);
  CHECK(r.i64() == std::numeric_limits<int64_t>::min());
  const float nz = r.f32();
  CHECK(nz == 0.0f && std::signbit(nz));
  CHECK(std::isinf(r.f64()));
  CHECK(r.ok() && r.at_end());
  CHECK(r.u8() == 0 && r.error() == BinError::Truncated);  // sticky afterwards
  CHECK(r.u32() == 0 && r.error() == BinError::Truncated);
}

void test_f64_accepts_i64_and_extremes() {
  BinWriter w;
  w.write_i64(-7);
  w.write_i64(std::numeric_limits<int64_t>::max());
  w.write_f64(std::nan(""));
  BinReader r(w.bytes());
  CHECK(r.read_f64() == -7.0);
  CHECK(r.read_i64() == std::numeric_limits<int64_t>::max());
  CHECK(std::isnan(r.read_f64()));
  CHECK(r.ok());
}

void test_errors() {
  {
    const std::vector<uint8_t> bad_bool = {1, 2};
    BinReader r(bad_bool);
    CHECK(!r.read_bool() && r.error() == BinError::InvalidBool);
    CHECK(BinReader::validate(bad_bool) == BinError::InvalidBool);
  }
  {
    const std::vector<uint8_t> bad_tag = {8};
    BinReader r(bad_tag);
    (void)r.peek_tag();
    CHECK(r.error() == BinError::InvalidTag);
  }
  {
    const std::vector<uint8_t> wrong = {0};
    BinReader r(wrong);
    CHECK(r.read_i64() == 0 && r.error() == BinError::UnexpectedTag);
  }
  {
    const std::vector<uint8_t> bad_utf8 = {4, 2, 0, 0, 0, 0xC0, 0xAF};
    BinReader r(bad_utf8);
    CHECK(r.read_str().empty() && r.error() == BinError::InvalidUtf8);
  }
  {
    const std::vector<uint8_t> huge_len = {4, 0xFF, 0xFF, 0xFF, 0xFF, 'a'};
    CHECK(BinReader::validate(huge_len) == BinError::Truncated);
  }
  {
    // Count bomb: claims 2^32-1 elements in a 4-byte payload.
    const std::vector<uint8_t> bomb = {6, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF};
    BinReader r(bomb);
    CHECK(r.begin_array() == 0u && r.error() == BinError::Truncated);
  }
  {
    // Map entries need >= 5 bytes each: 1 entry in a 4-byte remainder is rejected.
    const std::vector<uint8_t> map_bomb = {7, 8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0};
    CHECK(BinReader::validate(map_bomb) == BinError::Truncated);
  }
  {
    // Payload declares one spare byte after its single element.
    const std::vector<uint8_t> trailing = {6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0};
    CHECK(BinReader::validate(trailing) == BinError::TrailingBytes);
  }
  {
    const std::vector<uint8_t> two_values = {0, 0};
    CHECK(BinReader::validate(two_values) == BinError::TrailingBytes);
  }
  {
    // Element overruns its container into the next bytes.
    const std::vector<uint8_t> overrun = {6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0};
    CHECK(BinReader::validate(overrun) != BinError::None);
  }
}

std::vector<uint8_t> nested_arrays(uint32_t depth) {
  BinWriter w;
  std::vector<BinWriter::Mark> marks;
  for (uint32_t i = 0; i < depth; ++i) marks.push_back(w.begin_array(1));
  w.write_null();
  for (auto it = marks.rbegin(); it != marks.rend(); ++it) w.end(*it);
  return std::move(w).take();
}

void test_depth_limit() {
  CHECK(BinReader::validate(nested_arrays(nve::host::kBinValueMaxDepth)) == BinError::None);
  CHECK(BinReader::validate(nested_arrays(nve::host::kBinValueMaxDepth + 1)) == BinError::MaxDepth);
}

void test_skip_and_raw() {
  const std::vector<uint8_t> golden = from_hex(kGoldenHex);
  std::vector<uint8_t> two = golden;
  two.push_back(0);  // a trailing Null after the map
  BinReader r(two);
  const std::span<const uint8_t> raw = r.read_raw_value();
  CHECK(raw.size() == golden.size());
  CHECK(r.read_null() && r.at_end() && r.ok());
}

void test_truncation_every_prefix() {
  const std::vector<uint8_t> golden = from_hex(kGoldenHex);
  for (size_t n = 0; n < golden.size(); ++n) {
    const std::span<const uint8_t> prefix(golden.data(), n);
    CHECK(BinReader::validate(prefix) != BinError::None);
    BinReader r(prefix);
    r.skip_value();
    CHECK(!r.ok());
  }
}

void test_fuzz_never_overreads() {
  // Deterministic xorshift corpus: mutate the golden bytes and random buffers. Run under
  // ASan/UBSan (the CMake suite enables them) to prove no read leaves the span.
  uint64_t s = 0x9E3779B97F4A7C15ULL;
  auto next = [&s]() {
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    return s;
  };
  const std::vector<uint8_t> golden = from_hex(kGoldenHex);
  for (int iter = 0; iter < 20000; ++iter) {
    std::vector<uint8_t> buf = golden;
    if (iter % 2 == 0) {
      buf[next() % buf.size()] = static_cast<uint8_t>(next());
      buf[next() % buf.size()] = static_cast<uint8_t>(next());
    } else {
      buf.resize(next() % 48);
      for (uint8_t& b : buf) b = static_cast<uint8_t>(next() % 9);
    }
    const BinError e = BinReader::validate(buf);
    BinReader r(buf);
    (void)r.read_raw_value();
    if (e == BinError::None) CHECK(r.ok() && r.at_end());
  }
}

void test_dynamic_value() {
  const DynamicValue null_value;
  CHECK(null_value.is_null() && null_value.bytes().size() == 1u);
  const std::vector<uint8_t> golden = from_hex(kGoldenHex);
  const std::optional<DynamicValue> v = DynamicValue::from_bytes(golden);
  CHECK(v.has_value() && v->tag() == BinTag::Map);
  CHECK(!DynamicValue::from_bytes(std::span<const uint8_t>(golden.data(), 10)).has_value());
  CHECK(!DynamicValue::from_bytes({}).has_value());
  CHECK(nve::host::check_schema(nve::host::Key64{1}, nve::host::Key64{1}) == nve::host::HybridStatus::Ok);
  CHECK(nve::host::check_schema(nve::host::Key64{1}, nve::host::Key64{2}) == nve::host::HybridStatus::SchemaMismatch);
  CHECK(static_cast<int32_t>(nve::host::HostStatus::InternalError) == -8);
}

void test_writer_reuse_is_allocation_free() {
  BinWriter w(256);
  w.write_str("warm-up");
  w.clear();
  const size_t before = g_allocations;
  for (int i = 0; i < 100; ++i) {
    w.clear();
    const BinWriter::Mark m = w.begin_array(2);
    w.write_i64(i);
    w.write_str("steady");
    w.end(m);
  }
  CHECK(g_allocations == before);
  CHECK(BinReader::validate(w.bytes()) == BinError::None);
}

}  // namespace

int main() {
  test_golden_round_trip();
  test_primitives_unaligned();
  test_f64_accepts_i64_and_extremes();
  test_errors();
  test_depth_limit();
  test_skip_and_raw();
  test_truncation_every_prefix();
  test_fuzz_never_overreads();
  test_dynamic_value();
  test_writer_reuse_is_allocation_free();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_bin_codec_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_bin_codec_test: ok\n");
  return 0;
}
