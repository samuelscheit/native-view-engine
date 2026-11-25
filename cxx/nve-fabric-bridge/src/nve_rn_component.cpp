#include "nve_rn_component.hpp"
#include "command_decoder.hpp"

#include <cmath>
#include <cstdio>
#include <utility>

namespace nve {

namespace {

const char* canonical_event_name(EventKind kind) noexcept {
  switch (kind) {
    case EventKind::Press: return "onPress";
    case EventKind::LongPress: return "onLongPress";
    case EventKind::Scroll: return "onScroll";
    case EventKind::TextChange: return "onChangeText";
    case EventKind::Focus: return "onFocus";
    case EventKind::Blur: return "onBlur";
    case EventKind::Layout: return "onLayout";
    case EventKind::Custom: return "onCustom";
  }
  return "onUnknown";
}

void append_json_string(std::string& out, std::string_view s) {
  out.push_back('"');
  for (unsigned char c : s) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\b': out += "\\b"; break;
      case '\f': out += "\\f"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20) {
          char hex[8];
          std::snprintf(hex, sizeof(hex), "\\u%04x", static_cast<unsigned>(c));
          out += hex;
        } else {
          out.push_back(static_cast<char>(c));
        }
        break;
    }
  }
  out.push_back('"');
}

} // namespace

bool decode_event_ring(NveBufferSlice slice, std::vector<DecodedEvent>& out_events) {
  out_events.clear();
  if (slice.len == 0) {
    return true;
  }
  if (slice.ptr == nullptr) {
    return false;
  }

  BinaryReader r(slice.ptr, slice.len);
  uint32_t count = 0;
  if (!r.read(count)) {
    return false;
  }

  out_events.reserve(count < 4096 ? count : 4096);
  for (uint32_t i = 0; i < count; ++i) {
    DecodedEvent ev{};
    uint8_t raw_kind = 0;
    uint8_t payload_tag = 0;
    if (!r.read(ev.surface_id) || !r.read(ev.node_id) || !r.read(ev.timestamp_us) ||
        !r.read(raw_kind) || !r.read(payload_tag)) {
      out_events.clear();
      return false;
    }
    if (raw_kind < 1 || raw_kind > 8) {
      out_events.clear();
      return false;
    }
    ev.kind = static_cast<EventKind>(raw_kind);
    ev.name = canonical_event_name(ev.kind);

    switch (payload_tag) {
      case 0: /* Empty */
        break;
      case 1: /* Point */
        if (!r.read(ev.x) || !r.read(ev.y)) {
          out_events.clear();
          return false;
        }
        break;
      case 2: /* Scroll */
        if (!r.read(ev.x) || !r.read(ev.y) || !r.read(ev.width) || !r.read(ev.height)) {
          out_events.clear();
          return false;
        }
        break;
      case 3: { /* Text */
        uint32_t len = 0;
        std::string_view sv;
        if (!r.read(len) || !r.read_string(len, sv)) {
          out_events.clear();
          return false;
        }
        ev.text.assign(sv.begin(), sv.end());
        ev.payload_bytes.assign(
            reinterpret_cast<const uint8_t*>(sv.data()),
            reinterpret_cast<const uint8_t*>(sv.data()) + sv.size());
        break;
      }
      case 4: /* Layout */
        if (!r.read(ev.x) || !r.read(ev.y) || !r.read(ev.width) || !r.read(ev.height)) {
          out_events.clear();
          return false;
        }
        break;
      case 5: { /* Custom */
        uint16_t nlen = 0;
        std::string_view name;
        uint32_t dlen = 0;
        if (!r.read(nlen) || !r.read_string(nlen, name) || !r.read(dlen) ||
            !r.read_bytes(dlen, ev.payload_bytes)) {
          out_events.clear();
          return false;
        }
        ev.name.assign(name.begin(), name.end());
        break;
      }
      default:
        out_events.clear();
        return false;
    }
    out_events.push_back(std::move(ev));
  }
  if (r.remaining() != 0) {
    out_events.clear();
    return false;
  }
  return true;
}

std::string serialize_props_json(const PropMap& props) {
  std::string out;
  out.reserve(64 + props.size() * 24);
  out.push_back('{');
  bool first = true;
  for (const auto& [key, val] : props) {
    if (!first) out.push_back(',');
    first = false;
    append_json_string(out, key);
    out.push_back(':');
    std::visit(
        [&out](const auto& v) {
          using T = std::decay_t<decltype(v)>;
          if constexpr (std::is_same_v<T, std::nullptr_t>) {
            out += "null";
          } else if constexpr (std::is_same_v<T, bool>) {
            out += (v ? "true" : "false");
          } else if constexpr (std::is_same_v<T, int64_t>) {
            out += std::to_string(v);
          } else if constexpr (std::is_same_v<T, double>) {
            if (!std::isfinite(v)) {
              out += "null";
            } else {
              char buf[64];
              std::snprintf(buf, sizeof(buf), "%.17g", v);
              out += buf;
            }
          } else if constexpr (std::is_same_v<T, std::string>) {
            append_json_string(out, v);
          }
        },
        val);
  }
  out.push_back('}');
  return out;
}

NveRnComponentBridge::NveRnComponentBridge(
    NveModule module,
    NveSurfaceConfig config,
    EventEmitterCallback on_event)
    : module_(module), config_(config), on_event_(std::move(on_event)) {
  if (module_.surface == nullptr || module_.surface->create == nullptr) {
    return;
  }
  fabric_host_ = nve_fabric_host_create(config_);
  app_handle_ = module_.surface->create(config_.surface_id);
  if (!app_handle_) {
    nve_fabric_host_destroy(fabric_host_);
    fabric_host_ = nullptr;
  }
}

NveRnComponentBridge::NveRnComponentBridge(
    NveGetInterfaceFn get_interface,
    NveSurfaceConfig config,
    EventEmitterCallback on_event)
    : NveRnComponentBridge(nve_resolve_module(get_interface), config, std::move(on_event)) {}

NveRnComponentBridge::~NveRnComponentBridge() {
  destroy_handles();
}

NveRnComponentBridge::NveRnComponentBridge(NveRnComponentBridge&& other) noexcept
    : module_(other.module_),
      config_(other.config_),
      fabric_host_(other.fabric_host_),
      app_handle_(other.app_handle_),
      next_frame_id_(other.next_frame_id_),
      on_event_(std::move(other.on_event_)) {
  other.fabric_host_ = nullptr;
  other.app_handle_ = nullptr;
  other.module_ = {};
}

NveRnComponentBridge& NveRnComponentBridge::operator=(NveRnComponentBridge&& other) noexcept {
  if (this != &other) {
    destroy_handles();
    module_ = other.module_;
    config_ = other.config_;
    fabric_host_ = other.fabric_host_;
    app_handle_ = other.app_handle_;
    next_frame_id_ = other.next_frame_id_;
    on_event_ = std::move(other.on_event_);
    other.fabric_host_ = nullptr;
    other.app_handle_ = nullptr;
    other.module_ = {};
  }
  return *this;
}

void NveRnComponentBridge::destroy_handles() noexcept {
  if (app_handle_ && module_.surface && module_.surface->destroy) {
    module_.surface->destroy(app_handle_);
    app_handle_ = nullptr;
  }
  if (fabric_host_) {
    nve_fabric_host_destroy(fabric_host_);
    fabric_host_ = nullptr;
  }
}

void NveRnComponentBridge::set_event_callback(EventEmitterCallback callback) {
  on_event_ = std::move(callback);
}

void NveRnComponentBridge::set_surface_size(float width, float height, float scale_factor) {
  config_.width = width;
  config_.height = height;
  if (scale_factor > 0.0f) {
    config_.scale_factor = scale_factor;
  }
  if (fabric_host_) {
    fabric_host_set_surface_size(fabric_host_, config_.width, config_.height, config_.scale_factor);
  }
}

bool NveRnComponentBridge::update_props_json(std::string_view props_json, bool auto_render) {
  if (!is_valid() || !module_.component || !module_.component->set_props) {
    return false;
  }
  const NveBufferSlice slice{
      reinterpret_cast<const uint8_t*>(props_json.data()), props_json.size()};
  const uint32_t dirty = module_.component->set_props(app_handle_, slice);
  if (dirty != 0 && auto_render) {
    render(nullptr);
  }
  return dirty != 0;
}

bool NveRnComponentBridge::update_props(const PropMap& props, bool auto_render) {
  return update_props_json(serialize_props_json(props), auto_render);
}

int32_t NveRnComponentBridge::render(NveFrameStats* out_stats) {
  if (!is_valid() || !module_.surface || !module_.surface->render) {
    return -100;
  }
  const NveBufferSlice cmd = module_.surface->render(app_handle_, next_frame_id_++);
  return nve_fabric_host_submit_frame(fabric_host_, cmd, out_stats);
}

uint32_t NveRnComponentBridge::dispatch_native_events(NveBufferSlice events, bool auto_render) {
  if (!is_valid() || !module_.surface || !module_.surface->dispatch_events) {
    return 0;
  }
  const uint32_t dirty = module_.surface->dispatch_events(app_handle_, events);
  if (dirty != 0 && auto_render) {
    render(nullptr);
  }
  return dirty;
}

std::optional<NveIntrinsicSize> NveRnComponentBridge::measure(
    NveSizeConstraints constraints) const {
  if (!is_valid() || !module_.layout || !module_.layout->measure) {
    return std::nullopt;
  }
  return module_.layout->measure(app_handle_, constraints);
}

NveGestureDisposition NveRnComponentBridge::hit_test(
    float x,
    float y,
    uint8_t pointer_phase) const {
  if (!is_valid() || !module_.layout || !module_.layout->hit_test) {
    return NVE_GESTURE_PASS_THROUGH;
  }
  const uint8_t code = module_.layout->hit_test(app_handle_, x, y, pointer_phase);
  if (code == NVE_GESTURE_CLAIM) return NVE_GESTURE_CLAIM;
  if (code == NVE_GESTURE_DEFER_TO_PARENT) return NVE_GESTURE_DEFER_TO_PARENT;
  return NVE_GESTURE_PASS_THROUGH;
}

std::vector<DecodedEvent> NveRnComponentBridge::drain_emitted_events() {
  std::vector<DecodedEvent> events;
  if (!is_valid() || !module_.component || !module_.component->drain_emitted_events) {
    return events;
  }
  const NveBufferSlice slice = module_.component->drain_emitted_events(app_handle_);
  if (!decode_event_ring(slice, events)) {
    return {};
  }
  if (on_event_) {
    for (const auto& ev : events) {
      on_event_(ev);
    }
  }
  return events;
}

bool NveRnComponentBridge::apply_hot_patch_json(std::string_view patch_json, bool auto_render) {
  if (!is_valid() || !module_.dev || !module_.dev->apply_hot_patch) {
    return false;
  }
  const NveBufferSlice slice{
      reinterpret_cast<const uint8_t*>(patch_json.data()), patch_json.size()};
  const uint32_t dirty = module_.dev->apply_hot_patch(app_handle_, slice);
  if (dirty != 0 && auto_render) {
    render(nullptr);
  }
  return dirty != 0;
}

std::vector<uint8_t> NveRnComponentBridge::snapshot_state() const {
  if (!is_valid() || !module_.dev || !module_.dev->snapshot_state) {
    return {};
  }
  const NveBufferSlice slice = module_.dev->snapshot_state(app_handle_);
  if (!slice.ptr || slice.len == 0) {
    return {};
  }
  return std::vector<uint8_t>(slice.ptr, slice.ptr + slice.len);
}

void NveRnComponentBridge::restore_state(NveBufferSlice state, bool auto_render) {
  if (!is_valid() || !module_.dev || !module_.dev->restore_state) {
    return;
  }
  module_.dev->restore_state(app_handle_, state);
  if (auto_render) {
    render(nullptr);
  }
}

uint32_t NveRnComponentBridge::node_count() const noexcept {
  return fabric_host_ ? nve_fabric_host_node_count(fabric_host_) : 0;
}

std::optional<LayoutRect> NveRnComponentBridge::node_layout(uint32_t node_id) const {
  LayoutRect rect{};
  if (!fabric_host_ || !fabric_host_get_node_layout(fabric_host_, node_id, &rect)) {
    return std::nullopt;
  }
  return rect;
}

} // namespace nve
