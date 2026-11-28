// C++20 suite over the *generated* `nve_abi.generated.h` / `nve_tables.generated.h`: the static
// readers must work on the real generated structs, and every generated table key and field spec
// must agree with this package's own XXH64 and field validation (checked at compile time).
#include <array>
#include <cstdint>
#include <cstring>
#include <optional>
#include <span>
#include <vector>
#include <string_view>

#include "check.h"
#include "nve/generated/nve_abi.generated.h"
#include "nve/generated/nve_tables.generated.h"
#include "nve/host/scene_reader.hpp"
#include "nve/host/table_reader.hpp"
#include "nve/host/value_tree.hpp"

namespace {

using nve::host::Key64;
using nve::host::SceneReader;
using nve::host::TableReader;

// Every key is `xxh64(name)`, recomputed here by the static constexpr XXH64.
#define NVE_CHECK_KEY(IDENT, NAME)                                                   \
  static_assert(Key64::of(NAME).value == NVE_TABLE_##IDENT##_KEY, NAME " key");     \
  static_assert(std::string_view(NVE_TABLE_##IDENT##_NAME) == NAME, NAME " name");
NVE_TABLE_LIST(NVE_CHECK_KEY)
#undef NVE_CHECK_KEY

// Every field spec is valid and fits its row (`Str` needs its second word).
#define NVE_CHECK_FIELD(TABLE, FIELD) \
  static_assert(nve_field_fits(NVE_TABLE_##TABLE##_##FIELD, NVE_TABLE_##TABLE##_WORDS), #TABLE "." #FIELD);
#define NVE_CHECK_TABLE_FIELDS(IDENT, NAME) NVE_TABLE_##IDENT##_FIELD_LIST(NVE_CHECK_FIELD)
NVE_TABLE_LIST(NVE_CHECK_TABLE_FIELDS)
#undef NVE_CHECK_TABLE_FIELDS
#undef NVE_CHECK_FIELD

#define NVE_COUNT_TABLE(IDENT, NAME) +1
static_assert(0 NVE_TABLE_LIST(NVE_COUNT_TABLE) == NVE_TABLE_COUNT);
#undef NVE_COUNT_TABLE

void test_scene_over_generated_runtime_view() {
  std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> row{};
  std::memcpy(row.data(), NVE_TABLE_LAYOUT_DEFAULTS, sizeof row);
  const NveTableView table{static_cast<uint32_t>(NVE_TABLE_LAYOUT_KEY & 0xFFFFFFFFu),
                           static_cast<uint32_t>(NVE_TABLE_LAYOUT_KEY >> 32), 0u, NVE_TABLE_LAYOUT_WORDS,
                           1u, 0u, 0u, 0u, row.data(), nullptr, nullptr, nullptr, nullptr};
  const std::array<NveNodeRow, 1> rows = {NveNodeRow{}};
  const std::array<NveLayoutRect, 1> rects = {NveLayoutRect{}};
  const char text[] = "hi";
  NveSceneView view{};
  view.schema_hash = NVE_RUNTIME_SCHEMA_HASH;
  view.frame_id = 7;
  view.node_capacity = 1;
  view.rows = rows.data();
  view.rects = rects.data();
  view.text_slab_len = 2;
  view.table_count = 1;
  view.text_slab = reinterpret_cast<const uint8_t*>(text);
  view.tables = &table;

  nve::host::SceneError err{};
  const auto scene = SceneReader::open(&view, Key64{NVE_RUNTIME_SCHEMA_HASH}, &err);
  CHECK(scene.has_value());
  CHECK(scene->table_count() == 1u && scene->node_capacity() == 1u);
  CHECK(scene->row(0) != nullptr);
  CHECK(scene->text(0, 2) == std::optional<std::string_view>("hi"));
  CHECK(!SceneReader::open(&view, Key64{NVE_CORE_ABI_SCHEMA_HASH}, &err).has_value());
  CHECK(err == nve::host::SceneError::SchemaMismatch);

  const std::optional<TableReader> layout = scene->find_table(Key64{NVE_TABLE_LAYOUT_KEY});
  CHECK(layout.has_value());
  // `flex_shrink` (default 1.0) is the 13th word of the default row.
  CHECK(layout->f32(0, NVE_TABLE_LAYOUT_FLEX_SHRINK) == std::optional<float>(1.0f));
  CHECK(!scene->find_table(Key64{NVE_TABLE_VISUAL_KEY}).has_value());
}

// A generated two-word `Str` spec reads the `(offset, len)` span through the static reader.
void test_generated_str_field() {
  std::array<uint32_t, NVE_TABLE_ACCESSIBILITY_WORDS> row{};
  row[NVE_TABLE_ACCESSIBILITY_LABEL.word] = 5u;
  row[NVE_TABLE_ACCESSIBILITY_LABEL.word + 1u] = 7u;
  const NveTableView view{static_cast<uint32_t>(NVE_TABLE_ACCESSIBILITY_KEY & 0xFFFFFFFFu),
                          static_cast<uint32_t>(NVE_TABLE_ACCESSIBILITY_KEY >> 32), 0u,
                          NVE_TABLE_ACCESSIBILITY_WORDS, 1u, 0u, 0u, 0u, row.data(), nullptr, nullptr, nullptr, nullptr};
  const std::optional<TableReader> table = TableReader::from_view(view);
  CHECK(table.has_value());
  CHECK(table->str(0, NVE_TABLE_ACCESSIBILITY_LABEL) == std::optional<nve::host::StrSpan>(nve::host::StrSpan{5u, 7u}));
}

}  // namespace

void test_value_tree_mirrors_the_dynamic_value() {
  // {"k": [1, "\xC3\xA9", null, true, 1.5, b"\xFF"]}
  nve::host::BinWriter w;
  const auto map = w.begin_map(1);
  w.write_map_key("k");
  const auto arr = w.begin_array(6);
  w.write_i64(1);
  w.write_str("\xC3\xA9");
  w.write_null();
  w.write_bool(true);
  w.write_f64(1.5);
  const uint8_t ff = 0xFF;
  w.write_bytes(std::span<const uint8_t>(&ff, 1));
  w.end(arr);
  w.end(map);
  const std::optional<nve::host::DynamicValue> v = nve::host::DynamicValue::from_bytes(w.bytes());
  CHECK(v.has_value());
  const nve::host::NveValueTree tree(*v);
  const NveValue& root = tree.root();
  CHECK(root.tag == NVE_VAL_MAP && root.data.map_val.len == 1u);
  const NveMapEntry& e = root.data.map_val.ptr[0];
  CHECK(std::string_view(reinterpret_cast<const char*>(e.key.ptr), e.key.len) == "k");
  CHECK(e.value.tag == NVE_VAL_ARRAY && e.value.data.array_val.len == 6u);
  const NveValue* items = e.value.data.array_val.ptr;
  CHECK(items[0].tag == NVE_VAL_I64 && items[0].data.i64_val == 1);
  CHECK(items[1].tag == NVE_VAL_STR && items[1].data.str_val.len == 2u);
  CHECK(items[2].tag == NVE_VAL_NULL);
  CHECK(items[3].tag == NVE_VAL_BOOL && items[3].data.bool_val == 1u);
  CHECK(items[4].tag == NVE_VAL_F64 && items[4].data.f64_val == 1.5);
  CHECK(items[5].tag == NVE_VAL_BYTES && items[5].data.bytes_val.len == 1u && items[5].data.bytes_val.ptr[0] == 0xFF);
  // Deep nesting keeps every container slice valid (children append while parents are open).
  nve::host::BinWriter deep;
  std::vector<nve::host::BinWriter::Mark> marks;
  for (int i = 0; i < 40; ++i) {
    marks.push_back(deep.begin_array(2));
    deep.write_i64(i);
  }
  deep.write_null();
  for (auto it = marks.rbegin(); it != marks.rend(); ++it) deep.end(*it);
  const std::optional<nve::host::DynamicValue> dv = nve::host::DynamicValue::from_bytes(deep.bytes());
  CHECK(dv.has_value());
  const nve::host::NveValueTree deep_tree(*dv);
  const NveValue* node = &deep_tree.root();
  for (int i = 0; i < 40; ++i) {
    CHECK(node->tag == NVE_VAL_ARRAY && node->data.array_val.len == 2u);
    CHECK(node->data.array_val.ptr[0].data.i64_val == i);
    node = &node->data.array_val.ptr[1];
  }
  CHECK(node->tag == NVE_VAL_NULL);
}

int main() {
  test_value_tree_mirrors_the_dynamic_value();
  test_scene_over_generated_runtime_view();
  test_generated_str_field();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_generated_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_generated_test: ok (%u tables)\n", NVE_TABLE_COUNT);
  return 0;
}
