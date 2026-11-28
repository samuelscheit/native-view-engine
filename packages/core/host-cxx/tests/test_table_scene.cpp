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

#include "nve/host/runtime_session.hpp"
#include "nve/host/scene_reader.hpp"
#include "nve/host/table_reader.hpp"
#include "check.h"

namespace {

// Mirrors Rust `nve_abi::table::NveTableView`; its 64-bit layout is locked below.
struct TestTableView {
  uint32_t key_lo, key_hi, slot, words, row_count, dirty_count, row_index_dir_len, row_index_page_count;
  const uint32_t* rows;
  const uint32_t* owners;
  const uint32_t* dirty_rows;
  const uint32_t* row_index_dir;
  const uint32_t* row_index_pages;
};

NVE_STATIC_ASSERT(offsetof(TestTableView, dirty_count) == 20, "NveTableView.dirty_count @20");
NVE_STATIC_ASSERT(offsetof(TestTableView, row_index_page_count) == 28, "NveTableView.row_index_page_count @28");
NVE_STATIC_ASSERT(sizeof(void*) != 8 || sizeof(TestTableView) == 72, "NveTableView is 72 bytes on 64-bit");
NVE_STATIC_ASSERT(sizeof(TestTableView) == sizeof(NveTableView), "mirror matches the generated NveTableView");

using nve::host::field;
using nve::host::Key64;
using nve::host::LengthValue;
using nve::host::SceneError;
using nve::host::SceneReader;
using nve::host::TableReader;

constexpr Key64 kVisual = Key64::of("nve.visual");
constexpr Key64 kSchema = Key64{NVE_RUNTIME_SCHEMA_HASH};

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
  const TestTableView view{kVisual.lo(), kVisual.hi(), 4u, 6u, 2u, 1u, 0u, 0u, rows.data(), nullptr, dirty.data(), nullptr, nullptr};
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
  const std::array<uint32_t, 2> owners = {0x01000007u, 0x02000003u};
  // Paged row index (`row + 1`): node 7 -> row 0, node 3 -> row 1 in page 0; node 260 sits in
  // page 1 (directory slot 1) with a stale `row + 1` past `row_count`; directory slot 2 is
  // `NO_PAGE`, slot 3 names a page that does not exist.
  std::vector<uint32_t> pages(2u * NVE_ROW_INDEX_PAGE_LEN, 0u);
  pages[7] = 1u;
  pages[3] = 2u;
  pages[NVE_ROW_INDEX_PAGE_LEN + 4u] = 3u;
  const std::array<uint32_t, 4> dir = {0u, 1u, NVE_ROW_INDEX_NO_PAGE, 9u};
  const TestTableView sparse{1u, 2u, 0u, 6u, 2u, 0u, 4u, 2u, rows.data(), owners.data(), nullptr, dir.data(), pages.data()};
  const std::optional<TableReader> t = TableReader::from_view(sparse);
  CHECK(t.has_value() && t->is_sparse());
  CHECK(t->row_of_node(7u) == 0u);
  CHECK(t->row_of_node(3u) == 1u);
  CHECK(!t->row_of_node(4u).has_value());                            // no row
  CHECK(!t->row_of_node(NVE_ROW_INDEX_PAGE_LEN + 4u).has_value());   // row + 1 past row_count
  CHECK(!t->row_of_node(2u * NVE_ROW_INDEX_PAGE_LEN).has_value());   // NO_PAGE
  CHECK(!t->row_of_node(3u * NVE_ROW_INDEX_PAGE_LEN).has_value());   // page out of range
  CHECK(!t->row_of_node(4u * NVE_ROW_INDEX_PAGE_LEN).has_value());   // past the directory
  CHECK(!t->row_of_node(0xFFFFFFFFu).has_value());
  CHECK(t->row_of(0x02000003u) == 1u);
  CHECK(!t->row_of(0x03000003u).has_value());  // same slot, other generation
  CHECK(!t->row_of(1u).has_value());
  CHECK(t->owner(0) == 0x01000007u);
  // Null index spans with non-zero counts are rejected before a span is formed.
  const TestTableView null_dir{1u, 2u, 0u, 6u, 2u, 0u, 4u, 2u, rows.data(), owners.data(), nullptr, nullptr, pages.data()};
  CHECK(!TableReader::from_view(null_dir).has_value());
  const TestTableView null_pages{1u, 2u, 0u, 6u, 2u, 0u, 4u, 2u, rows.data(), owners.data(), nullptr, dir.data(), nullptr};
  CHECK(!TableReader::from_view(null_pages).has_value());
  // A sparse table with rows but no row index (or less than one page) is rejected, never read as
  // "no rows"; one without rows needs none.
  const TestTableView no_index{1u, 2u, 0u, 6u, 2u, 0u, 0u, 0u, rows.data(), owners.data(), nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(no_index).has_value());
  CHECK(!TableReader::make(kVisual, 0u, 6u, 2u, rows, owners, {}, dir, std::span(pages).first(10u)).has_value());
  const TestTableView no_dir{1u, 2u, 0u, 6u, 2u, 0u, 0u, 2u, rows.data(), owners.data(), nullptr, nullptr, pages.data()};
  CHECK(!TableReader::from_view(no_dir).has_value());
  const TestTableView empty_sparse{1u, 2u, 0u, 6u, 0u, 0u, 0u, 0u, rows.data(), owners.data(), nullptr, nullptr, nullptr};
  CHECK(TableReader::from_view(empty_sparse).has_value());

  const TestTableView null_rows{1u, 2u, 0u, 6u, 2u, 0u, 0u, 0u, nullptr, nullptr, nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(null_rows).has_value());
  const TestTableView null_dirty{1u, 2u, 0u, 6u, 2u, 3u, 0u, 0u, rows.data(), nullptr, nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(null_dirty).has_value());
  const TestTableView too_wide{1u, 2u, 0u, 33u, 0u, 0u, 0u, 0u, nullptr, nullptr, nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(too_wide).has_value());
  const TestTableView zero_words{1u, 2u, 0u, 0u, 0u, 0u, 0u, 0u, nullptr, nullptr, nullptr, nullptr, nullptr};
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
  const TestTableView huge{1u, 2u, 0u, 0x80000001u, 0xFFFFFFFFu, 0u, 0u, 0u, rows.data(), nullptr, nullptr, nullptr, nullptr};
  CHECK(!TableReader::from_view(huge).has_value());
  const TestTableView max_rows{1u, 2u, 0u, 32u, 0xFFFFFFFFu, 0xFFFFFFFFu, 0u, 0u, rows.data(), nullptr, rows.data(), nullptr, nullptr};
  if constexpr (sizeof(void*) == 4) CHECK(!TableReader::from_view(max_rows).has_value());
}


struct SceneFixture {
  std::vector<NveNodeRow> node_rows = make_node_rows();
  std::vector<NveLayoutRect> rects{{0.f, 0.f, 1.f, 1.f}, {1.f, 1.f, 2.f, 2.f}, {2.f, 2.f, 3.f, 3.f}};
  std::string_view text = "h\xC3\xA9" "\xF0\x9F\x9A\x80";  // "hé" + rocket
  std::vector<uint32_t> children{2u, 3u};
  std::vector<uint32_t> dirty{0u, 2u};
  std::vector<uint32_t> removed{42u};
  std::vector<uint32_t> rows = make_rows();
  std::vector<NveTableView> tables{{kVisual.lo(), kVisual.hi(), 0u, 6u, 2u, 0u, 0u, 0u, rows.data(), nullptr, nullptr, nullptr, nullptr}};

  static std::vector<NveNodeRow> make_node_rows() {
    std::vector<NveNodeRow> out(3);
    for (NveNodeRow& r : out) r = NveNodeRow{};
    for (NveNodeRow& r : out) r.flags = static_cast<uint8_t>(NVE_NODE_ROW_FLAG_LIVE);
    out[0].child_off = 0u;
    out[0].child_len = 2u;
    out[1].child_off = 2u;
    out[2].child_off = 2u;
    return out;
  }

  NveSceneView view() const {
    NveSceneView v{};
    v.schema_hash = kSchema.value;
    v.frame_id = 7u;
    v.node_capacity = static_cast<uint32_t>(node_rows.size());
    v.rows = node_rows.data();
    v.rects = rects.data();
    v.text_slab = reinterpret_cast<const uint8_t*>(text.data());
    v.text_slab_len = static_cast<uint32_t>(text.size());
    v.children_slab = children.data();
    v.children_slab_len = static_cast<uint32_t>(children.size());
    v.dirty_indices = dirty.data();
    v.dirty_count = static_cast<uint32_t>(dirty.size());
    v.removed_ids = removed.data();
    v.removed_count = static_cast<uint32_t>(removed.size());
    v.tables = tables.data();
    v.table_count = static_cast<uint32_t>(tables.size());
    return v;
  }
};

void test_scene_reader() {
  const SceneFixture fx;
  const NveSceneView view = fx.view();
  SceneError err = SceneError::NullView;
  const auto scene = SceneReader::open(&view, kSchema, &err);
  CHECK(scene.has_value() && err == SceneError::None);
  CHECK(scene->frame_id() == 7u && scene->node_capacity() == 3u && scene->table_count() == 1u);
  CHECK(scene->text(0, 3) == "h\xC3\xA9");
  CHECK(scene->text(3, 4) == "\xF0\x9F\x9A\x80");
  CHECK(!scene->text(2, 1).has_value());  // splits "é"
  CHECK(!scene->text(4, 3).has_value());  // starts mid-rocket
  CHECK(!scene->text(7, 1).has_value());  // out of range
  CHECK(scene->text(7, 0) == "");
  CHECK(!scene->text(0xFFFFFFFFu, 2).has_value());
  CHECK(scene->node_children(0)->size() == 2u);
  CHECK(scene->children(0, 2)->size() == 2u);
  CHECK(!scene->children(1, 2).has_value());
  CHECK(!scene->node_children(3).has_value());
  CHECK(scene->row(3) == nullptr && scene->rect(3) == nullptr && scene->row(2) != nullptr);
  uint32_t visited = 0;
  scene->for_each_dirty([&](uint32_t index, const NveNodeRow& n, const NveLayoutRect& r) {
    CHECK(n.child_off == fx.node_rows[index].child_off);
    CHECK(r.width == static_cast<float>(index + 1u));
    ++visited;
  });
  CHECK(visited == 2u);
  CHECK(scene->removed_ids().size() == 1u && scene->removed_ids()[0] == 42u);
  CHECK(scene->find_table(kVisual)->u32(0, kOpacity) == 200u);
  CHECK(!scene->find_table(Key64{1}).has_value());
  CHECK(scene->table(0).has_value() && !scene->table(1).has_value());

  // A leading BOM is text, never stripped.
  SceneFixture bom;
  bom.text = "\xEF\xBB\xBF\xEF\xBB\xBFx";
  const NveSceneView bom_view = bom.view();
  const auto bom_scene = SceneReader::open(&bom_view, kSchema);
  CHECK(bom_scene.has_value() && bom_scene->text(0, 7) == "\xEF\xBB\xBF\xEF\xBB\xBFx");
  CHECK(bom_scene->text(0, 3) == "\xEF\xBB\xBF");
}

void test_scene_reader_rejects() {
  SceneFixture fx;
  SceneError err = SceneError::None;
  CHECK(!SceneReader::open(nullptr, kSchema, &err) && err == SceneError::NullView);

  NveSceneView v = fx.view();
  CHECK(!SceneReader::open(&v, Key64{kSchema.value ^ 1u}, &err) && err == SceneError::SchemaMismatch);

  v = fx.view();
  v.children_slab = nullptr;
  CHECK(!SceneReader::open(&v, kSchema, &err) && err == SceneError::NullSlab);

  v = fx.view();
  v.node_capacity = 0;
  v.rows = nullptr;  // null with zero count is fine
  v.rects = nullptr;
  v.dirty_count = 0;
  CHECK(SceneReader::open(&v, kSchema, &err).has_value());

  // Dirty / moved indices index `rows` / `rects` unchecked: every one must be in range.
  const std::array<uint32_t, 3> out_of_range{0u, 2u, 3u};
  v = fx.view();
  v.dirty_indices = out_of_range.data();
  v.dirty_count = 3u;
  CHECK(!SceneReader::open(&v, kSchema, &err) && err == SceneError::InvalidIndex);
  v = fx.view();
  v.moved_indices = out_of_range.data();
  v.moved_count = 3u;
  CHECK(!SceneReader::open(&v, kSchema, &err) && err == SceneError::InvalidIndex);
  v.moved_count = 2u;
  CHECK(SceneReader::open(&v, kSchema, &err).has_value() && err == SceneError::None);

  // Table slots index the 128-bit row `table_set`: in range and unique.
  for (const uint32_t slot : {128u, 0xFFFFFFFFu}) {
    std::vector<NveTableView> bad = fx.tables;
    bad[0].slot = slot;
    v = fx.view();
    v.tables = bad.data();
    CHECK(!SceneReader::open(&v, kSchema, &err) && err == SceneError::InvalidTable);
  }
  std::vector<NveTableView> twice{fx.tables[0], fx.tables[0]};
  twice[1].key_lo ^= 1u;  // another table, same slot
  v = fx.view();
  v.tables = twice.data();
  v.table_count = 2u;
  CHECK(!SceneReader::open(&v, kSchema, &err) && err == SceneError::InvalidTable);
  twice[1].slot = 127u;
  CHECK(SceneReader::open(&v, kSchema, &err).has_value());

  const std::array<uint8_t, 2> bad_utf8 = {0xC0, 0xAF};
  v = fx.view();
  v.text_slab = bad_utf8.data();
  v.text_slab_len = 2;
  CHECK(!SceneReader::open(&v, kSchema, &err) && err == SceneError::InvalidUtf8);

  fx.tables[0].words = 40u;
  v = fx.view();
  CHECK(!SceneReader::open(&v, kSchema, &err) && err == SceneError::InvalidTable);
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
  // A slab of u32::MAX records exceeds PTRDIFF_MAX on 32-bit hosts and must be rejected before a
  // span is formed; on 64-bit hosts the same count is representable (the children slab is never
  // read at open).
  SceneFixture fx;
  NveSceneView v = fx.view();
  v.children_slab_len = 0xFFFFFFFFu;
  SceneError err = SceneError::None;
  const bool opened = SceneReader::open(&v, kSchema, &err).has_value();
  if constexpr (sizeof(void*) == 4) {
    CHECK(!opened && err == SceneError::NullSlab);
  } else {
    CHECK(opened);  // representable; no read happens at open
  }
}

void test_scene_reader_and_session_mock() {
  struct MockState {
    int acquires = 0;
    int releases = 0;
    int destroys = 0;
    int wake_fires = 0;
    bool corrupt_view = false;
    NveSceneView view{};
  };
  static MockState* g_mock = nullptr;
  MockState state{};
  g_mock = &state;

  const std::array<NveNodeRow, 2> rows = {
      NveNodeRow{{0u, 0u, 0u, 0u}, 0u, 0u, 0u, 0u, 0xFFu, 0u, 0u},
      NveNodeRow{{1u, 0u, 0u, 0u}, 0u, 0u, 0u, 1u, 0u, 0u, static_cast<uint8_t>(NVE_NODE_ROW_FLAG_LIVE)},
  };
  const std::array<NveLayoutRect, 2> rects = {
      NveLayoutRect{0.0f, 0.0f, 0.0f, 0.0f},
      NveLayoutRect{10.0f, 20.0f, 120.0f, 40.0f},
  };
  state.view = NveSceneView{};
  state.view.schema_hash = NVE_RUNTIME_SCHEMA_HASH;
  state.view.frame_id = 3u;
  state.view.node_capacity = 2u;
  state.view.rows = rows.data();
  state.view.rects = rects.data();

  auto noop_i32 = [](NveRuntimeHandle) -> int32_t { return NVE_RUNTIME_OK; };
  NveRuntimeApi mock_api{};
  mock_api.abi_version = NVE_RUNTIME_ABI_VERSION;
  mock_api.struct_size = static_cast<uint32_t>(sizeof(NveRuntimeApi));
  mock_api.schema_hash = NVE_RUNTIME_SCHEMA_HASH;
  mock_api.create = [](const NveRuntimeConfig*) -> NveRuntimeHandle { return g_mock; };
  mock_api.destroy = [](NveRuntimeHandle) { ++g_mock->destroys; };
  mock_api.set_wake_callback = [](NveRuntimeHandle, NveRuntimeWakeFn cb, void* ctx) -> int32_t {
    if (cb != nullptr) cb(ctx);
    return NVE_RUNTIME_OK;
  };
  mock_api.set_viewport = [](NveRuntimeHandle, float, float, float) -> int32_t { return NVE_RUNTIME_OK; };
  mock_api.attach_app_interface = [](NveRuntimeHandle, NveGetInterfaceFn) -> int32_t { return NVE_RUNTIME_OK; };
  mock_api.submit_nve2 = [](NveRuntimeHandle, NveBufferSlice) -> int32_t { return NVE_RUNTIME_OK; };
  mock_api.tick = [](NveRuntimeHandle, float, uint32_t* c) -> int32_t {
    if (c != nullptr) *c = 0u;
    return NVE_RUNTIME_OK;
  };
  mock_api.dispatch_events = [](NveRuntimeHandle, NveEventBatch, uint32_t*) -> int32_t { return NVE_RUNTIME_OK; };
  mock_api.hit_test = [](NveRuntimeHandle, float, float, uint32_t* out) -> int32_t {
    *out = 1u;
    return NVE_RUNTIME_OK;
  };
  mock_api.scroll_by = [](NveRuntimeHandle, uint32_t, float, float, float*, float*) -> int32_t {
    return NVE_RUNTIME_OK;
  };
  mock_api.scene_acquire = [](NveRuntimeHandle, NveSceneView* out) -> int32_t {
    ++g_mock->acquires;
    *out = g_mock->view;
    if (g_mock->corrupt_view) out->schema_hash ^= 1ULL;
    return NVE_RUNTIME_OK;
  };
  mock_api.scene_release = [](NveRuntimeHandle) -> int32_t {
    ++g_mock->releases;
    return NVE_RUNTIME_OK;
  };
  mock_api.get_metrics = [](NveRuntimeHandle, NveRuntimeMetrics* m) -> int32_t {
    *m = NveRuntimeMetrics{};
    return NVE_RUNTIME_OK;
  };
  mock_api.pair_connect = [](NveRuntimeHandle, NveStrSlice, NveStrSlice) -> int32_t { return NVE_RUNTIME_OK; };
  mock_api.pair_disconnect = noop_i32;
  mock_api.pair_is_connected = [](NveRuntimeHandle) -> uint32_t { return 0u; };
  mock_api.last_error = [](uint8_t*, size_t) -> size_t { return 0u; };
  // An incomplete vtable (any NULL slot) is refused before `create` runs.
  CHECK(!nve::host::RuntimeSession::create(&mock_api).has_value());
  mock_api.set_text_measurer = [](NveRuntimeHandle, NveRuntimeMeasureTextFn, void*) -> int32_t {
    return NVE_RUNTIME_OK;
  };
  mock_api.set_props = [](NveRuntimeHandle, const NveMapEntry*, uint32_t, uint32_t*) -> int32_t {
    return NVE_RUNTIME_UNSUPPORTED;
  };
  mock_api.drain_emitted_events = [](NveRuntimeHandle, NveEventBatch*) -> int32_t {
    return NVE_RUNTIME_UNSUPPORTED;
  };
  mock_api.snapshot_state = [](NveRuntimeHandle, NveBufferSlice*) -> int32_t { return NVE_RUNTIME_UNSUPPORTED; };
  mock_api.restore_state = [](NveRuntimeHandle, NveBufferSlice) -> int32_t { return NVE_RUNTIME_UNSUPPORTED; };
  mock_api.apply_hot_patch = [](NveRuntimeHandle, NveBufferSlice, uint32_t*) -> int32_t {
    return NVE_RUNTIME_UNSUPPORTED;
  };
  mock_api.resource_payload = [](NveRuntimeHandle, uint32_t, uint64_t*, NveBufferSlice*) -> int32_t {
    return NVE_RUNTIME_NOT_FOUND;
  };
  mock_api.load_app_module = [](NveRuntimeHandle, NveStrSlice, uint32_t) -> int32_t { return NVE_RUNTIME_IO; };
  mock_api.save_state_file = [](NveRuntimeHandle, NveStrSlice, uint32_t) -> int32_t { return NVE_RUNTIME_NOT_FOUND; };
  mock_api.restore_state_file = [](NveRuntimeHandle, NveStrSlice, uint32_t) -> int32_t {
    return NVE_RUNTIME_NOT_FOUND;
  };
  mock_api.clear_state_file = [](NveRuntimeHandle, NveStrSlice) -> int32_t { return NVE_RUNTIME_OK; };

  {
    auto sess = nve::host::RuntimeSession::create(&mock_api);
    CHECK(sess.has_value());
    if (!sess) return;
    auto wake_cb = [](void* ctx) { ++static_cast<MockState*>(ctx)->wake_fires; };
    CHECK(sess->set_wake_callback(wake_cb, &state) == NVE_RUNTIME_OK);
    CHECK(state.wake_fires == 1);
    CHECK(sess->clear_wake_callback() == NVE_RUNTIME_OK);
    {
      auto guard = sess->acquire_scene();
      CHECK(guard.has_value());
      CHECK(guard->active());
      CHECK((*guard)->frame_id() == 3u && (*guard)->is_live(1u) && (*guard)->has_table(1u, 0u));
      CHECK((*guard)->rect(1u)->width == 120.0f);

      // Move-ctor clears moved-from guard spans so no dangling std::span remains.
      nve::host::RuntimeSceneGuard moved(std::move(*guard));
      CHECK(!guard->active());
      CHECK(guard->reader().rows().empty() && guard->reader().rects().empty());
      CHECK(moved.active() && moved->frame_id() == 3u);

      // Move-assign + self-move safety.
      nve::host::RuntimeSceneGuard assigned(std::move(moved));
      CHECK(!moved.active() && moved.reader().rows().empty());
      nve::host::RuntimeSceneGuard* self_ptr = &assigned;
      assigned = std::move(*self_ptr);
      CHECK(assigned.active() && assigned->frame_id() == 3u);

      // Non-self move-assignment releases prior guard_a lock immediately and clears moved-from guard_b.
      auto guard_b = sess->acquire_scene();
      CHECK(guard_b.has_value() && guard_b->active());
      const int releases_before = state.releases;
      assigned = std::move(*guard_b);
      CHECK(state.releases == releases_before + 1);
      CHECK(!guard_b->active());
      CHECK(guard_b->reader().rows().empty() && guard_b->reader().rects().empty());
      CHECK(guard_b->raw().rows == nullptr && guard_b->raw().node_capacity == 0u);
      CHECK(assigned.active() && assigned->frame_id() == 3u);

      // Explicit release() clears raw_ and reader_ spans immediately.
      assigned.release();
      CHECK(!assigned.active());
      CHECK(assigned.reader().rows().empty() && assigned.reader().rects().empty());
      CHECK(assigned.raw().rows == nullptr && assigned.raw().node_capacity == 0u);
    }
    CHECK(state.acquires == 2 && state.releases == 2);

    // Misaligned slab pointer MUST be rejected by SceneReader::open() with MisalignedSlab.
    alignas(NveNodeRow) uint8_t unaligned_buf[sizeof(NveNodeRow) * 2u + 4u]{};
    NveSceneView misaligned = state.view;
    misaligned.rows = reinterpret_cast<const NveNodeRow*>(unaligned_buf + 1u);
    SceneError align_err = SceneError::None;
    CHECK(!nve::host::SceneReader::open(&misaligned, Key64{NVE_RUNTIME_SCHEMA_HASH}, &align_err)
               .has_value());
    CHECK(align_err == SceneError::MisalignedSlab);
    // `moved_indices`: NULL with a count, or misaligned, is rejected the same way.
    NveSceneView moved = state.view;
    moved.moved_count = 2u;
    moved.moved_indices = nullptr;
    align_err = SceneError::None;
    CHECK(!nve::host::SceneReader::open(&moved, Key64{NVE_RUNTIME_SCHEMA_HASH}, &align_err).has_value());
    CHECK(align_err == SceneError::NullSlab);
    moved.moved_indices = reinterpret_cast<const uint32_t*>(unaligned_buf + 1u);
    align_err = SceneError::None;
    CHECK(!nve::host::SceneReader::open(&moved, Key64{NVE_RUNTIME_SCHEMA_HASH}, &align_err).has_value());
    CHECK(align_err == SceneError::MisalignedSlab);
    alignas(uint32_t) const std::array<uint32_t, 2> moved_ids = {1u, 1u};
    moved.moved_indices = moved_ids.data();
    CHECK(nve::host::SceneReader::open(&moved, Key64{NVE_RUNTIME_SCHEMA_HASH}).has_value());

    // Corrupted view MUST release reader lock immediately instead of leaking!
    state.corrupt_view = true;
    SceneError err = SceneError::None;
    CHECK(!sess->acquire_scene(nullptr, &err).has_value());
    CHECK(err == SceneError::SchemaMismatch);
    CHECK(state.acquires == 3 && state.releases == 3);
  }
  CHECK(state.destroys == 1);
  g_mock = nullptr;
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
  test_scene_reader_and_session_mock();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_table_scene_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_table_scene_test: ok\n");
  return 0;
}
