// NVE host runtime: the built-in counter demo every native shell runs without `--dylib`, and the
// stateful app fixture host tests load as a `cdylib`. A pure C ABI app (`nve_get_interface`)
// writing `NVE2` frames through `Nve2Writer` from generated table descriptors.
//
// Tree (packed `NodeId` == index, generation 0):
//   1 ROOT -> 2 column [3 title, 4 "Increment" button, 5 count label, 6 "Reset" button]
// Press on 4 increments, press on 6 resets; `NveDevApi` snapshots the count as 4 LE bytes.
// `NveComponentApi`: the `title` string prop retitles node 3, and every count change emits one
// `nve.custom` event named `count` whose binary payload is an `NveBinValue` `I64(count)`.
#pragma once

#include <cstdint>
#include <string>
#include <string_view>

#include "../generated/nve_abi.generated.h"
#include "../generated/nve_tables.generated.h"
#include "bin_codec.hpp"
#include "nve2_writer.hpp"
#include "owned_events.hpp"

namespace nve::host::demo {

class CounterApp {
 public:
  static constexpr uint32_t kColumn = 2u;
  static constexpr uint32_t kTitle = 3u;
  static constexpr uint32_t kIncrement = 4u;
  static constexpr uint32_t kCount = 5u;
  static constexpr uint32_t kReset = 6u;

  [[nodiscard]] static std::string count_label(int32_t count) { return "Count: " + std::to_string(count); }

  /// `nve_get_interface` of this app.
  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    // Every slot of a returned interface must be filled (the runtime rejects partial vtables).
    static const NveDevApi kDev{&apply_hot_patch, &snapshot_state, &restore_state};
    static const NveComponentApi kComponent{&set_props, &drain_emitted_events};
    if (interface_id == NVE_IFACE_SURFACE) return &kSurface;
    if (interface_id == NVE_IFACE_DEV) return &kDev;
    if (interface_id == NVE_IFACE_COMPONENT) return &kComponent;
    return nullptr;
  }

  static constexpr std::string_view kCountEventName = "count";

 private:
  static NveAppHandle create(uint32_t) { return new CounterApp(); }
  static void destroy(NveAppHandle h) { delete static_cast<CounterApp*>(h); }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<CounterApp*>(h);
    if (!self->mounted_) self->mount();
    if (self->count_dirty_) {
      self->w_.set_str(kCount, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, count_label(self->count_));
      self->count_dirty_ = false;
    }
    if (!self->w_.pending()) return NveBufferSlice{nullptr, 0u};
    const auto frame = self->w_.finish(frame_id);
    return NveBufferSlice{frame.data(), frame.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<CounterApp*>(h);
    const int32_t before = self->count_;
    for (uint32_t i = 0u; batch.events != nullptr && i < batch.event_count; ++i) {
      const NveEvent& ev = batch.events[i];
      if (!is_interaction_event(ev, NVE_INTERACTION_EVENT_PRESS)) continue;
      if (ev.node == kIncrement) ++self->count_;
      if (ev.node == kReset) self->count_ = 0;
    }
    if (before != self->count_) self->count_changed();
    return before != self->count_ ? 1u : 0u;
  }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
    auto* self = static_cast<CounterApp*>(h);
    uint32_t changed = 0u;
    for (uint32_t i = 0u; entries != nullptr && i < count; ++i) {
      const NveMapEntry& e = entries[i];
      const std::string_view key(reinterpret_cast<const char*>(e.key.ptr), e.key.len);
      if (key != "title" || e.value.tag != NVE_VAL_STR) continue;
      const NveStrSlice s = e.value.data.str_val;
      const std::string_view title(reinterpret_cast<const char*>(s.ptr), s.len);
      if (!self->mounted_) self->mount();
      changed |= self->w_.set_str(kTitle, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, title) ? 1u : 0u;
    }
    return changed;
  }

  static NveEventBatch drain_emitted_events(NveAppHandle h) {
    auto* self = static_cast<CounterApp*>(h);
    self->drained_.swap(self->emitted_);
    self->emitted_.clear();
    return self->drained_.batch();
  }

  void count_changed() {
    count_dirty_ = true;
    bin_.clear();
    bin_.write_i64(count_);
    (void)emitted_.push_custom(1u, kCount, 0u, kCountEventName, bin_.bytes());
  }

  /// The demo has no `rsx!` templates to patch.
  static uint32_t apply_hot_patch(NveAppHandle, NveBufferSlice) { return 0u; }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    auto* self = static_cast<CounterApp*>(h);
    const auto c = static_cast<uint32_t>(self->count_);
    for (uint32_t i = 0u; i < 4u; ++i) self->state_[i] = static_cast<uint8_t>(c >> (i * 8u));
    return NveBufferSlice{self->state_, 4u};
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    auto* self = static_cast<CounterApp*>(h);
    if (state.ptr == nullptr || state.len != 4u) return;
    uint32_t c = 0u;
    for (uint32_t i = 0u; i < 4u; ++i) c |= uint32_t{state.ptr[i]} << (i * 8u);
    self->count_ = static_cast<int32_t>(c);
    self->count_changed();
  }

  void text_node(uint32_t id, const char* text, float font_size) {
    w_.create(id);
    w_.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, text);
    w_.set_f32(id, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, font_size);
    w_.set(id, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_COLOR, rgba(15, 23, 42));
  }

  void button(uint32_t id, const char* label, uint32_t background) {
    text_node(id, label, 16.0f);
    w_.set(id, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_COLOR, rgba(255, 255, 255));
    w_.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, length_points(40.0f));
    w_.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_LEFT, length_points(16.0f));
    w_.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_RIGHT, length_points(16.0f));
    w_.set(id, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, background);
    w_.set_f32(id, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 8.0f);
    w_.set_events(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD, NVE_INTERACTION_EVENT_SET_PRESS);
    w_.set_bool(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
    w_.set(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_BUTTON);
    w_.set_str(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, label);
  }

  void mount() {
    mounted_ = true;
    w_.create(1u);  // the receiving replica mirrors node ids exactly, ROOT included
    w_.create(kColumn);
    w_.set(kColumn, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_ROW_GAP, length_points(12.0f));
    for (const NveFieldSpec f : {NVE_TABLE_LAYOUT_PADDING_TOP, NVE_TABLE_LAYOUT_PADDING_RIGHT,
                                 NVE_TABLE_LAYOUT_PADDING_BOTTOM, NVE_TABLE_LAYOUT_PADDING_LEFT}) {
      w_.set(kColumn, NVE_TABLE_LAYOUT_DESC, f, length_points(24.0f));
    }
    w_.set(kColumn, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_ALIGN_ITEMS, NVE_ALIGN_ITEMS_FLEX_START);
    w_.set(kColumn, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, rgba(248, 250, 252));
    text_node(kTitle, "NVE native host", 22.0f);
    w_.set(kTitle, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_HEADING);
    button(kIncrement, "Increment", rgba(37, 99, 235));
    text_node(kCount, "", 18.0f);
    button(kReset, "Reset", rgba(100, 116, 139));
    const uint32_t kids[] = {kTitle, kIncrement, kCount, kReset};
    w_.set_children(kColumn, kids);
    const uint32_t root_kids[] = {kColumn};
    w_.set_children(1u, root_kids);
    count_dirty_ = true;
  }

  Nve2Writer w_;
  BinWriter bin_;
  int32_t count_ = 0;
  bool mounted_ = false;
  bool count_dirty_ = false;
  uint8_t state_[4] = {};
  OwnedEvents emitted_;
  OwnedEvents drained_;  // borrowed by the runtime until the next drain
};

}  // namespace nve::host::demo
