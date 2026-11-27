// Built-in demo NVE app for the Apple shells (launched without `--dylib`) and their tests: a counter
// card exporting every interface `libnve_runtime` forwards (surface, layout, component, dev).
//
// ROOT(1) -> card(2) -> [title(3) text, button(4) pressable -> label(5) "Clicks: N"]
#pragma once

#include <nve/host/nve2_builder.hpp>
#include <nve/generated/nve_abi.generated.h>

#include <array>
#include <cstdint>
#include <cstdlib>
#include <string>
#include <string_view>
#include <vector>

namespace nve::apple {

class DemoApp {
 public:
  static constexpr uint32_t kCard = 2u, kTitle = 3u, kButton = 4u, kLabel = 5u;

  explicit DemoApp(uint32_t surface) : surface_(surface) {}

  uint32_t count() const noexcept { return count_; }
  const std::string &title() const noexcept { return title_; }

  static const void *get_interface(uint32_t id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    static const NveLayoutApi kLayout{&measure, &hit_test};
    static const NveComponentApi kComponent{&set_props, &drain_emitted_events};
    static const NveDevApi kDev{&apply_hot_patch, &snapshot_state, &restore_state};
    switch (id) {
      case NVE_IFACE_SURFACE: return &kSurface;
      case NVE_IFACE_LAYOUT: return &kLayout;
      case NVE_IFACE_COMPONENT: return &kComponent;
      case NVE_IFACE_DEV: return &kDev;
      default: return nullptr;
    }
  }

 private:
  /// Borrowed UTF-8 of a slice (empty for NULL / zero length, never `std::string(nullptr, 0)`).
  static std::string_view view(NveStrSlice s) noexcept {
    return s.ptr != nullptr && s.len > 0u ? std::string_view(reinterpret_cast<const char *>(s.ptr), s.len)
                                          : std::string_view();
  }
  static DemoApp *self(NveAppHandle h) { return static_cast<DemoApp *>(h); }
  static NveAppHandle create(uint32_t surface) { return new DemoApp(surface); }
  static void destroy(NveAppHandle h) { delete self(h); }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    DemoApp *app = self(h);
    if (!app->dirty_) return NveBufferSlice{nullptr, 0u};
    app->dirty_ = false;
    app->frame_ = app->built_ ? app->delta(frame_id) : app->keyframe();
    app->built_ = true;
    return NveBufferSlice{app->frame_.data(), app->frame_.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    DemoApp *app = self(h);
    uint32_t changed = 0u;
    for (uint32_t i = 0; batch.ptr != nullptr && i < batch.len; ++i) {
      if (batch.ptr[i].kind == NVE_EVENT_PRESS && (batch.ptr[i].node_id & 0x00FFFFFFu) == kButton) {
        ++app->count_;
        app->dirty_ = true;
        app->emit_count();
        changed = 1u;
      }
    }
    return changed;
  }

  static NveIntrinsicSize measure(NveAppHandle, NveSizeConstraints c) {
    const float w = c.max_w < 320.0f ? c.max_w : 320.0f;
    return NveIntrinsicSize{w > c.min_w ? w : c.min_w, 200.0f};
  }

  static uint8_t hit_test(NveAppHandle, float, float, uint8_t) { return 0u; }

  static uint32_t set_props(NveAppHandle h, const NveMapEntry *entries, uint32_t count) {
    DemoApp *app = self(h);
    for (uint32_t i = 0; entries != nullptr && i < count; ++i) {
      const NveMapEntry &e = entries[i];
      if (view(e.key) == "title" && e.value.tag == NVE_VAL_STR) {
        std::string next(view(e.value.data.str_val));
        if (next != app->title_) {
          app->title_ = std::move(next);
          app->dirty_ = true;
        }
      } else if (view(e.key) == "accent" && e.value.tag == NVE_VAL_I64) {
        const uint32_t next = static_cast<uint32_t>(e.value.data.i64_val);
        if (next != app->accent_) {
          app->accent_ = next;
          app->dirty_ = true;
        }
      }
    }
    return app->dirty_ ? 1u : 0u;
  }

  static NveEventBatch drain_emitted_events(NveAppHandle h) {
    DemoApp *app = self(h);
    app->drained_.swap(app->emitted_);
    app->emitted_.clear();
    return NveEventBatch{app->drained_.data(), static_cast<uint32_t>(app->drained_.size()), 0u};
  }

  static uint32_t apply_hot_patch(NveAppHandle, NveBufferSlice) { return 0u; }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    DemoApp *app = self(h);
    app->state_ = std::to_string(app->count_);
    return NveBufferSlice{reinterpret_cast<const uint8_t *>(app->state_.data()), app->state_.size()};
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    DemoApp *app = self(h);
    if (state.ptr == nullptr || state.len == 0u) return;
    const std::string text(reinterpret_cast<const char *>(state.ptr), state.len);  // ptr checked above
    app->count_ = static_cast<uint32_t>(std::strtoul(text.c_str(), nullptr, 10));
    app->dirty_ = true;
  }

  void emit_count() {
    NveDirectEvent ev{};
    ev.surface_id = surface_;
    ev.node_id = kButton;
    ev.kind = NVE_EVENT_CUSTOM;
    ev.payload_tag = NVE_EVENT_PAYLOAD_CUSTOM;
    ev.name = NveStrSlice{reinterpret_cast<const uint8_t *>("countChanged"), 12u};
    ev.value.tag = NVE_VAL_I64;
    ev.value.data.i64_val = count_;
    emitted_.push_back(ev);
  }

  std::string label() const { return "Clicks: " + std::to_string(count_); }

  template <class Row>
  static void defaults(Row &row, const uint32_t *values) {
    for (size_t i = 0; i < row.size(); ++i) row[i] = values[i];
  }

  std::vector<uint8_t> keyframe() const {
    using nve::host::Key64;
    using nve::host::Nve2FrameBuilder;
    using nve::host::set_field;
    const uint8_t layout = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_LAYOUT_KEY});
    const uint8_t visual = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_VISUAL_KEY});
    const uint8_t text = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_TEXT_KEY});
    const uint8_t interaction = Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_INTERACTION_KEY});
    Nve2FrameBuilder b(1u);
    b.declare_core_tables().create_nodes({1u, kCard, kTitle, kButton, kLabel});

    std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> lay{};
    defaults(lay, NVE_TABLE_LAYOUT_DEFAULTS);
    uint32_t mask = 0u;
    for (const NveFieldSpec f : {NVE_TABLE_LAYOUT_PADDING_TOP, NVE_TABLE_LAYOUT_PADDING_RIGHT, NVE_TABLE_LAYOUT_PADDING_BOTTOM, NVE_TABLE_LAYOUT_PADDING_LEFT}) {
      mask |= set_field(lay, f, 0x41800000u);  // 16pt
    }
    b.patch(layout, kCard, mask, lay);
    defaults(lay, NVE_TABLE_LAYOUT_DEFAULTS);
    b.patch(layout, kButton, set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x42300000u), lay);  // 44pt

    std::array<uint32_t, NVE_TABLE_VISUAL_WORDS> vis{};
    defaults(vis, NVE_TABLE_VISUAL_DEFAULTS);
    b.patch(visual, kCard, set_field(vis, NVE_TABLE_VISUAL_BACKGROUND, 0xFF3B291Eu) | set_field(vis, NVE_TABLE_VISUAL_BORDER_RADIUS, 0x41400000u), vis);
    defaults(vis, NVE_TABLE_VISUAL_DEFAULTS);
    b.patch(visual, kButton, set_field(vis, NVE_TABLE_VISUAL_BACKGROUND, 0xFFEB6325u) | set_field(vis, NVE_TABLE_VISUAL_BORDER_RADIUS, 0x41000000u), vis);

    patch_card_typography(b);

    std::array<uint32_t, NVE_TABLE_INTERACTION_WORDS> in{};
    defaults(in, NVE_TABLE_INTERACTION_DEFAULTS);
    b.patch(interaction, kButton, set_field(in, NVE_TABLE_INTERACTION_EVENT_MASK, 1u /* EventMask::PRESS */), in);

    b.set_string(text, kTitle, NVE_TABLE_TEXT_TEXT.word, title_);
    b.set_string(text, kLabel, NVE_TABLE_TEXT_TEXT.word, label());
    b.set_children(1u, {kCard});
    b.set_children(kCard, {kTitle, kButton});
    b.set_children(kButton, {kLabel});
    return b.finish();
  }

  std::vector<uint8_t> delta(uint64_t frame_id) const {
    const uint8_t text = nve::host::Nve2FrameBuilder::core_slot(nve::host::Key64{NVE_TABLE_TEXT_KEY});
    nve::host::Nve2FrameBuilder b(frame_id < 2u ? 2u : frame_id);
    patch_card_typography(b);
    b.set_string(text, kTitle, NVE_TABLE_TEXT_TEXT.word, title_);
    b.set_string(text, kLabel, NVE_TABLE_TEXT_TEXT.word, label());
    return b.finish();
  }

  uint32_t surface_;
  /// Card typography: the title and label inherit color + size from it.
  void patch_card_typography(nve::host::Nve2FrameBuilder &b) const {
    using nve::host::set_field;
    const uint8_t typo = nve::host::Nve2FrameBuilder::core_slot(nve::host::Key64{NVE_TABLE_TYPOGRAPHY_KEY});
    std::array<uint32_t, NVE_TABLE_TYPOGRAPHY_WORDS> ty{};
    defaults(ty, NVE_TABLE_TYPOGRAPHY_DEFAULTS);
    b.patch(typo, kCard,
            set_field(ty, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_COLOR | NVE_TYPOGRAPHY_SET_FONT_SIZE) |
                set_field(ty, NVE_TABLE_TYPOGRAPHY_COLOR, accent_) | set_field(ty, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, 0x41900000u),
            ty);
  }

  uint32_t count_ = 0u;
  uint32_t accent_ = 0xFFF9F5F1u;
  std::string title_ = "NVE Apple Host";
  bool dirty_ = true;
  bool built_ = false;
  std::vector<uint8_t> frame_;
  std::vector<NveDirectEvent> emitted_;
  std::vector<NveDirectEvent> drained_;
  std::string state_;
};

}  // namespace nve::apple
