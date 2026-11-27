// C++20 suite over the *generated* `nve_abi.generated.h` / `nve_tables.generated.h`: the static
// readers must work on the real generated structs, and every generated table key and field spec
// must agree with this package's own XXH64 and field validation (checked at compile time).
#include <array>
#include <cstdint>
#include <cstring>
#include <string_view>

#include "check.h"
#include "nve/generated/nve_abi.generated.h"
#include "nve/generated/nve_tables.generated.h"
#include "nve/host/scene_reader.hpp"
#include "nve/host/table_reader.hpp"

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

void test_scene_over_generated_structs() {
  std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> row{};
  std::memcpy(row.data(), NVE_TABLE_LAYOUT_DEFAULTS, sizeof row);
  const NveTableView table{static_cast<uint32_t>(NVE_TABLE_LAYOUT_KEY & 0xFFFFFFFFu),
                           static_cast<uint32_t>(NVE_TABLE_LAYOUT_KEY >> 32), 0u, NVE_TABLE_LAYOUT_WORDS,
                           1u, 0u, row.data(), nullptr, nullptr};
  NveRenderNode node{};
  node.node_id = NVE_ROOT_NODE_ID;
  node.primary_slot = NVE_PRIMARY_SLOT_NONE;
  node.text_len = 2;
  const char text[] = "hi";
  NveSceneView view{};
  view.schema_hash = NVE_SCENE_SCHEMA_HASH;
  view.frame_id = 7;
  view.node_count = 1;
  view.text_slab_len = 2;
  view.table_count = 1;
  view.nodes = &node;
  view.text_slab = reinterpret_cast<const uint8_t*>(text);
  view.tables = &table;

  nve::host::SceneError err{};
  const auto scene = SceneReader<NveSceneView>::open(&view, Key64{NVE_SCENE_SCHEMA_HASH}, &err);
  CHECK(scene.has_value());
  CHECK(scene->table_count() == 1u);
  CHECK(scene->node(0) != nullptr && scene->node(0)->node_id == NVE_ROOT_NODE_ID);
  CHECK(scene->text(0, 2) == std::optional<std::string_view>("hi"));
  CHECK(!SceneReader<NveSceneView>::open(&view, Key64{NVE_CORE_ABI_SCHEMA_HASH}, &err).has_value());

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
                          NVE_TABLE_ACCESSIBILITY_WORDS, 1u, 0u, row.data(), nullptr, nullptr};
  const std::optional<TableReader> table = TableReader::from_view(view);
  CHECK(table.has_value());
  CHECK(table->str(0, NVE_TABLE_ACCESSIBILITY_LABEL) == std::optional<nve::host::StrSpan>(nve::host::StrSpan{5u, 7u}));
}

}  // namespace

int main() {
  test_scene_over_generated_structs();
  test_generated_str_field();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_generated_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_generated_test: ok (%u tables)\n", NVE_TABLE_COUNT);
  return 0;
}
