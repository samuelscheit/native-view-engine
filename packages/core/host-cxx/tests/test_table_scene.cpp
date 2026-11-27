// C++20 suite: TableReader / SceneReader over views shaped exactly like the generated
// `NveTableView` / `NveSceneView` (layout locked here with the Rust `offset_of!` values).
#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <optional>
#include <span>
#include <string_view>
#include <vector>

#include "nve/host/scene_reader.hpp"
#include "nve/host/table_reader.hpp"
#include "check.h"

namespace {

// Mirrors Rust `nve_abi::table::NveTableView` / `nve_abi::scene::NveSceneView` (64-bit).
struct TestTableView {
  uint32_t key_lo, key_hi, slot, words, row_count, dirty_count;
  const uint32_t* rows;
  const uint32_t* owners;
  const uint32_t* dirty_rows;
};

struct TestNode {  // stand-in for the generated 64-byte `NveRenderNode`
  uint32_t node_id;
  uint32_t text_offset;
  uint16_t text_len;
  uint16_t child_count;
  uint32_t child_offset;
};

struct TestSceneView {
  uint64_t schema_hash, frame_id;
  uint32_t node_count, dirty_count, removed_count, text_slab_len, children_slab_len, table_count;
  const TestNode* nodes;
  const uint8_t* text_slab;
  const uint32_t* children_slab;
  const uint32_t* dirty_indices;
  const uint32_t* removed_ids;
  const TestTableView* tables;
};

NVE_STATIC_ASSERT(offsetof(TestTableView, dirty_count) == 20, "NveTableView.dirty_count @20");
NVE_STATIC_ASSERT(sizeof(void*) != 8 || sizeof(TestTableView) == 48, "NveTableView is 48 bytes on 64-bit");
NVE_STATIC_ASSERT(offsetof(TestSceneView, frame_id) == 8, "NveSceneView.frame_id @8");
NVE_STATIC_ASSERT(offsetof(TestSceneView, table_count) == 36, "NveSceneView.table_count @36");
NVE_STATIC_ASSERT(offsetof(TestSceneView, nodes) == 40, "NveSceneView.nodes @40");

using nve::host::field;
using nve::host::Key64;
using nve::host::LengthValue;
using nve::host::SceneError;
using nve::host::SceneReader;
using nve::host::TableReader;

constexpr Key64 kVisual = Key64::of("nve.visual");
constexpr Key64 kSchema = Key64{0xF00DFACEC0FFEE11ULL};  // high bit set on purpose

constexpr auto kOpacity = field(0, 0, 8, NVE_FIELD_U32);
constexpr auto kZ = field(0, 8, 12, NVE_FIELD_I32);
constexpr auto kVisible = field(0, 20, 1, NVE_FIELD_BOOL);
constexpr auto kWidth = field(1, 0, 32, NVE_FIELD_LENGTH);
constexpr auto kColor = field(2, 0, 32, NVE_FIELD_COLOR);
constexpr auto kLabel = field(3, 0, 32, NVE_FIELD_STR);
constexpr auto kScale = field(5, 0, 32, NVE_FIELD_F32);

uint32_t f32_bits(float f) { return std::bit_cast<uint32_t>(f); }

std::vector<uint32_t> make_rows() {
  // Row 0: opacity 200, z -3, visible, width 50 %, color, label "hé" @0, scale 1.5
  // Row 1: opacity 0, z 2047, hidden, width auto, label "" @3, scale -0.0
  const uint32_t w0r0 = 200u | ((static_cast<uint32_t>(-3) & 0xFFFu) << 8) | (1u << 20);
  const uint32_t w0r1 = (2047u << 8);
  return {w0r0, f32_bits(50.0f) | 1u, 0x80402010u, 0u, 3u, f32_bits(1.5f),
          w0r1, 2u,                   0u,          3u, 0u, f32_bits(-0.0f)};
}

void test_table_reader_dense() {
  const std::vector<uint32_t> rows = make_rows();
  const std::array<uint32_t, 1> dirty = {1u};
  const TestTableView view{kVisual.lo(), kVisual.hi(), 4u, 6u, 2u, 1u, rows.data(), nullptr, dirty.data()};
  const std::optional<TableReader> t = TableReader::from_view(view);
  CHECK(t.has_value());
  CHECK(t->key() == kVisual && t->slot() == 4u && t->words() == 6u && t->row_count() == 2u && !t->is_sparse());
  CHECK(t->u32(0, kOpacity) == 200u);
  CHECK(t->i32(0, kZ) == -3);
  CHECK(t->i32(1, kZ) == 2047);
  CHECK(t->boolean(0, kVisible) == true);
  CHECK(t->boolean(1, kVisible) == false);
  CHECK(t->length(0, kWidth) == (LengthValue{NVE_LENGTH_PERCENT, 50.0f}));
  CHECK(t->length(1, kWidth)->unit == NVE_LENGTH_AUTO);
  CHECK(t->color(0, kColor)->a() == 0x80 && t->color(0, kColor)->r() == 0x10);
  CHECK(t->str(0, kLabel) == (nve::host::StrSpan{0u, 3u}));
  CHECK(t->f32(0, kScale) == 1.5f);
  CHECK(t->dirty_rows().size() == 1u && t->dirty_rows()[0] == 1u);
  CHECK(t->owner(1) == 1u);
  CHECK(t->row_of(1) == 1u);
  CHECK(!t->row_of(2).has_value());
  // Out of range rows / words never read.
  CHECK(!t->u32(2, kOpacity).has_value());
  CHECK(!t->word(0, 6).has_value());
  CHECK(!t->str(0, field(5, 0, 32, NVE_FIELD_STR)).has_value());  // second word past the row
  CHECK(t->row(9).empty() && t->row(1).size() == 6u);
}

void test_table_reader_sparse_and_invalid() {
  const std::vector<uint32_t> rows = make_rows();
  const std::array<uint32_t, 2> owners = {0x00010007u, 0x00020003u};
  const TestTableView sparse{1u, 2u, 0u, 6u, 2u, 0u, rows.data(), owners.data(), nullptr};
  const std::optional<TableReader> t = TableReader::from_view(sparse);
  CHECK(t.has_value() && t->is_sparse());
  CHECK(t->row_of(0x00020003u) == 1u);
  CHECK(!t->row_of(1u).has_value());
  CHECK(t->owner(0) == 0x00010007u);

  const TestTableView null_rows{1u, 2u, 0u, 6u, 2u, 0u, nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(null_rows).has_value());
  const TestTableView null_dirty{1u, 2u, 0u, 6u, 2u, 3u, rows.data(), nullptr, nullptr};
  CHECK(!TableReader::from_view(null_dirty).has_value());
  const TestTableView too_wide{1u, 2u, 0u, 33u, 0u, 0u, nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(too_wide).has_value());
  const TestTableView zero_words{1u, 2u, 0u, 0u, 0u, 0u, nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(zero_words).has_value());
  CHECK(!TableReader::make(kVisual, 0u, 6u, 3u, rows, {}, {}).has_value());  // 2 rows supplied
}

void test_hostile_specs_and_views() {
  const std::vector<uint32_t> rows = make_rows();
  const auto t = TableReader::make(kVisual, 0u, 6u, 2u, rows, {}, {});
  CHECK(t.has_value());
  // Raw C specs bypass `field()`: every malformed one reads as nullopt, never a UB shift (UBSan).
  const NveFieldSpec bad[] = {
      {0, 0, 0, NVE_FIELD_U32},  {0, 0, 33, NVE_FIELD_U32},  {0, 31, 2, NVE_FIELD_I32}, {0, 255, 255, NVE_FIELD_I32},
      {40, 0, 32, NVE_FIELD_U32}, {6, 0, 32, NVE_FIELD_U32}, {5, 0, 32, NVE_FIELD_STR}, {0, 0, 32, 99},
  };
  for (const NveFieldSpec& f : bad) {
    CHECK(!t->bits(0, f).has_value());
    CHECK(!t->i32(0, f).has_value());
    CHECK(!t->f32(0, f).has_value());
    CHECK(!t->str(0, f).has_value());
  }
  // `str()` only reads `Str` specs that fit the row (a U32 spec at word 3 is not a string).
  CHECK(!t->str(0, NveFieldSpec{3, 0, 32, NVE_FIELD_U32}).has_value());
  CHECK(t->str(0, kLabel).has_value());
  // Hostile word counts are rejected before any span is formed (64-bit overflow / 32-bit hosts).
  const TestTableView huge{1u, 2u, 0u, 0x80000001u, 0xFFFFFFFFu, 0u, rows.data(), nullptr, nullptr};
  CHECK(!TableReader::from_view(huge).has_value());
  const TestTableView max_rows{1u, 2u, 0u, 32u, 0xFFFFFFFFu, 0xFFFFFFFFu, rows.data(), nullptr, rows.data()};
  if constexpr (sizeof(void*) == 4) CHECK(!TableReader::from_view(max_rows).has_value());
}


struct SceneFixture {
  std::vector<TestNode> nodes{{1u, 0u, 3u, 2u, 0u}, {2u, 3u, 4u, 0u, 2u}, {3u, 7u, 0u, 0u, 2u}};
  std::string_view text = "h\xC3\xA9" "\xF0\x9F\x9A\x80";  // "hé" + rocket
  std::vector<uint32_t> children{2u, 3u};
  std::vector<uint32_t> dirty{0u, 2u, 99u};
  std::vector<uint32_t> removed{42u};
  std::vector<uint32_t> rows = make_rows();
  std::vector<TestTableView> tables{{kVisual.lo(), kVisual.hi(), 0u, 6u, 2u, 0u, rows.data(), nullptr, nullptr}};

  TestSceneView view() const {
    return TestSceneView{kSchema.value,
                         7u,
                         static_cast<uint32_t>(nodes.size()),
                         static_cast<uint32_t>(dirty.size()),
                         static_cast<uint32_t>(removed.size()),
                         static_cast<uint32_t>(text.size()),
                         static_cast<uint32_t>(children.size()),
                         static_cast<uint32_t>(tables.size()),
                         nodes.data(),
                         reinterpret_cast<const uint8_t*>(text.data()),
                         children.data(),
                         dirty.data(),
                         removed.data(),
                         tables.data()};
  }
};

void test_scene_reader() {
  const SceneFixture fx;
  const TestSceneView view = fx.view();
  SceneError err = SceneError::NullView;
  const auto scene = SceneReader<TestSceneView>::open(&view, kSchema, &err);
  CHECK(scene.has_value() && err == SceneError::None);
  CHECK(scene->frame_id() == 7u && scene->node_count() == 3u && scene->table_count() == 1u);
  const TestNode* n0 = scene->node(0);
  CHECK(n0 != nullptr && scene->text(n0->text_offset, n0->text_len) == "h\xC3\xA9");
  CHECK(scene->text(3, 4) == "\xF0\x9F\x9A\x80");
  CHECK(!scene->text(2, 1).has_value());  // splits "é"
  CHECK(!scene->text(4, 3).has_value());  // starts mid-rocket
  CHECK(!scene->text(7, 1).has_value());  // out of range
  CHECK(scene->text(7, 0) == "");
  CHECK(!scene->text(0xFFFFFFFFu, 2).has_value());
  CHECK(scene->children(n0->child_offset, n0->child_count)->size() == 2u);
  CHECK(!scene->children(1, 2).has_value());
  CHECK(scene->node(3) == nullptr);
  uint32_t visited = 0;
  scene->for_each_dirty([&](uint32_t index, const TestNode& n) {
    CHECK(n.node_id == index + 1u);
    ++visited;
  });
  CHECK(visited == 2u);  // index 99 skipped
  CHECK(scene->removed_ids().size() == 1u && scene->removed_ids()[0] == 42u);
  CHECK(scene->find_table(kVisual)->u32(0, kOpacity) == 200u);
  CHECK(!scene->find_table(Key64{1}).has_value());
  CHECK(scene->table(0).has_value() && !scene->table(1).has_value());

  // A leading BOM is text, never stripped.
  SceneFixture bom;
  bom.text = "\xEF\xBB\xBF\xEF\xBB\xBFx";
  const TestSceneView bom_view = bom.view();
  const auto bom_scene = SceneReader<TestSceneView>::open(&bom_view, kSchema);
  CHECK(bom_scene.has_value() && bom_scene->text(0, 7) == "\xEF\xBB\xBF\xEF\xBB\xBFx");
  CHECK(bom_scene->text(0, 3) == "\xEF\xBB\xBF");
}

void test_scene_reader_rejects() {
  SceneFixture fx;
  SceneError err = SceneError::None;
  CHECK(!SceneReader<TestSceneView>::open(nullptr, kSchema, &err) && err == SceneError::NullView);

  TestSceneView v = fx.view();
  CHECK(!SceneReader<TestSceneView>::open(&v, Key64{kSchema.value ^ 1u}, &err) && err == SceneError::SchemaMismatch);

  v = fx.view();
  v.children_slab = nullptr;
  CHECK(!SceneReader<TestSceneView>::open(&v, kSchema, &err) && err == SceneError::NullSlab);

  v = fx.view();
  v.node_count = 0;
  v.nodes = nullptr;  // null with zero count is fine
  CHECK(SceneReader<TestSceneView>::open(&v, kSchema, &err).has_value());

  const std::array<uint8_t, 2> bad_utf8 = {0xC0, 0xAF};
  v = fx.view();
  v.text_slab = bad_utf8.data();
  v.text_slab_len = 2;
  CHECK(!SceneReader<TestSceneView>::open(&v, kSchema, &err) && err == SceneError::InvalidUtf8);

  fx.tables[0].words = 40u;
  v = fx.view();
  CHECK(!SceneReader<TestSceneView>::open(&v, kSchema, &err) && err == SceneError::InvalidTable);
}

void test_count_fits_with_32_bit_limits() {
  // The address-space guard is parameterized, so this 64-bit run exercises 32-bit limits directly.
  constexpr uint64_t kPtrdiff32 = 0x7FFFFFFFu;  // PTRDIFF_MAX on a 32-bit host
  static_assert(nve::host::count_fits(0x1FFFFFFFu, 4u, kPtrdiff32));
  static_assert(!nve::host::count_fits(0x20000000u, 4u, kPtrdiff32));
  static_assert(!nve::host::count_fits(0xFFFFFFFFu, 4u, kPtrdiff32));  // u32::MAX dirty rows
  static_assert(!nve::host::count_fits(0xFFFFFFFFu, 64u, kPtrdiff32));  // u32::MAX 64-byte nodes
  static_assert(nve::host::count_fits(0x1FFFFFFu, 64u, kPtrdiff32));
  static_assert(!nve::host::count_fits(1u, 0u, kPtrdiff32));  // zero-size elements are refused
  static_assert(nve::host::count_fits(0u, 64u, 0u));
  static_assert(!nve::host::count_fits(32ull * 0xFFFFFFFFu, 4u, kPtrdiff32));  // max table rows x 32 words
  static_assert(nve::host::count_fits(32ull * 0xFFFFFFFFu, 4u, static_cast<uint64_t>(INT64_MAX)));
  CHECK(nve::host::count_fits_host(0xFFFFFFFFu, 4u) == (sizeof(void*) == 8));
}

void test_scene_slab_counts_fit_the_address_space() {
  // A node slab of u32::MAX 64-byte records exceeds PTRDIFF_MAX on 32-bit hosts and must be
  // rejected before a span is formed; on 64-bit hosts the same count is representable.
  SceneFixture fx;
  TestSceneView v = fx.view();
  v.dirty_count = 0xFFFFFFFFu;
  SceneError err = SceneError::None;
  const bool opened = SceneReader<TestSceneView>::open(&v, kSchema, &err).has_value();
  if constexpr (sizeof(void*) == 4) {
    CHECK(!opened && err == SceneError::NullSlab);
  } else {
    CHECK(opened);  // representable; no read happens at open
  }
}

}  // namespace

int main() {
  test_table_reader_dense();
  test_table_reader_sparse_and_invalid();
  test_hostile_specs_and_views();
  test_scene_slab_counts_fit_the_address_space();
  test_count_fits_with_32_bit_limits();
  test_scene_reader();
  test_scene_reader_rejects();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_table_scene_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_table_scene_test: ok\n");
  return 0;
}
