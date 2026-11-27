// C++20 suite: BinReader / BinWriter primitives, NveBinValue golden bytes, bounds, fuzz, and
// zero-allocation reads (global operator new is counted).
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <new>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "nve/host/bin_codec.hpp"
#include "nve/host/hybrid_runtime.hpp"
#include "nve/host/utf16.hpp"
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
  const BinReader::ReadMark map = r.begin_map();
  CHECK(map.count == 1u);
  CHECK(r.read_map_key() == "k");
  const BinReader::ReadMark arr = r.begin_array();
  CHECK(arr.count == 6u);
  CHECK(r.read_i64() == 1);
  CHECK(r.read_str() == "\xC3\xA9");
  CHECK(r.read_null());
  CHECK(!r.read_null());  // `true` stays in place
  CHECK(r.read_bool());
  CHECK(r.read_f64() == 1.5);
  const std::span<const uint8_t> b = r.read_bytes();
  CHECK(b.size() == 1u && b[0] == 0xFF);
  r.end(arr);
  r.end(map);
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
    CHECK(r.begin_array().count == 0u && r.error() == BinError::Truncated);
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

void test_container_limit_is_enforced() {
  // Rust validates children against `&bytes[..container_end]`: the string's length field lies past
  // the 5-byte array payload, so this is `Truncated` — never a read of the bytes after the container.
  const std::vector<uint8_t> escape = {6, 5, 0, 0, 0, 1, 0, 0, 0, 4, 2, 0, 0, 0, 0xC0, 0xAF};
  CHECK(BinReader::validate(escape) == BinError::Truncated);
  // Nested: inner array claims more payload than the outer one has left.
  const std::vector<uint8_t> nested = {6, 10, 0, 0, 0, 1, 0, 0, 0, 6, 9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0};
  CHECK(BinReader::validate(nested) == BinError::Truncated);
}

void test_streaming_reads_stay_inside_their_container() {
  // `[[1], 7]` where the inner array claims 2 elements in a payload holding 1: the second read
  // must fail at the inner container's end, not consume the outer `7`.
  BinWriter w;
  const BinWriter::Mark outer = w.begin_array(2);
  const BinWriter::Mark inner = w.begin_array(1);
  w.write_i64(1);
  w.end(inner);
  w.write_i64(7);
  w.end(outer);
  std::vector<uint8_t> bytes(w.bytes().begin(), w.bytes().end());
  bytes[14] = 2;  // inner count: 1 -> 2 (payload stays 13 bytes, room for 1 bool-sized entry)
  {
    BinReader r(bytes);
    const BinReader::ReadMark o = r.begin_array();
    const BinReader::ReadMark i = r.begin_array();
    CHECK(o.count == 2u && i.count == 2u && r.ok());
    CHECK(r.read_i64() == 1);
    CHECK(r.read_i64() == 0 && r.error() == BinError::Truncated);  // never the outer `7`
  }
  {
    // A container whose entries were not all read is `TrailingBytes` at `end`.
    BinReader r(w.bytes());
    const BinReader::ReadMark o = r.begin_array();
    (void)r.read_raw_value();
    r.end(o);
    CHECK(r.error() == BinError::TrailingBytes);
  }
  {
    // Exact reads close cleanly and restore the outer bounds.
    BinReader r(w.bytes());
    const BinReader::ReadMark o = r.begin_array();
    const BinReader::ReadMark i = r.begin_array();
    CHECK(r.read_i64() == 1);
    r.end(i);
    CHECK(r.read_i64() == 7);
    r.end(o);
    CHECK(r.ok() && r.at_end());
  }
}

void test_checked_values_and_error_names() {
  const std::vector<uint8_t> golden = from_hex(kGoldenHex);
  BinReader r(golden);
  const DynamicValue v = DynamicValue::read(r);
  CHECK(r.ok() && r.at_end() && !v.is_null() && v.bytes().size() == golden.size());
  // A map whose string is invalid UTF-8: `read_raw_value` skips it, `DynamicValue::read` fails
  // the reader (sticky) and yields null.
  const std::vector<uint8_t> bad = {7, 15, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 'k', 4, 1, 0, 0, 0, 0xFF};
  BinReader skip(bad);
  CHECK(skip.read_raw_value().size() == bad.size() && skip.ok());
  BinReader checked(bad);
  const DynamicValue rejected = DynamicValue::read(checked);
  CHECK(rejected.is_null() && checked.error() == BinError::InvalidUtf8);
  CHECK(std::string_view(nve::host::to_string(BinError::TrailingBytes)) == "TrailingBytes");
  CHECK(std::string_view(nve::host::to_string(BinError::None)) == "None");
}

void test_utf16_round_trips_and_rejects_lone_surrogates() {
  const std::u16string all = u"a\u00E9\u20AC\U0001F600";
  const std::optional<std::string> utf8 = nve::host::utf16_to_utf8(all);
  CHECK(utf8.has_value() && *utf8 == "a\xC3\xA9\xE2\x82\xAC\xF0\x9F\x98\x80");
  CHECK(nve::host::utf8_to_utf16(*utf8) == all);
  // Embedded NUL survives (Modified UTF-8 would turn it into `C0 80`).
  const std::u16string nul(u"a\0b", 3);
  CHECK(nve::host::utf16_to_utf8(nul) == std::string("a\0b", 3));
  CHECK(!nve::host::utf16_to_utf8(std::u16string(1, char16_t{0xD800})).has_value());
  CHECK(!nve::host::utf16_to_utf8(std::u16string(1, char16_t{0xDC00})).has_value());
  CHECK(!nve::host::utf16_to_utf8(std::u16string{char16_t{0xD800}, u'x'}).has_value());
  CHECK(!nve::host::utf8_to_utf16("\xED\xA0\x80").has_value());
  CHECK(!nve::host::utf8_to_utf16("\xFF").has_value());
  CHECK(nve::host::utf8_to_utf16("").has_value());
}

void test_bom_and_invalid_utf8_writes() {
  for (const std::string_view s : {std::string_view("\xEF\xBB\xBF"), std::string_view("\xEF\xBB\xBFx"),
                                   std::string_view("\xEF\xBB\xBF\xEF\xBB\xBFx")}) {
    BinWriter w;
    const BinWriter::Mark m = w.begin_map(1);
    w.write_map_key(s);
    w.write_str(s);
    w.end(m);
    BinReader r(w.bytes());
    const BinReader::ReadMark mark = r.begin_map();
    CHECK(mark.count == 1u && r.read_map_key() == s && r.read_str() == s && r.ok());
    r.end(mark);
    CHECK(r.ok() && r.at_end());
  }
  // A lone surrogate (CESU/WTF-8 `ED A0 80`) or overlong is rejected by every writer path.
  for (const std::string_view bad : {std::string_view("\xED\xA0\x80"), std::string_view("\xC0\xAF")}) {
    BinWriter w;
    w.write_str(bad);
    CHECK(w.error() == BinError::InvalidUtf8);
    BinWriter k;
    k.write_map_key(bad);
    CHECK(k.error() == BinError::InvalidUtf8);
  }
}

void test_stale_marks_never_write_out_of_bounds() {
  BinWriter w;
  const BinWriter::Mark m = w.begin_array(1);
  w.write_null();
  w.clear();
  w.end(m);  // buffer is empty: must not touch memory (ASan)
  CHECK(w.error() == BinError::InvalidMark);
  BinWriter d;
  d.write_null();
  d.end(BinWriter::Mark{});
  CHECK(d.error() == BinError::InvalidMark);
  BinWriter far;
  const BinWriter::Mark open = far.begin_array(0);
  far.end(BinWriter::Mark{open.len_pos + 1u, open.generation});  // not right after a container tag
  CHECK(far.error() == BinError::InvalidMark);
  far.clear();
  CHECK(far.ok());

  // Stale after `clear()` + refill: the old position is in range again, but the generation differs.
  BinWriter refill;
  const BinWriter::Mark stale = refill.begin_array(1);
  refill.clear();
  const std::vector<uint8_t> sixteen(16, 0xAB);
  refill.write_bytes(sixteen);
  const std::vector<uint8_t> before(refill.bytes().begin(), refill.bytes().end());
  refill.end(stale);
  CHECK(refill.error() == BinError::InvalidMark);
  CHECK(std::vector<uint8_t>(refill.bytes().begin(), refill.bytes().end()) == before);  // untouched

  // Refill recreates a container at the very same offset: only the generation can tell them apart.
  BinWriter same;
  const BinWriter::Mark old = same.begin_array(1);
  same.clear();
  const BinWriter::Mark fresh = same.begin_array(2);
  same.write_null();
  same.write_null();
  same.end(old);
  CHECK(same.error() == BinError::InvalidMark);
  BinWriter same_ok;
  (void)same_ok.begin_array(1);
  same_ok.clear();
  const BinWriter::Mark live = same_ok.begin_array(1);
  same_ok.write_null();
  same_ok.end(live);
  CHECK(same_ok.ok() && BinReader::validate(same_ok.bytes()) == BinError::None);
  (void)fresh;
}

void test_marks_never_patch_foreign_bytes() {
  // `end` at offset 0 of a 9-byte buffer would overwrite the I64 tag + 3 payload bytes.
  BinWriter w;
  w.write_i64(0x0102030405060708);
  const std::vector<uint8_t> before(w.bytes().begin(), w.bytes().end());
  w.end(BinWriter::Mark{0u, 1u});
  w.end(BinWriter::Mark{1u, 1u});
  CHECK(w.error() == BinError::InvalidMark);
  CHECK(std::vector<uint8_t>(w.bytes().begin(), w.bytes().end()) == before);
  // Stale after `clear()` + a 9-byte refill.
  BinWriter s;
  const BinWriter::Mark m = s.begin_array(1);
  s.clear();
  s.write_i64(-1);
  const std::vector<uint8_t> refill(s.bytes().begin(), s.bytes().end());
  s.end(m);
  CHECK(s.error() == BinError::InvalidMark);
  CHECK(std::vector<uint8_t>(s.bytes().begin(), s.bytes().end()) == refill);
  // Double `end`: the second would re-patch a closed container with the grown length.
  BinWriter d;
  const BinWriter::Mark open = d.begin_array(1);
  d.write_null();
  d.end(open);
  CHECK(d.ok());
  d.write_null();
  const std::vector<uint8_t> closed(d.bytes().begin(), d.bytes().end());
  d.end(open);
  CHECK(d.error() == BinError::InvalidMark);
  CHECK(std::vector<uint8_t>(d.bytes().begin(), d.bytes().end()) == closed);
}

void test_rejected_writes_leave_no_bytes() {
  for (const std::string_view bad : {std::string_view("\xED\xA0\x80"), std::string_view("ok\xC0\xAF")}) {
    BinWriter w;
    w.write_null();
    w.write_str(bad);
    CHECK(w.error() == BinError::InvalidUtf8 && w.size() == 1u);  // no orphaned tag or length
    BinWriter k;
    k.write_null();
    k.write_map_key(bad);
    CHECK(k.error() == BinError::InvalidUtf8 && k.size() == 1u);
  }
}

void test_validation_is_allocation_free() {
  BinWriter w(1 << 20);
  const BinWriter::Mark m = w.begin_array(3);
  const std::vector<uint8_t> blob(256 * 1024, 0xAB);
  w.write_bytes(blob);
  w.write_str(std::string(128 * 1024, 'x'));
  w.write_str("h\xC3\xA9");
  w.end(m);
  const size_t before = g_allocations;
  CHECK(BinReader::validate(w.bytes()) == BinError::None);
  CHECK(g_allocations == before);
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
  test_container_limit_is_enforced();
  test_bom_and_invalid_utf8_writes();
  test_stale_marks_never_write_out_of_bounds();
  test_validation_is_allocation_free();
  test_marks_never_patch_foreign_bytes();
  test_rejected_writes_leave_no_bytes();
  test_streaming_reads_stay_inside_their_container();
  test_checked_values_and_error_names();
  test_utf16_round_trips_and_rejects_lone_surrogates();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_bin_codec_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_bin_codec_test: ok\n");
  return 0;
}
