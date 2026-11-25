#include "test_common.hpp"

#include <unordered_set>

namespace nve_test {

namespace {

const NveRenderNode* find_scene_node(const NveSceneView& view, uint32_t node_id) noexcept {
  for (uint32_t i = 0u; i < view.node_count; ++i) {
    if (view.nodes[i].node_id == node_id) {
      return &view.nodes[i];
    }
  }
  return nullptr;
}

std::unordered_set<uint32_t> dirty_node_ids(const NveSceneView& view) {
  std::unordered_set<uint32_t> out;
  for (uint32_t i = 0u; i < view.dirty_count; ++i) {
    const uint32_t idx = view.dirty_indices[i];
    assert(idx < view.node_count);
    out.insert(view.nodes[idx].node_id);
  }
  return out;
}

struct MockSceneViewHandle {
  uint32_t id{0};
  uint8_t kind{0};
  uint8_t flags{0};
  std::string text;
  float x{0.0f};
  float y{0.0f};
  float width{0.0f};
  float height{0.0f};
  uint32_t bg_rgba{0};
  uint32_t text_rgba{0};
  std::vector<uint32_t> children;
  int create_count{0};
  int update_count{0};
};

struct MockIncrementalSceneAdapter {
  int root_updates{0};
  int creates{0};
  int updates{0};
  int destroys{0};
  int child_syncs{0};
  std::vector<uint32_t> root_children;
  std::vector<uint32_t> destroyed_ids;

  void update_root(const NveSceneView& /*view*/, const NveRenderNode& node) {
    assert(node.node_id == NVE_ROOT_NODE_ID);
    ++root_updates;
  }

  MockSceneViewHandle create_view(const NveSceneView& /*view*/, const NveRenderNode& node) {
    ++creates;
    MockSceneViewHandle h{};
    h.id = node.node_id;
    h.kind = node.kind;
    h.flags = node.flags;
    h.create_count = 1;
    return h;
  }

  void update_view(
      const NveSceneView& view,
      MockSceneViewHandle& h,
      const NveRenderNode& node) {
    ++updates;
    ++h.update_count;
    h.kind = node.kind;
    h.flags = node.flags;
    h.text = std::string(nve::scene_node_text(view, node));
    h.x = node.x;
    h.y = node.y;
    h.width = node.width;
    h.height = node.height;
    h.bg_rgba = node.bg_rgba;
    h.text_rgba = node.text_rgba;
  }

  void destroy_view(uint32_t node_id, MockSceneViewHandle& /*h*/) {
    ++destroys;
    destroyed_ids.push_back(node_id);
  }

  template <typename LookupFn>
  void sync_children(
      uint32_t parent_id,
      MockSceneViewHandle* parent_view,
      std::span<const uint32_t> child_ids,
      LookupFn&& lookup_fn) {
    ++child_syncs;
    std::vector<uint32_t> mounted;
    mounted.reserve(child_ids.size());
    for (uint32_t cid : child_ids) {
      if (MockSceneViewHandle* child = lookup_fn(cid)) {
        mounted.push_back(child->id);
      }
    }
    if (parent_id == NVE_ROOT_NODE_ID) {
      root_children = std::move(mounted);
    } else if (parent_view != nullptr) {
      parent_view->children = std::move(mounted);
    }
  }
};

void test_direct_scene_view_o_changed_and_image_src() {
  // Null host yields valid empty schema view.
  {
    const NveSceneView empty = nve::fabric_host_scene_view(nullptr);
    assert(empty.schema_hash == NVE_SCENE_SCHEMA_HASH);
    assert(empty.node_count == 0u);
    assert(empty.nodes == nullptr);
  }

  NveSurfaceConfig cfg{.surface_id = 1, .width = 400.0f, .height = 600.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);

  // Immediately after creation (before submit_frame), scene view has valid root node.
  {
    const NveSceneView v0 = nve::fabric_host_scene_view(host);
    assert(v0.schema_hash == NVE_SCENE_SCHEMA_HASH);
    assert(v0.node_count == 1u);
    assert(find_scene_node(v0, NVE_ROOT_NODE_ID) != nullptr);
  }

  // Frame 1: Mount a 60-node tree + an Image node with `src` + sparse high node IDs (> 65536).
  constexpr uint32_t kImageId = 70001u;
  constexpr uint32_t kSparseTextId = 131077u;
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(2, NVE_COMP_SCROLL_VIEW);
    nve::LayoutProps list_lp{};
    list_lp.width = {2, 100.0f};
    list_lp.height = {2, 100.0f};
    list_lp.flex_direction = 0;
    enc.set_layout(2, list_lp);

    std::vector<uint32_t> list_children;
    for (uint32_t i = 0u; i < 10u; ++i) {
      const uint32_t row_id = 10u + i * 2u;
      const uint32_t text_id = row_id + 1u;
      enc.create_node(row_id, NVE_COMP_VIEW);
      nve::LayoutProps row_lp{};
      row_lp.width = {2, 100.0f};
      row_lp.height = {1, 30.0f};
      row_lp.flex_direction = 1;
      row_lp.flex_shrink = 0.0f;
      enc.set_layout(row_id, row_lp);

      enc.create_node(text_id, NVE_COMP_TEXT);
      nve::StyleProps st{};
      st.text_color_rgba = 0xFF112233u;
      st.font_size = 14.0f;
      enc.set_style(text_id, st);
      enc.set_text(text_id, "Row " + std::to_string(i));
      enc.set_children(row_id, {text_id});
      list_children.push_back(row_id);
    }

    enc.create_node(kImageId, NVE_COMP_IMAGE);
    nve::LayoutProps img_lp{};
    img_lp.width = {1, 48.0f};
    img_lp.height = {1, 48.0f};
    img_lp.flex_shrink = 0.0f;
    enc.set_layout(kImageId, img_lp);
    enc.set_raw_prop_string(kImageId, "src", "https://cdn.example.com/avatar.png");
    enc.set_raw_prop_string(kImageId, "accessibilityLabel", "User avatar");
    enc.set_raw_prop_bool(kImageId, "disabled", false);
    enc.set_raw_prop_i64(kImageId, "priority", 3);
    enc.set_raw_prop_f64(kImageId, "aspectRatio", 1.5);
    enc.set_raw_prop_bytes(kImageId, "hash", std::vector<uint8_t>{0xDE, 0xAD});
    list_children.push_back(kImageId);

    enc.create_node(kSparseTextId, NVE_COMP_TEXT);
    enc.set_text(kSparseTextId, "Sparse high ID node");
    list_children.push_back(kSparseTextId);

    enc.set_children(2, list_children);
    enc.set_children(NVE_ROOT_NODE_ID, {2});
    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  }

  // Verify NveSceneView fields & RenderedNodeSnapshot::raw_props.
  {
    const NveSceneView v1 = nve::fabric_host_scene_view(host);
    assert(v1.schema_hash == NVE_SCENE_SCHEMA_HASH);
    assert(v1.frame_id == 1u);
    assert(v1.node_count == 24u); // root + list + 10*2 + image + sparse_text
    assert(v1.removed_count == 0u);
    assert(v1.dirty_count == 24u);

    const NveRenderNode* img = find_scene_node(v1, kImageId);
    assert(img != nullptr);
    assert(img->kind == NVE_NODE_KIND_IMAGE);
    assert(approx_eq(img->width, 48.0f) && approx_eq(img->height, 48.0f));
    assert(nve::scene_node_text(v1, *img) == "https://cdn.example.com/avatar.png");

    const NveRenderNode* sparse = find_scene_node(v1, kSparseTextId);
    assert(sparse != nullptr);
    assert(sparse->kind == NVE_NODE_KIND_TEXT);
    assert((sparse->flags & NVE_NODE_LEAF_TEXT) != 0u);
    assert(nve::scene_node_text(v1, *sparse) == "Sparse high ID node");

    const NveRenderNode* list = find_scene_node(v1, 2u);
    assert(list != nullptr);
    assert((list->flags & NVE_NODE_CLIP_BOUNDS) != 0u);
    assert(nve::scene_node_children(v1, *list).size() == 12u);

    // Verify RenderedNodeSnapshot::raw_props populated directly (no frame0 fallback scan needed).
    const auto snaps = snapshot_by_id(host);
    const auto& img_snap = snaps.at(kImageId);
    assert(std::get<std::string>(img_snap.raw_props.at("src")) ==
           "https://cdn.example.com/avatar.png");
    assert(std::get<std::string>(img_snap.raw_props.at("accessibilityLabel")) == "User avatar");
    assert(std::get<bool>(img_snap.raw_props.at("disabled")) == false);
    assert(std::get<int64_t>(img_snap.raw_props.at("priority")) == 3);
    assert(std::fabs(std::get<double>(img_snap.raw_props.at("aspectRatio")) - 1.5) < 1e-6);
    assert(std::get<std::vector<uint8_t>>(img_snap.raw_props.at("hash")) ==
           (std::vector<uint8_t>{0xDE, 0xAD}));
    assert(snaps.at(11u).raw_props.empty());
  }

  // Frame 2: Incremental O(changed) patch touching ONLY 2 nodes (1 text + 1 image src).
  {
    CommandBufferEncoder enc(1, 2);
    enc.set_text(11u, "Row 0 updated");
    nve::StyleProps st{};
    st.text_color_rgba = 0xFF38BDF8u;
    st.font_size = 14.0f;
    enc.set_style(11u, st);
    enc.set_raw_prop_string(kImageId, "uri", "https://cdn.example.com/avatar-v2.png");
    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

    const NveSceneView v2 = nve::fabric_host_scene_view(host);
    assert(v2.frame_id == 2u);
    assert(v2.removed_count == 0u);
    assert(v2.dirty_count == 2u);
    const auto dirty = dirty_node_ids(v2);
    assert(dirty.count(11u) == 1u);
    assert(dirty.count(kImageId) == 1u);

    const NveRenderNode* t11 = find_scene_node(v2, 11u);
    assert(t11 != nullptr && nve::scene_node_text(v2, *t11) == "Row 0 updated");
    assert(t11->text_rgba == 0xFF38BDF8u);

    const NveRenderNode* img = find_scene_node(v2, kImageId);
    assert(img != nullptr &&
           nve::scene_node_text(v2, *img) == "https://cdn.example.com/avatar-v2.png");
  }

  // Frame 3: Remove sparse text node AND recreate node 13 in the same frame.
  {
    CommandBufferEncoder enc(1, 3);
    enc.remove_node(kSparseTextId);
    enc.create_node(13u, NVE_COMP_TEXT);
    enc.set_text(13u, "Recreated row 1 text");
    enc.set_children(12u, {13u});
    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);

    const NveSceneView v3 = nve::fabric_host_scene_view(host);
    assert(v3.frame_id == 3u);
    assert(v3.node_count == 23u);
    assert(v3.removed_count == 1u);
    assert(v3.removed_ids[0] == kSparseTextId);
    // Node 13 was recreated in the same frame so it must NOT be in removed_ids.
    const NveRenderNode* n13 = find_scene_node(v3, 13u);
    assert(n13 != nullptr && nve::scene_node_text(v3, *n13) == "Recreated row 1 text");
    const auto dirty = dirty_node_ids(v3);
    assert(dirty.count(2u) == 1u);  // parent of removed kSparseTextId
    assert(dirty.count(13u) == 1u); // recreated node
  }

  // Resize surface: lazy scene publication flushes updated layout into NveSceneView.
  {
    assert(nve::fabric_host_set_surface_size(host, 520.0f, 700.0f, 2.0f));
    const NveSceneView v_resized = nve::fabric_host_scene_view(host);
    const NveRenderNode* root = find_scene_node(v_resized, NVE_ROOT_NODE_ID);
    assert(root != nullptr && approx_eq(root->width, 520.0f) && approx_eq(root->height, 700.0f));
    const auto dirty = dirty_node_ids(v_resized);
    assert(dirty.count(NVE_ROOT_NODE_ID) == 1u);
    assert(dirty.count(2u) == 1u);
  }

  // Text slab compaction stress test (> 64KB churned text bytes).
  {
    std::string payload(512u, 'A');
    for (uint64_t f = 4u; f < 160u; ++f) {
      payload[0] = static_cast<char>('A' + (f % 26u));
      CommandBufferEncoder enc(1, f);
      enc.set_text(11u, payload);
      enc.commit(true);
      assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
    }
    const NveSceneView v_compacted = nve::fabric_host_scene_view(host);
    assert(v_compacted.text_slab_len < 65536u);
    const NveRenderNode* t11 = find_scene_node(v_compacted, 11u);
    assert(t11 != nullptr && nve::scene_node_text(v_compacted, *t11) == payload);
    const NveRenderNode* img = find_scene_node(v_compacted, kImageId);
    assert(img != nullptr &&
           nve::scene_node_text(v_compacted, *img) == "https://cdn.example.com/avatar-v2.png");
  }

  nve_fabric_host_destroy(host);
}

void test_incremental_scene_reconciler_and_session_scene_view() {
  NveSurfaceConfig cfg{.surface_id = 1, .width = 360.0f, .height = 640.0f, .scale_factor = 2.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);

  // Reject malformed schema hash and null pointers.
  {
    nve::IncrementalSceneReconciler<MockSceneViewHandle, MockIncrementalSceneAdapter> rec;
    MockIncrementalSceneAdapter adapter;
    NveSceneView bad{};
    bad.schema_hash = 0xDEADBEEFULL;
    assert(!rec.sync(bad, adapter));
    bad.schema_hash = NVE_SCENE_SCHEMA_HASH;
    bad.node_count = 1u;
    bad.nodes = nullptr;
    assert(!rec.sync(bad, adapter));
  }

  // Frame 1: Initial mount (root -> container 2 -> [leaf text 3, leaf text 4, image 5]).
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(2, NVE_COMP_VIEW);
    enc.create_node(3, NVE_COMP_TEXT);
    enc.create_node(4, NVE_COMP_TEXT);
    enc.create_node(5, NVE_COMP_IMAGE);
    enc.set_text(3, "Title");
    enc.set_text(4, "Static subtitle");
    enc.set_raw_prop_string(5, "src", "icon.png");
    nve::LayoutProps box_lp{};
    box_lp.width = {1, 300.0f};
    box_lp.height = {1, 120.0f};
    enc.set_layout(2, box_lp);
    enc.set_children(2, {3, 4, 5});
    enc.set_children(NVE_ROOT_NODE_ID, {2});
    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  }

  nve::IncrementalSceneReconciler<MockSceneViewHandle, MockIncrementalSceneAdapter> rec;
  MockIncrementalSceneAdapter adapter;
  assert(rec.sync(nve::fabric_host_scene_view(host), adapter));
  assert(rec.size() == 4u);
  assert(adapter.creates == 4);
  assert(adapter.updates == 4);
  assert(adapter.root_updates == 1);
  assert(adapter.root_children == (std::vector<uint32_t>{2u}));
  assert(rec.find(2u)->children == (std::vector<uint32_t>{3u, 4u, 5u}));
  assert(rec.find(5u)->text == "icon.png");

  // Frame 2: Update ONLY node 3 color (no layout shift!) -> untouched views 2, 4, 5 get 0 updates!
  const int updates_before = adapter.updates;
  const int creates_before = adapter.creates;
  const int sub_updates_before = rec.find(4u)->update_count;
  const int img_updates_before = rec.find(5u)->update_count;
  {
    CommandBufferEncoder enc(1, 2);
    nve::StyleProps st{};
    st.text_color_rgba = 0xFF00FF00u;
    st.font_size = 16.0f;
    enc.set_style(3, st);
    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  }
  assert(rec.sync(nve::fabric_host_scene_view(host), adapter));
  assert(adapter.creates == creates_before);
  assert(adapter.updates == updates_before + 1);
  assert(rec.find(3u)->text_rgba == 0xFF00FF00u);
  assert(rec.find(4u)->update_count == sub_updates_before);
  assert(rec.find(5u)->update_count == img_updates_before);

  // Frame 3: Convert leaf text 3 into wrapper text (child 6) -> tag changes -> destroy + recreate 3.
  const int destroys_before = adapter.destroys;
  {
    CommandBufferEncoder enc(1, 3);
    enc.create_node(6, NVE_COMP_TEXT);
    enc.set_text(6, "Nested span");
    enc.set_children(3, {6});
    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  }
  assert(rec.sync(nve::fabric_host_scene_view(host), adapter));
  assert(rec.size() == 5u);
  assert(adapter.destroys == destroys_before + 1);
  assert((rec.find(3u)->flags & NVE_NODE_LEAF_TEXT) == 0u);
  assert(rec.find(3u)->children == (std::vector<uint32_t>{6u}));
  assert(rec.find(6u)->text == "Nested span");

  // Frame 4: Remove image node 5 -> incremental removal destroys node 5 only.
  {
    CommandBufferEncoder enc(1, 4);
    enc.remove_node(5);
    enc.commit(true);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == 0);
  }
  assert(rec.sync(nve::fabric_host_scene_view(host), adapter));
  assert(rec.size() == 4u);
  assert(rec.find(5u) == nullptr);
  assert(rec.find(2u)->children == (std::vector<uint32_t>{3u, 4u}));

  // Frame reset (e.g. hot-reload back to frame 1): triggers full sync and prunes old nodes.
  {
    nve::NativeHostSession session;
    assert(session.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
    const NveSceneView sv = session.scene_view();
    assert(sv.schema_hash == NVE_SCENE_SCHEMA_HASH);
    assert(sv.frame_id == 1u);
    assert(sv.node_count == 3u); // root(1), pressable(2), text(3)
    const NveRenderNode* text_node = find_scene_node(sv, 3u);
    assert(text_node != nullptr && nve::scene_node_text(sv, *text_node) == "Count: 0");

    assert(rec.sync(sv, adapter));
    assert(rec.size() == 2u);
    assert(rec.find(2u) != nullptr);
    assert(rec.find(3u) != nullptr && rec.find(3u)->text == "Count: 0");
    assert(rec.find(4u) == nullptr);
    assert(rec.find(6u) == nullptr);

    // Press updates counter and NativeHostSession::scene_view() reflects frame 2 immediately.
    assert(session.dispatch_press(2u, 10.0f, 10.0f, true) == 1u);
    const NveSceneView sv2 = session.scene_view();
    assert(sv2.frame_id == 2u);
    assert(rec.sync(sv2, adapter));
    assert(rec.find(3u)->text == "Count: 1");
  }

  rec.clear(adapter);
  assert(rec.empty());
  nve_fabric_host_destroy(host);
}

} // namespace

void test_direct_scene_view_and_reconciler() {
  test_direct_scene_view_o_changed_and_image_src();
  test_incremental_scene_reconciler_and_session_scene_view();
}

} // namespace nve_test
