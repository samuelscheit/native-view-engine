// C++20 live suite (links `libnve_runtime`): `Nve2FrameBuilder` frames declaring the generated
// core tables, decoded by the runtime and read back through `RuntimeSceneReader` + the generated
// field specs; `resource_bytes`; and an instant-boot `SnapshotFile` round trip via `encode_scene`.
#include <array>
#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <string_view>
#include <vector>

#include "check.h"
#include "nve/host/nve2_builder.hpp"
#include "nve/host/runtime_session.hpp"
#include "nve/host/snapshot_file.hpp"

using nve::host::Key64;
using nve::host::Nve2FrameBuilder;
using nve::host::RuntimeSession;
using nve::host::SnapshotFile;
using nve::host::set_field;

namespace {

constexpr std::string_view kUrl = "https://cdn.example.com/a.png";

std::vector<uint8_t> image_src_url(std::string_view url) {
  std::vector<uint8_t> out{0u};
  const auto n = static_cast<uint32_t>(url.size());
  for (uint32_t s = 0u; s < 32u; s += 8u) out.push_back(static_cast<uint8_t>((n >> s) & 0xFFu));
  out.insert(out.end(), url.begin(), url.end());
  return out;
}

/// ROOT(1) -> [card(2): visual bg + text "Hi", image(3): src resource 7].
std::vector<uint8_t> build_scene() {
  const uint8_t visual = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_VISUAL_KEY});
  const uint8_t text = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_TEXT_KEY});
  const uint8_t image = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_IMAGE_KEY});
  const uint8_t layout = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_LAYOUT_KEY});
  CHECK(visual != 0xFFu && text != 0xFFu && image != 0xFFu && layout != 0xFFu);

  Nve2FrameBuilder b;
  b.declare_core_tables().create_nodes({1u, 2u, 3u});
  const std::vector<uint8_t> src = image_src_url(kUrl);
  b.define_resource(7u, Key64::of("nve.res.image_src"), src);

  std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> lay{};
  std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
  const uint32_t lay_mask = set_field(lay, NVE_LAYOUT_WIDTH, 0x42C80000u) |  // 100pt (tag 0)
                            set_field(lay, NVE_LAYOUT_HEIGHT, 0x42480000u);   // 50pt
  b.patch(layout, 3u, lay_mask, lay);

  std::array<uint32_t, NVE_TABLE_VISUAL_WORDS> vis{};
  std::copy(std::begin(NVE_TABLE_VISUAL_DEFAULTS), std::end(NVE_TABLE_VISUAL_DEFAULTS), vis.begin());
  const uint32_t vis_mask = set_field(vis, NVE_VISUAL_BACKGROUND, 0xFF0000FFu) | set_field(vis, NVE_VISUAL_BORDER_RADIUS, 0x41000000u);
  b.patch(visual, 2u, vis_mask, vis);
  b.set_string(text, 2u, NVE_TEXT_TEXT.word, "Hi");

  std::array<uint32_t, NVE_TABLE_IMAGE_WORDS> img{};
  std::copy(std::begin(NVE_TABLE_IMAGE_DEFAULTS), std::end(NVE_TABLE_IMAGE_DEFAULTS), img.begin());
  const uint32_t img_mask = set_field(img, NVE_IMAGE_SRC, 7u) | set_field(img, NVE_IMAGE_FIT, NVE_IMAGE_FIT_CONTAIN);
  b.patch(image, 3u, img_mask, img);
  b.set_children(1u, {2u, 3u});
  return b.finish();
}

void check_scene(const RuntimeSession& s) {
  auto guard = s.acquire_scene();
  CHECK(guard.has_value());
  if (!guard) return;
  const auto& scene = guard->reader();
  CHECK(scene.is_live(2u) && scene.is_live(3u));
  const auto vis = scene.find_table(Key64{NVE_TABLE_VISUAL_KEY});
  const auto text = scene.find_table(Key64{NVE_TABLE_TEXT_KEY});
  const auto image = scene.find_table(Key64{NVE_TABLE_IMAGE_KEY});
  CHECK(vis.has_value() && text.has_value() && image.has_value());
  if (!vis || !text || !image) return;
  CHECK(vis->color(2u, NVE_VISUAL_BACKGROUND).has_value());
  CHECK(vis->color(2u, NVE_VISUAL_BACKGROUND)->packed == 0xFF0000FFu);
  CHECK(vis->f32(2u, NVE_VISUAL_BORDER_RADIUS) == 8.0f);
  CHECK(vis->f32(2u, NVE_VISUAL_OPACITY) == 1.0f);  // unpatched words keep their defaults
  // Dense rows exist for every slot, but only nodes whose `table_set` holds the table own one:
  // ROOT has no `nve.visual`, so hosts must gate dense reads on `has_table`.
  CHECK(!scene.has_table(1u, static_cast<uint8_t>(vis->slot())) && scene.has_table(2u, static_cast<uint8_t>(vis->slot())));
  const auto text_row = text->row_of(2u | (uint32_t{scene.row(2u)->generation} << 24u));
  CHECK(text_row.has_value());
  if (text_row) {
    const auto span = text->str(*text_row, NVE_TEXT_TEXT);
    CHECK(span.has_value() && scene.text(span->offset, span->len) == "Hi");
  }
  const auto img_row = image->row_of(3u | (uint32_t{scene.row(3u)->generation} << 24u));
  CHECK(img_row.has_value());
  if (img_row) {
    CHECK(image->u32(*img_row, NVE_IMAGE_SRC) == 7u);
    CHECK(image->u32(*img_row, NVE_IMAGE_FIT) == NVE_IMAGE_FIT_CONTAIN);
  }
  const NveLayoutRect* r = scene.rect(3u);
  CHECK(r != nullptr && r->width == 100.0f && r->height == 50.0f);
}

}  // namespace

int main() {
  const NveRuntimeApi* api = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
  auto live = RuntimeSession::create(api, RuntimeSession::default_config(1u, 320.0f, 240.0f, 2.0f));
  CHECK(live.has_value());
  if (!live) return g_failures;

  const std::vector<uint8_t> frame = build_scene();
  CHECK(live->submit_nve2(frame) == NVE_RUNTIME_OK);
  bool changed = false;
  CHECK(live->tick(0.0f, &changed) == NVE_RUNTIME_OK);
  check_scene(*live);

  std::vector<uint8_t> res;
  CHECK(live->resource_bytes(7u, res) == NVE_RUNTIME_OK);
  CHECK(res == image_src_url(kUrl));
  CHECK(live->resource_bytes(8u, res) == NVE_RUNTIME_NOT_FOUND && res.empty());

  // Instant boot: encode -> atomic file -> decode -> replay into a cold surface.
  SnapshotFile snap;
  snap.schema = 3u;
  CHECK(live->encode_scene(snap.frame0) == NVE_RUNTIME_OK && snap.frame0.size() > 24u);
  snap.state = {1u, 2u, 3u};
  const auto path = std::filesystem::temp_directory_path() / "nve_host_cxx_snapshot_test.nvesnap";
  CHECK(snap.write(path));
  CHECK(!SnapshotFile::read(path, 4u).has_value());  // stale schema is refused
  const auto loaded = SnapshotFile::read(path, 3u);
  CHECK(loaded.has_value() && loaded->frame0 == snap.frame0 && loaded->state == snap.state);
  std::filesystem::remove(path);
  auto truncated = snap.encode();
  truncated.pop_back();
  CHECK(!SnapshotFile::decode(truncated, 3u).has_value());
  CHECK(!SnapshotFile::decode(std::span<const uint8_t>(), 3u).has_value());
  CHECK(SnapshotFile::default_path("app-a") != SnapshotFile::default_path("app-b"));

  auto cold = RuntimeSession::create(api, RuntimeSession::default_config(2u, 320.0f, 240.0f, 2.0f));
  CHECK(cold.has_value());
  if (cold && loaded) {
    CHECK(cold->submit_nve2(loaded->frame0) == NVE_RUNTIME_OK);
    CHECK(cold->tick(0.0f, &changed) == NVE_RUNTIME_OK);
    check_scene(*cold);
    std::vector<uint8_t> replayed;
    CHECK(cold->resource_bytes(7u, replayed) == NVE_RUNTIME_OK && replayed == image_src_url(kUrl));
  }
  std::printf("nve_host_runtime_scene_builder: %s\n", g_failures == 0 ? "ok" : "FAILED");
  return g_failures;
}