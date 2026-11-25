#include "test_common.hpp"

namespace nve_test {

// 9. Test nve_host_runtime.hpp utilities: unpack_rgba, current_timestamp_us, view_tag_for_snapshot,
//    and load_nve_dylib error codes.
void test_host_runtime_utilities_and_dylib_errors() {
  const uint64_t t1 = nve::current_timestamp_us();
  const uint64_t t2 = nve::current_timestamp_us();
  assert(t2 >= t1);

  // RGBA unpacking (little-endian: R low byte, A high byte)
  {
    const nve::RgbaColor clear = nve::unpack_rgba(0x00000000u);
    assert(clear.r8 == 0 && clear.g8 == 0 && clear.b8 == 0 && clear.a8 == 0);
    assert(approx_eq(clear.r, 0.0f, 1e-6f) && approx_eq(clear.a, 0.0f, 1e-6f));

    const nve::RgbaColor black = nve::unpack_rgba(0xFF000000u);
    assert(black.r8 == 0 && black.g8 == 0 && black.b8 == 0 && black.a8 == 255);
    assert(approx_eq(black.r, 0.0f, 1e-6f) && approx_eq(black.a, 1.0f, 1e-6f));

    const nve::RgbaColor white = nve::unpack_rgba(0xFFFFFFFFu);
    assert(white.r8 == 255 && white.g8 == 255 && white.b8 == 255 && white.a8 == 255);
    assert(approx_eq(white.r, 1.0f, 1e-6f) && approx_eq(white.g, 1.0f, 1e-6f) &&
           approx_eq(white.b, 1.0f, 1e-6f) && approx_eq(white.a, 1.0f, 1e-6f));

    const nve::RgbaColor sample = nve::unpack_rgba(0x80336699u);
    assert(sample.r8 == 0x99u && sample.g8 == 0x66u && sample.b8 == 0x33u && sample.a8 == 0x80u);
    assert(approx_eq(sample.r, 0x99 / 255.0f, 1e-5f));
    assert(approx_eq(sample.g, 0x66 / 255.0f, 1e-5f));
    assert(approx_eq(sample.b, 0x33 / 255.0f, 1e-5f));
    assert(approx_eq(sample.a, 0x80 / 255.0f, 1e-5f));
  }

  // View tags distinguish leaf text vs wrapper text
  {
    nve::RenderedNodeSnapshot leaf_text{};
    leaf_text.kind = NVE_COMP_TEXT;
    nve::RenderedNodeSnapshot wrapper_text{};
    wrapper_text.kind = NVE_COMP_TEXT;
    wrapper_text.children = {10};
    assert(nve::view_tag_for_snapshot(leaf_text) != nve::view_tag_for_snapshot(wrapper_text));

    nve::RenderedNodeSnapshot view_empty{};
    view_empty.kind = NVE_COMP_VIEW;
    nve::RenderedNodeSnapshot view_with_kids{};
    view_with_kids.kind = NVE_COMP_VIEW;
    view_with_kids.children = {10};
    assert(nve::view_tag_for_snapshot(view_empty) == nve::view_tag_for_snapshot(view_with_kids));
  }

  // load_nve_dylib error codes: empty path -> 1, missing file -> 1
  {
    nve::LoadedModule empty_res = nve::load_nve_dylib("");
    assert(!empty_res.ok());
    assert(empty_res.error_code == 1);
    assert(!empty_res.error_message.empty());
    assert(!empty_res.lib.is_open());

    nve::LoadedModule missing_res = nve::load_nve_dylib("/nonexistent/path/nve_missing_module.so");
    assert(!missing_res.ok());
    assert(missing_res.error_code == 1);
    assert(!missing_res.error_message.empty());
    assert(!missing_res.lib.is_open());
  }

#ifdef NVE_BRIDGE_FIXTURE_NO_ENTRY
  {
    nve::LoadedModule no_entry = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_NO_ENTRY);
    assert(!no_entry.ok());
    assert(no_entry.error_code == 2);
    assert(!no_entry.error_message.empty());
    assert(!no_entry.lib.is_open());
  }
#endif

#ifdef NVE_BRIDGE_FIXTURE_NO_SURFACE
  {
    nve::LoadedModule no_surface = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_NO_SURFACE);
    assert(!no_surface.ok());
    assert(no_surface.error_code == 3);
    assert(!no_surface.error_message.empty());
    assert(!no_surface.lib.is_open());
  }
#endif

#ifdef NVE_BRIDGE_FIXTURE_VALID
  {
    nve::LoadedModule valid = nve::load_nve_dylib(NVE_BRIDGE_FIXTURE_VALID);
    assert(valid.ok());
    assert(valid.error_code == 0);
    assert(valid.lib.is_open());
    assert(valid.get_interface != nullptr);
    assert(valid.module.surface != nullptr);
  }
#endif
}

struct MockReconciledView {
  uint32_t node_id{0};
  uint32_t instance_id{0};
  bool is_text_wrapper{false};
  std::string text;
  int update_count{0};
  std::vector<uint32_t> mounted_children;
};

struct MockReconcilerAdapter {
  uint32_t next_instance_id{1};
  int root_updates{0};
  uint32_t last_root_bg{0};
  int creates{0};
  std::vector<uint32_t> destroyed_nodes;
  std::vector<uint32_t> root_children;

  void update_root(const nve::RenderedNodeSnapshot& snap) {
    ++root_updates;
    last_root_bg = snap.background_rgba;
  }

  std::unique_ptr<MockReconciledView> create_view(const nve::RenderedNodeSnapshot& snap) {
    ++creates;
    auto view = std::make_unique<MockReconciledView>();
    view->node_id = snap.node_id;
    view->instance_id = next_instance_id++;
    view->is_text_wrapper = (snap.kind == NVE_COMP_TEXT && !snap.children.empty());
    return view;
  }

  void update_view(
      std::unique_ptr<MockReconciledView>& view,
      const nve::RenderedNodeSnapshot& snap) {
    assert(view != nullptr);
    view->text = snap.text;
    view->update_count += 1;
  }

  void destroy_view(uint32_t node_id, std::unique_ptr<MockReconciledView>& view) {
    assert(view != nullptr);
    assert(view->node_id == node_id);
    destroyed_nodes.push_back(node_id);
    view.reset();
  }

  template <typename LookupFn>
  void sync_children(
      uint32_t parent_id,
      std::unique_ptr<MockReconciledView>* parent_view,
      const std::vector<uint32_t>& child_ids,
      LookupFn&& lookup_fn) {
    std::vector<uint32_t> resolved;
    for (uint32_t cid : child_ids) {
      auto* child_ptr = lookup_fn(cid);
      if (child_ptr != nullptr && *child_ptr != nullptr) {
        resolved.push_back((*child_ptr)->node_id);
      }
    }
    assert(lookup_fn(999999u) == nullptr);
    if (parent_id == NVE_ROOT_NODE_ID) {
      assert(parent_view == nullptr);
      root_children = std::move(resolved);
    } else {
      assert(parent_view != nullptr && *parent_view != nullptr);
      (*parent_view)->mounted_children = std::move(resolved);
    }
  }
};

// 10. Test nve::RetainedViewReconciler across mount, update, text leaf<->wrapper tag swap,
//     child reordering, pruning, and clear.
void test_retained_view_reconciler() {
  nve::RetainedViewReconciler<std::unique_ptr<MockReconciledView>, MockReconcilerAdapter>
      reconciler;
  MockReconcilerAdapter adapter;

  auto make_snap = [](uint32_t id,
                      uint16_t kind,
                      std::string text = {},
                      std::vector<uint32_t> children = {},
                      uint32_t bg = 0) {
    nve::RenderedNodeSnapshot s{};
    s.node_id = id;
    s.kind = kind;
    s.text = std::move(text);
    s.children = std::move(children);
    s.background_rgba = bg;
    return s;
  };

  // Frame 1: initial mount (root -> [2(VIEW)], 2 -> [3(leaf TEXT), 4(PRESSABLE)])
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}, 0xFF102030u),
          make_snap(2, NVE_COMP_VIEW, "", {3, 4}),
          make_snap(3, NVE_COMP_TEXT, "Leaf"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(reconciler.size() == 3);
  assert(adapter.root_updates == 1);
  assert(adapter.last_root_bg == 0xFF102030u);
  assert(adapter.creates == 3);
  assert(adapter.destroyed_nodes.empty());
  assert(adapter.root_children == std::vector<uint32_t>({2}));
  assert(reconciler.find(2) != nullptr);
  assert((*reconciler.find(2))->mounted_children == std::vector<uint32_t>({3, 4}));
  assert(!(*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->text == "Leaf");
  const uint32_t leaf_text_inst = (*reconciler.find(3))->instance_id;
  const uint32_t view2_inst = (*reconciler.find(2))->instance_id;

  // Frame 2: incremental update + child reordering (no view recreation!)
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}, 0xFF102030u),
          make_snap(2, NVE_COMP_VIEW, "", {4, 3}),
          make_snap(3, NVE_COMP_TEXT, "Leaf updated"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(adapter.creates == 3);
  assert(adapter.destroyed_nodes.empty());
  assert((*reconciler.find(2))->instance_id == view2_inst);
  assert((*reconciler.find(3))->instance_id == leaf_text_inst);
  assert((*reconciler.find(3))->text == "Leaf updated");
  assert((*reconciler.find(3))->update_count == 2);
  assert((*reconciler.find(2))->mounted_children == std::vector<uint32_t>({4, 3}));

  // Frame 3: text leaf -> wrapper tag swap (node 3 gains child 5) => destroys & recreates node 3
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}),
          make_snap(2, NVE_COMP_VIEW, "", {4, 3}),
          make_snap(3, NVE_COMP_TEXT, "", {5}),
          make_snap(5, NVE_COMP_TEXT, "Nested span"),
          make_snap(4, NVE_COMP_PRESSABLE, ""),
      },
      adapter);
  assert(reconciler.size() == 4);
  assert(adapter.creates == 5); /* +1 recreated node 3, +1 new node 5 */
  assert(adapter.destroyed_nodes == std::vector<uint32_t>({3}));
  assert((*reconciler.find(3))->instance_id != leaf_text_inst);
  assert((*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->mounted_children == std::vector<uint32_t>({5}));
  const uint32_t wrapper_text_inst = (*reconciler.find(3))->instance_id;

  // Frame 4: wrapper -> leaf tag swap + pruning node 4 and node 5
  reconciler.sync(
      {
          make_snap(NVE_ROOT_NODE_ID, NVE_COMP_ROOT, "", {2}),
          make_snap(2, NVE_COMP_VIEW, "", {3}),
          make_snap(3, NVE_COMP_TEXT, "Back to leaf"),
      },
      adapter);
  assert(reconciler.size() == 2);
  assert(reconciler.find(4) == nullptr);
  assert(reconciler.find(5) == nullptr);
  assert((*reconciler.find(3))->instance_id != wrapper_text_inst);
  assert(!(*reconciler.find(3))->is_text_wrapper);
  assert((*reconciler.find(3))->text == "Back to leaf");

  // Clear destroys remaining views
  reconciler.clear(adapter);
  assert(reconciler.size() == 0);
  assert(reconciler.empty());
  assert(reconciler.find(2) == nullptr);
  assert(reconciler.find(3) == nullptr);
}

// 11. Test nve::NativeHostSession (attach, custom text measurer re-applied across reload,
//     press dispatch, props update, atomic failed reload, and real dylib hot-reload).
void test_native_host_session() {
  NveSurfaceConfig cfg{.surface_id = 55, .width = 360.0f, .height = 240.0f, .scale_factor = 2.0f};
  nve::NativeHostSession session;
  assert(!session.is_valid());
  assert(session.bridge() == nullptr);
  assert(session.render() == -100);
  assert(session.dispatch_press(2, 5.0f, 5.0f) == 0);
  assert(session.drain_events().empty());
  assert(session.snapshot_nodes().empty());

  int measurer_calls = 0;
  session.set_text_measurer(
      [&](std::string_view text, float, float) -> NveIntrinsicSize {
        ++measurer_calls;
        return NveIntrinsicSize{static_cast<float>(text.size()) * 11.0f, 44.0f};
      });

  std::vector<nve::DecodedEvent> callback_events;
  session.set_event_callback(
      [&](const nve::DecodedEvent& ev) { callback_events.push_back(ev); });

  // Attach in-process interface (automatically installs text measurer & renders frame 1)
  assert(session.attach_interface(&MockEmbeddedRustComponent::get_interface, cfg));
  assert(session.is_valid());
  assert(session.bridge() != nullptr);
  assert(session.snapshot_nodes().size() == 3);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  assert(measurer_calls >= 1);
  auto text_rect = session.bridge()->node_layout(3);
  assert(text_rect.has_value() && approx_eq(text_rect->height, 44.0f, 1.0f));
#endif

  // Update props via JSON -> emits onPropsApplied
  assert(session.update_props_json("{\"title\":\"HostSession\"}"));
  auto props_evs = session.drain_events();
  assert(props_evs.size() == 1);
  assert(props_evs[0].name == "onPropsApplied");
  assert(callback_events.size() == 1);

  // Dispatch press -> increments counter to 1 and emits binary onIncrement
  assert(session.dispatch_press(2, 12.0f, 18.0f) == 1);
  auto press_evs = session.drain_events();
  assert(press_evs.size() == 1);
  assert(press_evs[0].name == "onIncrement");
  const auto press_view = nve::BinValueView::parse(
      press_evs[0].payload_bytes.data(), press_evs[0].payload_bytes.size());
  assert(press_view.has_value() && press_view->map_get("count")->as_i64() == 1);
  assert(callback_events.size() == 2);

  // Resize surface
  session.set_surface_size(480.0f, 320.0f, 2.0f);
  assert(approx_eq(session.surface_config().width, 480.0f));
  assert(approx_eq(session.surface_config().height, 320.0f));

  // Failed reload_dylib must be atomic: returns false with code 1 and preserves active session/state
  int err_code = 0;
  std::string err_msg;
  assert(!session.reload_dylib("/nonexistent/bad_reload.dylib", &err_code, &err_msg));
  assert(err_code == 1);
  assert(!err_msg.empty());
  assert(session.is_valid());
  const auto preserved_state = session.bridge()->snapshot_state();
  assert(preserved_state.size() == 1 && preserved_state[0] == '1');

#ifdef NVE_BRIDGE_FIXTURE_VALID
  // Full real-dylib load_dylib + press + reload_dylib verifying state preservation AND
  // custom text measurer re-application across reload!
  {
    nve::NativeHostSession dylib_session;
    int dylib_measurer_calls = 0;
    dylib_session.set_text_measurer(
        [&](std::string_view text, float, float) -> NveIntrinsicSize {
          ++dylib_measurer_calls;
          return NveIntrinsicSize{static_cast<float>(text.size()) * 10.0f, 52.0f};
        });

    int load_err = -1;
    std::string load_msg;
    assert(dylib_session.load_dylib(NVE_BRIDGE_FIXTURE_VALID, cfg, &load_err, &load_msg));
    assert(load_err == 0 && load_msg.empty());
    assert(dylib_session.is_valid());
    assert(dylib_session.dispatch_press(2, 10.0f, 10.0f) == 1);
    assert(dylib_session.dispatch_press(2, 10.0f, 10.0f) == 1);
    assert(dylib_session.update_props_json("{\"v\":1}"));

    const int calls_before_reload = dylib_measurer_calls;
    int reload_err = -1;
    std::string reload_msg;
    assert(dylib_session.reload_dylib(NVE_BRIDGE_FIXTURE_VALID, &reload_err, &reload_msg));
    assert(reload_err == 0 && reload_msg.empty());
    assert(dylib_session.is_valid());

    // State ("2") must survive reload_dylib, and text node 3 reflects "Fixture:2"
    const auto reloaded_snaps = dylib_session.snapshot_nodes();
    bool saw_preserved_text = false;
    for (const auto& s : reloaded_snaps) {
      if (s.node_id == 3 && s.text == "Fixture:2") {
        saw_preserved_text = true;
      }
    }
    assert(saw_preserved_text);

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    // Custom text measurer must have been re-applied on the reloaded bridge
    assert(dylib_measurer_calls > calls_before_reload);
    auto reloaded_rect = dylib_session.bridge()->node_layout(3);
    assert(reloaded_rect.has_value() && approx_eq(reloaded_rect->height, 52.0f, 1.0f));
#else
    (void)calls_before_reload;
#endif
  }
#endif
}

} // namespace nve_test
