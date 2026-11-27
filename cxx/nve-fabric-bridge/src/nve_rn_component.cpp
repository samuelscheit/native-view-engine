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

bool is_valid_utf8(const uint8_t* data, size_t len) noexcept {
  if (len == 0u) return true;
  if (data == nullptr) return false;
  size_t i = 0u;
  while (i < len) {
    const uint8_t b0 = data[i];
    if (b0 < 0x80u) {
      ++i;
    } else if (b0 >= 0xC2u && b0 <= 0xDFu) {
      if (i + 1u >= len || (data[i + 1u] & 0xC0u) != 0x80u) return false;
      i += 2u;
    } else if (b0 == 0xE0u) {
      if (i + 2u >= len || data[i + 1u] < 0xA0u || data[i + 1u] > 0xBFu ||
          (data[i + 2u] & 0xC0u) != 0x80u) return false;
      i += 3u;
    } else if ((b0 >= 0xE1u && b0 <= 0xECu) || (b0 >= 0xEEu && b0 <= 0xEFu)) {
      if (i + 2u >= len || (data[i + 1u] & 0xC0u) != 0x80u ||
          (data[i + 2u] & 0xC0u) != 0x80u) return false;
      i += 3u;
    } else if (b0 == 0xEDu) {
      if (i + 2u >= len || data[i + 1u] < 0x80u || data[i + 1u] > 0x9Fu ||
          (data[i + 2u] & 0xC0u) != 0x80u) return false;
      i += 3u;
    } else if (b0 == 0xF0u) {
      if (i + 3u >= len || data[i + 1u] < 0x90u || data[i + 1u] > 0xBFu ||
          (data[i + 2u] & 0xC0u) != 0x80u || (data[i + 3u] & 0xC0u) != 0x80u) return false;
      i += 4u;
    } else if (b0 >= 0xF1u && b0 <= 0xF3u) {
      if (i + 3u >= len || (data[i + 1u] & 0xC0u) != 0x80u ||
          (data[i + 2u] & 0xC0u) != 0x80u || (data[i + 3u] & 0xC0u) != 0x80u) return false;
      i += 4u;
    } else if (b0 == 0xF4u) {
      if (i + 3u >= len || data[i + 1u] < 0x80u || data[i + 1u] > 0x8Fu ||
          (data[i + 2u] & 0xC0u) != 0x80u || (data[i + 3u] & 0xC0u) != 0x80u) return false;
      i += 4u;
    } else {
      return false;
    }
  }
  return true;
}

bool decode_nve_event_record(
    const NveEvent& src,
    const uint8_t* arena,
    size_t arena_len,
    DecodedEvent& out) {
  if (src.bytes.offset > arena_len || src.bytes.len > arena_len - src.bytes.offset ||
      (src.bytes.len > 0u && arena == nullptr)) {
    return false;
  }
  const uint8_t* ev_bytes = src.bytes.len > 0u ? (arena + src.bytes.offset) : nullptr;
  const size_t ev_len = src.bytes.len;

  const bool has_utf8 = (src.flags & NVE_EVENT_FLAG_UTF8_BYTES) != 0u;
  const bool has_inline_key = (src.flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) != 0u;
  const bool has_split_key = (src.flags & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) != 0u;
  const bool has_custom = (src.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) != 0u;
  if ((has_inline_key && has_split_key) || (has_custom && (has_inline_key || has_split_key))) {
    return false;
  }
  if (has_utf8 && !is_valid_utf8(ev_bytes, ev_len)) {
    return false;
  }

  out = DecodedEvent{};
  out.surface_id = src.surface;
  out.node_id = src.node;
  out.timestamp_us = src.timestamp;
  out.table = src.table;
  out.event_id = src.event;
  out.modifiers = static_cast<uint8_t>(src.flags & 0x1Fu);
  out.event_flags = src.flags;

  if (src.table == NVE_TABLE_INTERACTION) {
    switch (src.event) {
      case NVE_INTERACTION_EVENT_PRESS:
        out.kind = EventKind::Press;
        out.x = event_payload_f32(src, 0u);
        out.y = event_payload_f32(src, 1u);
        break;
      case NVE_INTERACTION_EVENT_LONG_PRESS:
        out.kind = EventKind::LongPress;
        out.x = event_payload_f32(src, 0u);
        out.y = event_payload_f32(src, 1u);
        break;
      case NVE_INTERACTION_EVENT_FOCUS:
        out.kind = EventKind::Focus;
        break;
      case NVE_INTERACTION_EVENT_BLUR:
        out.kind = EventKind::Blur;
        break;
      case NVE_INTERACTION_EVENT_KEY_DOWN:
      case NVE_INTERACTION_EVENT_KEY_UP: {
        out.kind = (src.event == NVE_INTERACTION_EVENT_KEY_DOWN) ? EventKind::KeyDown
                                                                 : EventKind::KeyUp;
        out.key_code = src.payload[0];
        const uint32_t key_len = src.payload[1];
        if (has_inline_key) {
          if (key_len > NVE_EVENT_INLINE_KEY_CAP) {
            return false;
          }
          std::array<uint8_t, NVE_EVENT_INLINE_KEY_CAP> kb{};
          for (size_t w = 0u; w < 4u; ++w) {
            const uint32_t word = src.payload[2u + w];
            kb[w * 4u + 0u] = static_cast<uint8_t>(word & 0xFFu);
            kb[w * 4u + 1u] = static_cast<uint8_t>((word >> 8u) & 0xFFu);
            kb[w * 4u + 2u] = static_cast<uint8_t>((word >> 16u) & 0xFFu);
            kb[w * 4u + 3u] = static_cast<uint8_t>((word >> 24u) & 0xFFu);
          }
          if (!is_valid_utf8(kb.data(), key_len) || !is_valid_utf8(ev_bytes, ev_len)) {
            return false;
          }
          out.text.assign(reinterpret_cast<const char*>(kb.data()), key_len);
          if (ev_len > 0u) {
            out.code.assign(reinterpret_cast<const char*>(ev_bytes), ev_len);
          }
        } else if (has_split_key) {
          if (key_len > ev_len || !is_valid_utf8(ev_bytes, key_len) ||
              !is_valid_utf8(ev_bytes + key_len, ev_len - key_len)) {
            return false;
          }
          if (key_len > 0u) {
            out.text.assign(reinterpret_cast<const char*>(ev_bytes), key_len);
          }
          if (ev_len > key_len) {
            out.code.assign(reinterpret_cast<const char*>(ev_bytes + key_len), ev_len - key_len);
          }
        } else if (ev_len > 0u) {
          if (!is_valid_utf8(ev_bytes, ev_len)) {
            return false;
          }
          out.text.assign(reinterpret_cast<const char*>(ev_bytes), ev_len);
        }
        break;
      }
      case NVE_INTERACTION_EVENT_POINTER_DOWN:
      case NVE_INTERACTION_EVENT_POINTER_MOVE:
      case NVE_INTERACTION_EVENT_POINTER_UP:
      case NVE_INTERACTION_EVENT_POINTER_ENTER:
      case NVE_INTERACTION_EVENT_POINTER_LEAVE:
        switch (src.event) {
          case NVE_INTERACTION_EVENT_POINTER_DOWN: out.kind = EventKind::PointerDown; break;
          case NVE_INTERACTION_EVENT_POINTER_MOVE: out.kind = EventKind::PointerMove; break;
          case NVE_INTERACTION_EVENT_POINTER_UP: out.kind = EventKind::PointerUp; break;
          case NVE_INTERACTION_EVENT_POINTER_ENTER: out.kind = EventKind::PointerEnter; break;
          default: out.kind = EventKind::PointerLeave; break;
        }
        out.x = event_payload_f32(src, 0u);
        out.y = event_payload_f32(src, 1u);
        break;
      case NVE_INTERACTION_EVENT_WHEEL:
        out.kind = EventKind::Wheel;
        out.x = event_payload_f32(src, 0u);
        out.y = event_payload_f32(src, 1u);
        break;
      default:
        return false;
    }
    out.name = canonical_event_name(out.kind);
    return true;
  }

  if (src.table == NVE_TABLE_TEXT_INPUT) {
    switch (src.event) {
      case NVE_TEXT_INPUT_EVENT_CHANGE: out.kind = EventKind::TextChange; break;
      case NVE_TEXT_INPUT_EVENT_SUBMIT: out.kind = EventKind::Submit; break;
      case NVE_TEXT_INPUT_EVENT_COMPOSITION_START: out.kind = EventKind::CompositionStart; break;
      case NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE: out.kind = EventKind::CompositionUpdate; break;
      case NVE_TEXT_INPUT_EVENT_COMPOSITION_END: out.kind = EventKind::CompositionEnd; break;
      default: return false;
    }
    if (!is_valid_utf8(ev_bytes, ev_len)) {
      return false;
    }
    out.name = canonical_event_name(out.kind);
    if (ev_len > 0u) {
      out.text.assign(reinterpret_cast<const char*>(ev_bytes), ev_len);
      out.payload_bytes.assign(ev_bytes, ev_bytes + ev_len);
    }
    return true;
  }

  if (src.table == NVE_TABLE_SCROLL) {
    if (src.event != NVE_SCROLL_EVENT_SCROLL) return false;
    out.kind = EventKind::Scroll;
    out.name = canonical_event_name(out.kind);
    out.x = event_payload_f32(src, 0u);
    out.y = event_payload_f32(src, 1u);
    out.width = event_payload_f32(src, 2u);
    out.height = event_payload_f32(src, 3u);
    return true;
  }

  if (src.table == NVE_TABLE_LAYOUT) {
    if (src.event != NVE_LAYOUT_EVENT_LAYOUT) return false;
    out.kind = EventKind::Layout;
    out.name = canonical_event_name(out.kind);
    out.x = event_payload_f32(src, 0u);
    out.y = event_payload_f32(src, 1u);
    out.width = event_payload_f32(src, 2u);
    out.height = event_payload_f32(src, 3u);
    return true;
  }

  if (src.table != NVE_TABLE_CUSTOM) {
    return false;
  }
  out.kind = EventKind::Custom;
  std::string_view cname;
  std::span<const uint8_t> cpayload;
  if (!event_custom_parts(src, arena, arena_len, cname, cpayload)) {
    return false;
  }
  if (has_custom &&
      !is_valid_utf8(reinterpret_cast<const uint8_t*>(cname.data()), cname.size())) {
    return false;
  }
  out.name = has_custom ? std::string(cname)
                        : std::string(canonical_event_name(EventKind::Custom));
  if (!cpayload.empty()) {
    out.payload_bytes.assign(cpayload.begin(), cpayload.end());
  }
  return true;
}

uint64_t read_le_u64(const uint8_t* p) noexcept {
  return static_cast<uint64_t>(wire::read_le_u32(p)) |
         (static_cast<uint64_t>(wire::read_le_u32(p + 4u)) << 32u);
}

NveEvent read_event_le(const uint8_t* p) noexcept {
  NveEvent ev{};
  ev.timestamp = read_le_u64(p + wire::kEventOffsetTimestamp);
  ev.table = read_le_u64(p + wire::kEventOffsetTable);
  ev.surface = wire::read_le_u32(p + wire::kEventOffsetSurface);
  ev.node = wire::read_le_u32(p + wire::kEventOffsetNode);
  ev.event = wire::read_le_u32(p + wire::kEventOffsetEvent);
  ev.flags = wire::read_le_u32(p + wire::kEventOffsetFlags);
  for (size_t i = 0u; i < 6u; ++i) {
    ev.payload[i] = wire::read_le_u32(p + wire::kEventOffsetPayload + i * 4u);
  }
  ev.bytes.offset = wire::read_le_u32(p + wire::kEventOffsetBytesOffset);
  ev.bytes.len = wire::read_le_u32(p + wire::kEventOffsetBytesLen);
  return ev;
}

} // namespace

bool decode_event_ring(NveBufferSlice slice, std::vector<DecodedEvent>& out_events) {
  out_events.clear();
  if (slice.ptr == nullptr || slice.len < wire::kEventWireHeaderSize) {
    return false;
  }

  const auto hdr = wire::decode_event_batch_header(std::span<const uint8_t>(slice.ptr, slice.len));
  if (!hdr.has_value()) {
    return false;
  }
  const size_t body_len = slice.len - wire::kEventWireHeaderSize;
  const size_t max_events = body_len / wire::kEventRecordSize;
  if (static_cast<size_t>(hdr->event_count) > max_events) {
    return false;
  }
  const size_t ev_bytes = static_cast<size_t>(hdr->event_count) * wire::kEventRecordSize;
  if (body_len - ev_bytes != static_cast<size_t>(hdr->bytes_len)) {
    return false;
  }

  const uint8_t* ev_base = slice.ptr + wire::kEventWireHeaderSize;
  const uint8_t* arena_base = ev_base + ev_bytes;
  const size_t arena_len = hdr->bytes_len;

  out_events.reserve(hdr->event_count < 4096u ? hdr->event_count : 4096u);
  for (uint32_t i = 0u; i < hdr->event_count; ++i) {
    const NveEvent raw = read_event_le(ev_base + static_cast<size_t>(i) * wire::kEventRecordSize);
    DecodedEvent decoded{};
    if (!decode_nve_event_record(raw, arena_base, arena_len, decoded)) {
      out_events.clear();
      return false;
    }
    out_events.push_back(std::move(decoded));
  }
  return true;
}

bool decode_direct_events(NveEventBatch batch, std::vector<DecodedEvent>& out_events) {
  out_events.clear();
  if (batch.bytes_len > 0u && batch.bytes == nullptr) {
    return false;
  }
  if (batch.event_count == 0u) {
    return true;
  }
  if (batch.events == nullptr) {
    return false;
  }
  out_events.reserve(batch.event_count < 4096u ? batch.event_count : 4096u);
  for (uint32_t i = 0u; i < batch.event_count; ++i) {
    DecodedEvent decoded{};
    if (!decode_nve_event_record(batch.events[i], batch.bytes, batch.bytes_len, decoded)) {
      out_events.clear();
      return false;
    }
    out_events.push_back(std::move(decoded));
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
