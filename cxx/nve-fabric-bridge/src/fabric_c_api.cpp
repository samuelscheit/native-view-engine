#include "fabric_internal.hpp"

namespace nve {

void FabricHost::register_custom_component(CustomComponentDescriptor descriptor) {
  if (descriptor.component_name.empty()) return;
  custom_components_[descriptor.component_name] = std::move(descriptor);
}

void FabricHost::enqueue_press(uint32_t node_id, uint64_t ts_us, float x, float y) {
  std::lock_guard<std::mutex> lock(event_mutex_);
  event_ring_.push_press(config_.surface_id, node_id, ts_us, x, y);
}

void FabricHost::enqueue_text_change(uint32_t node_id, uint64_t ts_us, std::string_view text) {
  std::lock_guard<std::mutex> lock(event_mutex_);
  event_ring_.push_text_change(config_.surface_id, node_id, ts_us, text);
}

void FabricHost::enqueue_custom_event(
    uint32_t node_id,
    uint64_t ts_us,
    std::string_view name,
    const uint8_t* data,
    size_t len) {
  std::lock_guard<std::mutex> lock(event_mutex_);
  event_ring_.push_custom(config_.surface_id, node_id, ts_us, name, data, len);
}

NveBufferSlice FabricHost::drain_events() {
  std::lock_guard<std::mutex> lock(event_mutex_);
  drained_snapshot_ = event_ring_.finalize();
  event_ring_.clear();
  return NveBufferSlice{drained_snapshot_.data(), drained_snapshot_.size()};
}

bool FabricHost::get_node_layout(uint32_t node_id, LayoutRect* out_rect) const {
  if (!out_rect) return false;
  const ShadowNodeEntry* entry = nodes_.find(node_id);
  if (entry == nullptr || !entry->has_computed_layout) {
    return false;
  }
  *out_rect = entry->computed_layout;
  return true;
}

std::vector<RenderedNodeSnapshot> FabricHost::snapshot_nodes() const {
  std::vector<RenderedNodeSnapshot> out;
  out.reserve(nodes_.size());
  for (const auto& entry : nodes_.entries()) {
    RenderedNodeSnapshot snap{};
    snap.node_id = entry.node_id;
    snap.kind = entry.kind;
    snap.custom_name = entry.custom_name;
    snap.text = entry.text;
    snap.layout = entry.computed_layout;
    snap.background_rgba = entry.style.background_rgba;
    snap.text_color_rgba = entry.style.text_color_rgba;
    snap.border_color_rgba = entry.style.border_color_rgba;
    snap.border_width = entry.style.border_width;
    snap.border_radius = entry.style.border_radius;
    snap.opacity = entry.style.opacity;
    snap.font_size = entry.style.font_size;
    snap.z_index = entry.style.z_index;
    snap.event_mask = entry.event_mask;
    snap.children = entry.children;
    if (entry.raw_props && !entry.raw_props->empty()) {
      snap.raw_props = *entry.raw_props;
    }
    out.push_back(std::move(snap));
  }
  return out;
}

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
void FabricHost::attach_rn_uimanager(std::shared_ptr<facebook::react::UIManager> ui_manager) {
  detach_rn_uimanager();
  rn_ui_manager_ = std::move(ui_manager);
  if (!rn_ui_manager_) return;

  rn_event_listener_ = std::make_shared<const facebook::react::EventListener>(
      [this](const facebook::react::RawEvent& raw_event) -> bool {
        return this->intercept_rn_raw_event(raw_event);
      });
  rn_ui_manager_->addEventListener(rn_event_listener_);
}

void FabricHost::detach_rn_uimanager() {
  if (rn_ui_manager_ && rn_event_listener_) {
    rn_ui_manager_->removeEventListener(rn_event_listener_);
  }
  rn_event_listener_.reset();
  rn_ui_manager_.reset();
}

bool FabricHost::intercept_rn_raw_event(const facebook::react::RawEvent& raw_event) {
  auto family = raw_event.shadowNodeFamily.lock();
  if (!family) return false;
  const auto surface_id = static_cast<uint32_t>(family->getSurfaceId());
  if (surface_id != config_.surface_id) return false;

  const auto node_id = static_cast<uint32_t>(family->getTag());
  const uint64_t ts = now_us();

  std::lock_guard<std::mutex> lock(event_mutex_);
  if (!raw_event.eventPayload) {
    event_ring_.push_custom(surface_id, node_id, ts, raw_event.type, nullptr, 0);
    return true;
  }

  switch (raw_event.eventPayload->getType()) {
    case facebook::react::EventPayloadType::PointerEvent: {
      const float x = static_cast<float>(
          raw_event.eventPayload->extractValue({"locationX"}).value_or(
              raw_event.eventPayload->extractValue({"clientX"}).value_or(0.0)));
      const float y = static_cast<float>(
          raw_event.eventPayload->extractValue({"locationY"}).value_or(
              raw_event.eventPayload->extractValue({"clientY"}).value_or(0.0)));
      event_ring_.push_press(surface_id, node_id, ts, x, y);
      return true;
    }
    case facebook::react::EventPayloadType::ScrollEvent: {
      const float ox = static_cast<float>(
          raw_event.eventPayload->extractValue({"contentOffset", "x"}).value_or(0.0));
      const float oy = static_cast<float>(
          raw_event.eventPayload->extractValue({"contentOffset", "y"}).value_or(0.0));
      const float cw = static_cast<float>(
          raw_event.eventPayload->extractValue({"contentSize", "width"}).value_or(0.0));
      const float ch = static_cast<float>(
          raw_event.eventPayload->extractValue({"contentSize", "height"}).value_or(0.0));
      event_ring_.push_scroll(surface_id, node_id, ts, ox, oy, cw, ch);
      return true;
    }
    case facebook::react::EventPayloadType::ValueFactory:
    default: {
      facebook::jsi::Value val = raw_event.eventPayload->asJSIValue(jsi_recorder_);
      jsi_recorder_.record_custom_event(event_ring_, surface_id, node_id, ts, raw_event.type, val);
      return true;
    }
  }
}

void FabricHost::commit_rn_shadow_tree(bool sync_mount) {
  if (!rn_ui_manager_) return;
  const auto surface_id = static_cast<facebook::react::SurfaceId>(config_.surface_id);
  rn_ui_manager_->getShadowTreeRegistry().visit(
      surface_id,
      [sync_mount](const facebook::react::ShadowTree& shadow_tree) {
        facebook::react::ShadowTree::CommitOptions options{};
        options.enableStateReconciliation = false;
        options.mountSynchronously = sync_mount;
        shadow_tree.commit(
            [](const facebook::react::RootShadowNode& old_root)
                -> facebook::react::RootShadowNode::Unshared {
              return std::static_pointer_cast<facebook::react::RootShadowNode>(old_root.clone({}));
            },
            options);
      });
}
#endif

bool fabric_host_get_node_layout(NveFabricHostHandle host, uint32_t node_id, LayoutRect* out_rect) {
  if (!host || !out_rect) return false;
  return static_cast<FabricHost*>(host)->get_node_layout(node_id, out_rect);
}

bool fabric_host_has_yoga_layout() noexcept {
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  return true;
#else
  return false;
#endif
}

bool fabric_host_set_surface_size(
    NveFabricHostHandle host,
    float width,
    float height,
    float scale_factor) {
  if (!host) return false;
  static_cast<FabricHost*>(host)->set_surface_size(width, height, scale_factor);
  return true;
}

void fabric_host_enqueue_press(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    float x,
    float y) {
  if (!host) return;
  static_cast<FabricHost*>(host)->enqueue_press(node_id, timestamp_us, x, y);
}

void fabric_host_enqueue_text_change(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    std::string_view text) {
  if (!host) return;
  static_cast<FabricHost*>(host)->enqueue_text_change(node_id, timestamp_us, text);
}

void fabric_host_enqueue_custom_event(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    std::string_view name,
    const uint8_t* data,
    size_t len) {
  if (!host) return;
  static_cast<FabricHost*>(host)->enqueue_custom_event(node_id, timestamp_us, name, data, len);
}

void fabric_host_set_text_measurer(
    NveFabricHostHandle host,
    TextMeasurerCallback measurer) {
  if (!host) return;
  static_cast<FabricHost*>(host)->set_text_measurer(std::move(measurer));
}

void fabric_host_register_custom_component(
    NveFabricHostHandle host,
    CustomComponentDescriptor descriptor) {
  if (!host) return;
  static_cast<FabricHost*>(host)->register_custom_component(std::move(descriptor));
}

std::vector<RenderedNodeSnapshot> fabric_host_snapshot_nodes(NveFabricHostHandle host) {
  if (!host) return {};
  return static_cast<FabricHost*>(host)->snapshot_nodes();
}

std::vector<uint8_t> fabric_host_serialize_frame0(NveFabricHostHandle host, uint64_t frame_id) {
  if (!host) return {};
  return static_cast<FabricHost*>(host)->serialize_frame0(frame_id);
}

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
void fabric_host_attach_rn_uimanager(
    NveFabricHostHandle host,
    std::shared_ptr<facebook::react::UIManager> ui_manager) {
  if (!host) return;
  static_cast<FabricHost*>(host)->attach_rn_uimanager(std::move(ui_manager));
}
#endif

} // namespace nve

extern "C" {

NveFabricHostHandle nve_fabric_host_create(NveSurfaceConfig config) {
  return new nve::FabricHost(config);
}

void nve_fabric_host_destroy(NveFabricHostHandle host) {
  delete static_cast<nve::FabricHost*>(host);
}

int32_t nve_fabric_host_submit_frame(
    NveFabricHostHandle host,
    NveBufferSlice command_buffer,
    NveFrameStats* out_stats) {
  if (!host) return -100;
  return static_cast<nve::FabricHost*>(host)->submit_frame(command_buffer, out_stats);
}

NveBufferSlice nve_fabric_host_drain_events(NveFabricHostHandle host) {
  if (!host) return NveBufferSlice{nullptr, 0};
  return static_cast<nve::FabricHost*>(host)->drain_events();
}

uint32_t nve_fabric_host_node_count(NveFabricHostHandle host) {
  if (!host) return 0;
  return static_cast<nve::FabricHost*>(host)->node_count();
}

} // extern "C"
