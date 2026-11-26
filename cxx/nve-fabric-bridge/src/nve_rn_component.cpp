#include "nve_rn_component.hpp"
#include "command_decoder.hpp"
#include "mock_jsi_recorder.hpp"

#include <cmath>
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
    case EventKind::CompositionStart: return "onCompositionStart";
    case EventKind::CompositionUpdate: return "onCompositionUpdate";
    case EventKind::CompositionEnd: return "onCompositionEnd";
    case EventKind::KeyDown: return "onKeyDown";
    case EventKind::KeyUp: return "onKeyUp";
    case EventKind::Submit: return "onSubmit";
    case EventKind::PointerDown: return "onPointerDown";
    case EventKind::PointerMove: return "onPointerMove";
    case EventKind::PointerUp: return "onPointerUp";
    case EventKind::PointerEnter: return "onPointerEnter";
    case EventKind::PointerLeave: return "onPointerLeave";
    case EventKind::Wheel: return "onWheel";
  }
  return "onUnknown";
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
    if (raw_kind < 1 || raw_kind > 20) {
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
      case 6: { /* Key */
        uint8_t is_composing = 0;
        uint16_t klen = 0;
        uint16_t clen = 0;
        std::string_view key;
        std::string_view code;
        if (!r.read(ev.modifiers) || !r.read(is_composing) || !r.read(ev.key_code) ||
            !r.read(klen) || !r.read_string(klen, key) || !r.read(clen) ||
            !r.read_string(clen, code)) {
          out_events.clear();
          return false;
        }
        ev.event_flags = is_composing != 0 ? NVE_EVENT_FLAG_COMPOSING : 0u;
        ev.text.assign(key.begin(), key.end());
        ev.code.assign(code.begin(), code.end());
        break;
      }
      case 7: /* Pointer */
        if (!r.read(ev.x) || !r.read(ev.y) || !r.read(ev.modifiers) || !r.read(ev.event_flags)) {
          out_events.clear();
          return false;
        }
        break;
      case 8: /* Wheel */
        if (!r.read(ev.x) || !r.read(ev.y) || !r.read(ev.modifiers)) {
          out_events.clear();
          return false;
        }
        break;
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

bool decode_direct_events(NveEventBatch batch, std::vector<DecodedEvent>& out_events) {
  out_events.clear();
  if (batch.len == 0u) {
    return true;
  }
  if (batch.ptr == nullptr) {
    return false;
  }
  out_events.reserve(batch.len < 4096u ? batch.len : 4096u);
  for (uint32_t i = 0u; i < batch.len; ++i) {
    const NveDirectEvent& src = batch.ptr[i];
    if (src.kind < 1u || src.kind > 20u) {
      out_events.clear();
      return false;
    }
    DecodedEvent ev{};
    ev.surface_id = src.surface_id;
    ev.node_id = src.node_id;
    ev.timestamp_us = src.timestamp_us;
    ev.kind = static_cast<EventKind>(src.kind);
    ev.name = canonical_event_name(ev.kind);
    ev.modifiers = src.modifiers;
    ev.event_flags = src.event_flags;
    ev.key_code = src.key_code;

    switch (src.payload_tag) {
      case NVE_EVENT_PAYLOAD_EMPTY:
        break;
      case NVE_EVENT_PAYLOAD_POINT:
      case NVE_EVENT_PAYLOAD_POINTER:
      case NVE_EVENT_PAYLOAD_WHEEL:
        ev.x = src.x;
        ev.y = src.y;
        break;
      case NVE_EVENT_PAYLOAD_SCROLL:
      case NVE_EVENT_PAYLOAD_LAYOUT:
        ev.x = src.x;
        ev.y = src.y;
        ev.width = src.width;
        ev.height = src.height;
        break;
      case NVE_EVENT_PAYLOAD_TEXT: {
        const std::string_view sv = str_slice_view(src.text);
        ev.text.assign(sv.begin(), sv.end());
        ev.payload_bytes.assign(
            reinterpret_cast<const uint8_t*>(sv.data()),
            reinterpret_cast<const uint8_t*>(sv.data()) + sv.size());
        break;
      }
      case NVE_EVENT_PAYLOAD_CUSTOM: {
        const std::string_view name = str_slice_view(src.name);
        ev.name.assign(name.begin(), name.end());
        if (src.value.tag == NVE_VAL_BYTES) {
          if (src.value.data.bytes_val.ptr != nullptr && src.value.data.bytes_val.len > 0u) {
            ev.payload_bytes.assign(
                src.value.data.bytes_val.ptr,
                src.value.data.bytes_val.ptr + src.value.data.bytes_val.len);
          }
        } else if (src.value.tag != NVE_VAL_NULL) {
          if (!encode_bin_value(ev.payload_bytes, src.value)) {
            out_events.clear();
            return false;
          }
        } else if (src.text.ptr != nullptr && src.text.len > 0u) {
          ev.payload_bytes.assign(src.text.ptr, src.text.ptr + src.text.len);
        }
        break;
      }
      case NVE_EVENT_PAYLOAD_KEY: {
        const std::string_view key = str_slice_view(src.text);
        const std::string_view code = str_slice_view(src.name);
        ev.text.assign(key.begin(), key.end());
        ev.code.assign(code.begin(), code.end());
        break;
      }
      default:
        out_events.clear();
        return false;
    }
    out_events.push_back(std::move(ev));
  }
  return true;
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
      on_event_(std::move(other.on_event_)),
      prop_arena_(std::move(other.prop_arena_)) {
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
    prop_arena_ = std::move(other.prop_arena_);
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

bool NveRnComponentBridge::update_props(
    const NveMapEntry* entries,
    uint32_t count,
    bool auto_render) {
  if (!is_valid() || !module_.component || module_.component->set_props == nullptr) {
    return false;
  }
  const uint32_t dirty = module_.component->set_props(app_handle_, entries, count);
  if (dirty != 0 && auto_render) {
    render(nullptr);
  }
  return dirty != 0;
}

bool NveRnComponentBridge::update_props(const PropMap& props, bool auto_render) {
  if (!is_valid() || !module_.component) {
    return false;
  }
  prop_arena_.reset();
  const uint32_t count = static_cast<uint32_t>(props.size());
  NveMapEntry* entries = count > 0 ? prop_arena_.alloc_array<NveMapEntry>(count) : nullptr;
  uint32_t idx = 0;
  for (const auto& [key, val] : props) {
    NveValue nv = make_null_value();
    std::visit(
        [&nv](const auto& v) {
          using T = std::decay_t<decltype(v)>;
          if constexpr (std::is_same_v<T, std::nullptr_t>) {
            nv = make_null_value();
          } else if constexpr (std::is_same_v<T, bool>) {
            nv = make_bool_value(v);
          } else if constexpr (std::is_same_v<T, int64_t>) {
            nv = make_i64_value(v);
          } else if constexpr (std::is_same_v<T, double>) {
            nv = std::isfinite(v) ? make_f64_value(v) : make_null_value();
          } else if constexpr (std::is_same_v<T, std::string>) {
            nv = make_str_value(std::string_view(v));
          }
        },
        val);
    entries[idx++] = NveMapEntry{make_str_slice(key), nv};
  }
  return update_props(entries, count, auto_render);
}

int32_t NveRnComponentBridge::render(NveFrameStats* out_stats) {
  if (!is_valid() || !module_.surface || !module_.surface->render) {
    return -100;
  }
  const NveBufferSlice cmd = module_.surface->render(app_handle_, next_frame_id_++);
  return nve_fabric_host_submit_frame(fabric_host_, cmd, out_stats);
}

uint32_t NveRnComponentBridge::dispatch_events(NveEventBatch batch, bool auto_render) {
  if (!is_valid() || !module_.surface || module_.surface->dispatch_events == nullptr) {
    return 0;
  }
  const uint32_t dirty = module_.surface->dispatch_events(app_handle_, batch);
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
  const NveEventBatch batch = module_.component->drain_emitted_events(app_handle_);
  if (!decode_direct_events(batch, events)) {
    return {};
  }
  if (on_event_) {
    for (const auto& ev : events) {
      on_event_(ev);
    }
  }
  return events;
}

bool NveRnComponentBridge::apply_hot_patch(std::span<const uint8_t> patch_nvp1, bool auto_render) {
  if (!is_valid() || !module_.dev || !module_.dev->apply_hot_patch) {
    return false;
  }
  const NveBufferSlice slice{patch_nvp1.data(), patch_nvp1.size()};
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
