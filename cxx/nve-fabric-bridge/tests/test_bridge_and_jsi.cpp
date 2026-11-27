#include "test_common.hpp"

#include <cmath>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

namespace nve_test {

// 2. Test corrupt buffer error codes thoroughly.
void test_corrupt_buffer_error_codes() {
  NveSurfaceConfig cfg{.surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);

  assert(nve_fabric_host_submit_frame(nullptr, NveBufferSlice{nullptr, 0}, nullptr) == -100);

  const uint8_t short_buf[8] = {0};
  assert(nve_fabric_host_submit_frame(host, NveBufferSlice{short_buf, sizeof(short_buf)}, nullptr) == -1);
  assert(nve_fabric_host_submit_frame(host, NveBufferSlice{nullptr, 24}, nullptr) == -1);

  {
    CommandBufferEncoder enc(1, 1);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[0] = 0xDE;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -2);
  }
  {
    CommandBufferEncoder enc(1, 1);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[4] = 99;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -2);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.finish();
    uint32_t one = 1;
    std::memcpy(enc.raw().data() + 20, &one, 4);
    assert(nve_fabric_host_submit_frame(
               host, NveBufferSlice{enc.raw().data(), enc.raw().size()}, nullptr) == -3);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(2, NVE_COMP_VIEW);
    auto slice = enc.finish();
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{slice.ptr, slice.len - 2}, nullptr) == -4);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.set_text(999, "missing");
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -5);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.remove_node(NVE_ROOT_NODE_ID);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -5);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(10, NVE_COMP_VIEW);
    enc.set_raw_prop_null(10, "k");
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad.back() = 99;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -6);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.create_node(11, 0x1234);
    assert(nve_fabric_host_submit_frame(host, enc.finish(), nullptr) == -6);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.commit(false);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad[24] = 250;
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -7);
  }
  {
    CommandBufferEncoder enc(1, 1);
    enc.commit(false);
    auto slice = enc.finish();
    std::vector<uint8_t> bad(slice.ptr, slice.ptr + slice.len);
    bad.push_back(0x42);
    assert(nve_fabric_host_submit_frame(host, NveBufferSlice{bad.data(), bad.size()}, nullptr) == -8);
  }

  nve_fabric_host_destroy(host);
}

void test_rn_component_bridge() {
  std::vector<nve::DecodedEvent> emitted_via_cb;
  NveSurfaceConfig cfg{.surface_id = 42, .width = 320.0f, .height = 120.0f, .scale_factor = 3.0f};
  nve::NveRnComponentBridge bridge(
      &MockEmbeddedRustComponent::get_interface,
      cfg,
      [&](const nve::DecodedEvent& ev) { emitted_via_cb.push_back(ev); });

  assert(bridge.is_valid());
  auto measured = bridge.measure(NveSizeConstraints{0.0f, 0.0f, 320.0f, 120.0f});
  assert(measured.has_value() && approx_eq(measured->width, 200.0f) && approx_eq(measured->height, 60.0f));
  assert(bridge.hit_test(10.0f, 10.0f, 1) == NVE_GESTURE_CLAIM);

  // 1. Update props via typed PropMap -> BumpArena -> direct set_props -> auto-render
  nve::PropMap props;
  props["title"] = std::string("Hello \"RN\"");
  props["initialCount"] = static_cast<int64_t>(5);
  props["enabled"] = true;
  props["ratio"] = 0.75;
  props["optional"] = nullptr;

  assert(bridge.update_props(props, /*auto_render=*/true));
  assert(bridge.frame_count() == 1);
  assert(bridge.node_count() == 3);

  auto* mock_app = static_cast<MockEmbeddedRustComponent*>(bridge.app_handle());
  assert(mock_app->direct_props_calls == 1);
  assert(mock_app->last_direct_title == "Hello \"RN\"");
  assert(mock_app->last_direct_initial_count == 5);
  assert(mock_app->last_direct_enabled == true);
  assert(std::fabs(mock_app->last_direct_ratio - 0.75) < 1e-9);
  assert(mock_app->saw_direct_null);

  auto rect = bridge.node_layout(2);
  assert(rect.has_value());
  assert(approx_eq(rect->width, 200.0f) && approx_eq(rect->height, 60.0f));

  auto evs = bridge.drain_emitted_events();
  assert(evs.size() == 1);
  assert(evs[0].kind == nve::EventKind::Custom);
  assert(evs[0].name == "onPropsApplied");
  const auto ev0_view = nve::BinValueView::parse(evs[0].payload_bytes.data(), evs[0].payload_bytes.size());
  assert(ev0_view.has_value() && ev0_view->tag() == NVE_VAL_MAP);
  assert(ev0_view->map_get("accepted")->as_bool() == true);
  assert(emitted_via_cb.size() == 1);

  // 2. Direct press dispatch -> outbound onIncrement direct event
  const NveEvent press_ev = nve::make_press_event(42, 2, 99999, 15.0f, 25.0f);
  assert(bridge.dispatch_events(nve::make_single_event_batch(press_ev), /*auto_render=*/true) == 1);
  assert(bridge.frame_count() == 2);

  auto evs2 = bridge.drain_emitted_events();
  assert(evs2.size() == 1);
  assert(evs2[0].name == "onIncrement");
  const auto ev2_view = nve::BinValueView::parse(evs2[0].payload_bytes.data(), evs2[0].payload_bytes.size());
  assert(ev2_view.has_value() && ev2_view->map_get("count")->as_i64() == 1);
  assert(emitted_via_cb.size() == 2);

  // 2b. Direct custom event dispatch via NveRnComponentBridge::dispatch_events
  nve::EventBatchBuilder custom_batch;
  assert(custom_batch.push_custom_value(42, 2, 100500, "onDirectPinch", nve::make_f64_value(2.5)));
  assert(bridge.dispatch_events(custom_batch.as_batch(), /*auto_render=*/true) == 1);
  assert(mock_app->direct_event_calls == 2);
  assert(mock_app->last_direct_event_name == "onDirectPinch");
  assert(bridge.frame_count() == 3);

  // 3. Snapshot and restore state
  const auto snap = bridge.snapshot_state();
  assert(snap.size() == 1 && snap[0] == '1');
  const std::string next_state = "7";
  bridge.restore_state(
      NveBufferSlice{reinterpret_cast<const uint8_t*>(next_state.data()), next_state.size()},
      /*auto_render=*/true);
  assert(bridge.frame_count() == 4);
  const auto snap2 = bridge.snapshot_state();
  assert(snap2.size() == 1 && snap2[0] == '7');
}

#if defined(NVE_WITH_RN_JSI) && NVE_WITH_RN_JSI
void test_mock_jsi_recorder_runtime() {
  nve::MockJsiRecorderRuntime rt;

  struct RawBuffer final : public facebook::jsi::MutableBuffer {
    std::vector<uint8_t> bytes{0xCA, 0xFE, 0xBA, 0xBE};
    size_t size() const override { return bytes.size(); }
    uint8_t* data() override { return bytes.data(); }
  };

  {
    facebook::jsi::Object root(rt);
    root.setProperty(rt, "event", "onSvgLoad");
    root.setProperty(rt, "width", 128.5);
    root.setProperty(rt, "animated", true);
    root.setProperty(rt, "tag", nullptr);
    root.setProperty(rt, "big", facebook::jsi::BigInt::fromInt64(rt, -9007199254740993LL));
    root.setProperty(rt, "blob", facebook::jsi::ArrayBuffer(rt, std::make_shared<RawBuffer>()));

    facebook::jsi::Array pts(rt, 2);
    pts.setValueAtIndex(rt, 0, 10.0);
    pts.setValueAtIndex(rt, 1, 20.0);
    root.setProperty(rt, "points", pts);

    facebook::jsi::Object child(rt);
    root.setProperty(rt, "nested", child);
    child.setProperty(rt, "lateKey", "lateVal");

    assert(rt.record_direct_event(1, 77, 123456, "onSvgLoad", facebook::jsi::Value(rt, root)));

    bool sink_called = false;
    const uint32_t rc = rt.dispatch_direct([&](NveEventBatch batch) -> uint32_t {
      sink_called = true;
      assert(batch.event_count == 1);
      const NveEvent& ev = batch.events[0];
      assert(ev.surface == 1 && ev.node == 77 && ev.timestamp == 123456);
      assert(ev.table == NVE_TABLE_CUSTOM && (ev.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) != 0u);
      std::string_view cname;
      std::span<const uint8_t> cpayload;
      assert(nve::event_custom_parts(ev, batch.bytes, batch.bytes_len, cname, cpayload));
      assert(cname == "onSvgLoad");
      const auto view = nve::BinValueView::parse(cpayload.data(), cpayload.size());
      assert(view.has_value() && view->tag() == NVE_VAL_MAP);
      assert(view->map_get("event")->as_str() == "onSvgLoad");
      assert(std::fabs(*view->map_get("width")->as_f64() - 128.5) < 1e-9);
      assert(view->map_get("animated")->as_bool() == true);
      assert(view->map_get("tag")->is_null());
      assert(view->map_get("big")->as_i64() == -9007199254740993LL);
      const auto blob = view->map_get("blob")->as_bytes();
      assert(blob.has_value() && blob->len == 4 && blob->ptr[0] == 0xCA && blob->ptr[3] == 0xBE);
      const auto pts = view->map_get("points");
      assert(pts.has_value() && pts->array_len() == 2);
      assert(std::fabs(*pts->array_at(0)->as_f64() - 10.0) < 1e-9);
      assert(std::fabs(*pts->array_at(1)->as_f64() - 20.0) < 1e-9);
      assert(view->map_get("nested")->map_get("lateKey")->as_str() == "lateVal");
      return 1u;
    });
    assert(sink_called && rc == 1u);

    nve::EventRingWriter ring;
    ring.clear();
    assert(rt.record_custom_event(ring, 1, 77, 123456, "onSvgLoad", facebook::jsi::Value(rt, root)));

    const auto& bytes = ring.finalize();
    std::vector<nve::DecodedEvent> decoded;
    assert(nve::decode_event_ring(NveBufferSlice{bytes.data(), bytes.size()}, decoded));
    assert(decoded.size() == 1);
    assert(decoded[0].kind == nve::EventKind::Custom);
    assert(decoded[0].node_id == 77);
    assert(decoded[0].name == "onSvgLoad");

    const auto view = nve::BinValueView::parse(
        decoded[0].payload_bytes.data(), decoded[0].payload_bytes.size());
    assert(view.has_value() && view->tag() == NVE_VAL_MAP);
    const auto ev_str = view->map_get("event")->as_str();
    assert(ev_str.has_value() && *ev_str == "onSvgLoad");
    const auto* p_start = reinterpret_cast<const char*>(decoded[0].payload_bytes.data());
    const auto* p_end = p_start + decoded[0].payload_bytes.size();
    assert(ev_str->data() >= p_start && ev_str->data() < p_end);
    assert(std::fabs(*view->map_get("width")->as_f64() - 128.5) < 1e-9);
    assert(view->map_get("animated")->as_bool() == true);
    assert(view->map_get("tag")->is_null());
    assert(view->map_get("big")->as_i64() == -9007199254740993LL);
    assert(view->map_get("blob")->as_bytes()->len == 4);
    assert(view->map_get("points")->array_len() == 2);
    assert(std::fabs(*view->map_get("points")->array_at(0)->as_f64() - 10.0) < 1e-9);
    assert(std::fabs(*view->map_get("points")->array_at(1)->as_f64() - 20.0) < 1e-9);
    assert(view->map_get("nested")->map_get("lateKey")->as_str() == "lateVal");
  }

  rt.reset_frame();
  {
    facebook::jsi::Object cyc(rt);
    cyc.setProperty(rt, "self", cyc);
    const NveValue captured = rt.capture_value(facebook::jsi::Value(rt, cyc));
    assert(captured.tag == NVE_VAL_MAP);
    std::vector<uint8_t> bin;
    assert(!nve::encode_bin_value(bin, captured));
    assert(bin.empty());
  }

  rt.reset_frame();
  size_t slabs_after_warmup = 0;
  for (int frame = 0; frame < 1200; ++frame) {
    rt.reset_frame();
    for (int k = 0; k < 8; ++k) {
      facebook::jsi::Object ev(rt);
      ev.setProperty(rt, "event", "onGestureUpdate");
      ev.setProperty(rt, "x", static_cast<double>(frame + k));
      ev.setProperty(rt, "y", 42.5);
      ev.setProperty(rt, "active", (k & 1) == 0);
      facebook::jsi::Array v(rt, 2);
      v.setValueAtIndex(rt, 0, 1.5);
      v.setValueAtIndex(rt, 1, -2.5);
      ev.setProperty(rt, "velocity", std::move(v));
      assert(rt.record_direct_event(1, static_cast<uint32_t>(10 + k), static_cast<uint64_t>(frame), "onGestureUpdate", facebook::jsi::Value(rt, ev)));
    }
    assert(rt.direct_event_batch().event_count == 8);
    if (frame == 0) {
      slabs_after_warmup = rt.arena().slab_count();
    } else {
      assert(rt.arena().slab_count() == slabs_after_warmup);
    }
  }
}
#endif

} // namespace nve_test
