#pragma once

#include "command_decoder.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_abi.h"
#include "nve_rn_component.hpp"
#include "nve_scene_abi.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <memory>
#include <mutex>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
#include <yoga/Yoga.h>
#endif

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
#include <react/renderer/core/EventListener.h>
#include <react/renderer/core/EventPayload.h>
#include <react/renderer/core/EventPayloadType.h>
#include <react/renderer/core/RawEvent.h>
#include <react/renderer/core/ShadowNodeFamily.h>
#include <react/renderer/mounting/ShadowTree.h>
#include <react/renderer/uimanager/UIManager.h>
#endif

namespace nve {

inline constexpr uint64_t kEventMaskLayout = 1ULL << 6;
inline constexpr uint8_t kAlignSelfAuto = 0xFFu;
inline constexpr uint32_t kDenseIdCap = 65536u;
inline constexpr uint32_t kInvalidSlot = std::numeric_limits<uint32_t>::max();

// Per-node dirty bits controlling minimal `NveRenderNode` updates in `scene_publisher.cpp`.
inline constexpr uint8_t kSceneDirtyLayout = 1u << 0;
inline constexpr uint8_t kSceneDirtyStyle = 1u << 1;
inline constexpr uint8_t kSceneDirtyText = 1u << 2;
inline constexpr uint8_t kSceneDirtyChildren = 1u << 3;
inline constexpr uint8_t kSceneDirtyAll = 0x0Fu;

inline uint64_t now_us() {
  using Clock = std::chrono::steady_clock;
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch())
          .count());
}

inline bool rect_changed(const LayoutRect& a, const LayoutRect& b) noexcept {
  constexpr float kEps = 1e-3f;
  return std::fabs(a.x - b.x) > kEps || std::fabs(a.y - b.y) > kEps ||
         std::fabs(a.width - b.width) > kEps || std::fabs(a.height - b.height) > kEps;
}

/// Shadow node stored contiguously inside `ShadowNodeTable`.
/// `raw_props` is heap-allocated lazily (nullptr for >99% of nodes) to keep nodes cache-compact.
struct ShadowNodeEntry {
  uint64_t event_mask{0};
  uint64_t visit_gen{0};
  uint64_t scene_dirty_gen{0};
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  YGNodeRef yg_node{nullptr};
#else
  uint64_t measure_gen{0};
  uint64_t baseline_gen{0};
  NveIntrinsicSize cached_measure{};
  float cached_avail_w{-2.0f};
  float cached_avail_h{-2.0f};
  float cached_baseline_w{-2.0f};
  float cached_baseline_h{-2.0f};
  float cached_baseline{0.0f};
  uint8_t cached_w_mode{0};
  uint8_t cached_h_mode{0};
#endif
  uint32_t node_id{};
  uint32_t parent_id{0};
  uint16_t kind{};
  uint8_t align_self{kAlignSelfAuto};
  uint8_t scene_dirty_mask{0};
  bool has_explicit_style{false};
  bool has_computed_layout{false};
  bool raw_props_dirty{false};
  bool has_parent{false};
  bool subtree_layout_dirty{true};
  Dimension flex_basis{0u, 0.0f};
  LayoutProps layout{};
  StyleProps style{};
  LayoutRect computed_layout{};
  std::string custom_name;
  std::string text;
  std::string image_src;
  std::vector<uint32_t> children;
  std::unique_ptr<RawPropMap> raw_props;
};

inline float compute_text_leaf_baseline(const ShadowNodeEntry& entry, float height) noexcept {
  if (entry.text.empty()) {
    return (std::isfinite(height) && height >= 0.0f) ? height : 0.0f;
  }
  const float font_size =
      (std::isfinite(entry.style.font_size) && entry.style.font_size > 0.0f)
          ? entry.style.font_size
          : 16.0f;
  const float pad_t = std::max(0.0f, entry.layout.padding.top);
  const float border_t =
      (std::isfinite(entry.style.border_width) && entry.style.border_width > 0.0f)
          ? entry.style.border_width
          : 0.0f;
  return pad_t + border_t + font_size * 0.8f;
}

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
void apply_yg_layout_props(YGNodeRef node, const LayoutProps& l) noexcept;
void apply_yg_align_self(YGNodeRef node, uint8_t align_self) noexcept;
void apply_yg_flex_basis(YGNodeRef node, const Dimension& d) noexcept;
#endif

/// Hybrid dense-slot + sparse-fallback ID table storing `ShadowNodeEntry` contiguously.
/// Direct `O(1)` array lookup for `node_id < kDenseIdCap` (65536), safe sparse map fallback
/// for arbitrary/fuzzed high IDs. Swap-removes on erase to keep iteration cache-dense.
class ShadowNodeTable {
 public:
  ShadowNodeTable() = default;

  void reserve(size_t count) {
    entries_.reserve(count);
    const size_t dense_cap = std::min<size_t>(kDenseIdCap, count + 8u);
    if (dense_id_to_slot_.size() < dense_cap) {
      dense_id_to_slot_.resize(dense_cap, kInvalidSlot);
    }
  }

  uint32_t find_slot(uint32_t id) const noexcept {
    if (id < dense_id_to_slot_.size()) {
      return dense_id_to_slot_[id];
    }
    if (id < kDenseIdCap) {
      return kInvalidSlot;
    }
    auto it = sparse_id_to_slot_.find(id);
    return (it != sparse_id_to_slot_.end()) ? it->second : kInvalidSlot;
  }

  ShadowNodeEntry* find(uint32_t id) noexcept {
    const uint32_t slot = find_slot(id);
    return (slot != kInvalidSlot) ? &entries_[slot] : nullptr;
  }

  const ShadowNodeEntry* find(uint32_t id) const noexcept {
    const uint32_t slot = find_slot(id);
    return (slot != kInvalidSlot) ? &entries_[slot] : nullptr;
  }

  /// Construct a new entry directly in-place (caller guarantees `id` is not present).
  ShadowNodeEntry& emplace_fresh(uint32_t id, uint16_t kind, std::string_view custom_name = {}) {
    const uint32_t slot = static_cast<uint32_t>(entries_.size());
    ShadowNodeEntry& entry = entries_.emplace_back();
    entry.node_id = id;
    entry.kind = kind;
    if (!custom_name.empty()) {
      entry.custom_name.assign(custom_name.data(), custom_name.size());
    }
    set_slot(id, slot);
    return entry;
  }

  bool erase(uint32_t id) {
    const uint32_t slot = find_slot(id);
    if (slot == kInvalidSlot) {
      return false;
    }
    clear_slot(id);
    const uint32_t last_slot = static_cast<uint32_t>(entries_.size() - 1u);
    if (slot != last_slot) {
      entries_[slot] = std::move(entries_[last_slot]);
      set_slot(entries_[slot].node_id, slot);
    }
    entries_.pop_back();
    return true;
  }

  size_t size() const noexcept { return entries_.size(); }
  bool empty() const noexcept { return entries_.empty(); }

  std::vector<ShadowNodeEntry>& entries() noexcept { return entries_; }
  const std::vector<ShadowNodeEntry>& entries() const noexcept { return entries_; }

 private:
  void set_slot(uint32_t id, uint32_t slot) {
    if (id < kDenseIdCap) {
      if (id >= dense_id_to_slot_.size()) {
        size_t next_size = std::max<size_t>(64u, dense_id_to_slot_.size() * 2u);
        while (next_size <= static_cast<size_t>(id)) {
          next_size *= 2u;
        }
        if (next_size > kDenseIdCap) {
          next_size = kDenseIdCap;
        }
        dense_id_to_slot_.resize(next_size, kInvalidSlot);
      }
      dense_id_to_slot_[id] = slot;
    } else {
      sparse_id_to_slot_[id] = slot;
    }
  }

  void clear_slot(uint32_t id) noexcept {
    if (id < dense_id_to_slot_.size()) {
      dense_id_to_slot_[id] = kInvalidSlot;
    } else if (id >= kDenseIdCap) {
      sparse_id_to_slot_.erase(id);
    }
  }

  std::vector<ShadowNodeEntry> entries_;
  std::vector<uint32_t> dense_id_to_slot_;
  std::unordered_map<uint32_t, uint32_t> sparse_id_to_slot_;
};

/// Chunked, pointer-stable bump scratch arena for recursive standalone Flexbox resolution.
/// Existing chunks are never reallocated while active scopes hold pointers into them;
/// rewinding a `Scope` resets `used` offsets in O(1) with zero heap free/malloc traffic.
class FlexScratchArena {
 public:
  static constexpr size_t kInitialChunkBytes = 65536u;

  FlexScratchArena() = default;
  FlexScratchArena(const FlexScratchArena&) = delete;
  FlexScratchArena& operator=(const FlexScratchArena&) = delete;

  struct Scope {
    FlexScratchArena& arena;
    size_t saved_chunk;
    size_t saved_used;

    explicit Scope(FlexScratchArena& a) noexcept
        : arena(a), saved_chunk(a.current_chunk_), saved_used(a.current_used()) {}
    ~Scope() noexcept { arena.restore(saved_chunk, saved_used); }

    Scope(const Scope&) = delete;
    Scope& operator=(const Scope&) = delete;
  };

  template <typename T>
  T* alloc(size_t count) {
    static_assert(std::is_trivially_destructible_v<T>);
    if (count == 0u) {
      return nullptr;
    }
    return static_cast<T*>(alloc_bytes(count * sizeof(T), alignof(T)));
  }

  template <typename T>
  T* alloc_zeroed(size_t count) {
    T* ptr = alloc<T>(count);
    if (ptr != nullptr) {
      std::memset(ptr, 0, count * sizeof(T));
    }
    return ptr;
  }

 private:
  struct Chunk {
    std::unique_ptr<uint8_t[]> data;
    size_t capacity{0};
    size_t used{0};
  };

  size_t current_used() const noexcept {
    return chunks_.empty() ? 0u : chunks_[current_chunk_].used;
  }

  void restore(size_t saved_chunk, size_t saved_used) noexcept {
    if (chunks_.empty()) {
      return;
    }
    for (size_t c = saved_chunk + 1u; c <= current_chunk_ && c < chunks_.size(); ++c) {
      chunks_[c].used = 0u;
    }
    current_chunk_ = saved_chunk;
    if (saved_chunk < chunks_.size()) {
      chunks_[saved_chunk].used = saved_used;
    }
  }

  static size_t align_up(size_t offset, size_t align) noexcept {
    const size_t mask = align - 1u;
    return (offset + mask) & ~mask;
  }

  void* alloc_bytes(size_t bytes, size_t align) {
    if (chunks_.empty()) {
      const size_t cap = std::max(kInitialChunkBytes, bytes + align);
      chunks_.push_back(Chunk{std::make_unique<uint8_t[]>(cap), cap, 0u});
      current_chunk_ = 0u;
    }

    for (;;) {
      Chunk& cur = chunks_[current_chunk_];
      const size_t aligned = align_up(cur.used, align);
      if (aligned <= cur.capacity && bytes <= cur.capacity - aligned) {
        cur.used = aligned + bytes;
        return cur.data.get() + aligned;
      }
      if (current_chunk_ + 1u < chunks_.size()) {
        ++current_chunk_;
        chunks_[current_chunk_].used = 0u;
        continue;
      }
      const size_t next_cap = std::max(cur.capacity * 2u, bytes + align);
      chunks_.push_back(Chunk{std::make_unique<uint8_t[]>(next_cap), next_cap, 0u});
      current_chunk_ = chunks_.size() - 1u;
    }
  }

  std::vector<Chunk> chunks_;
  size_t current_chunk_{0};
};

/// Direct 64-byte `NveRenderNode[]` + flat slab publisher state stored inside `FabricHost`.
struct ScenePublisherState {
  uint64_t frame_id{0};
  std::vector<NveRenderNode> nodes;
  std::vector<uint32_t> dense_id_to_index;
  std::unordered_map<uint32_t, uint32_t> sparse_id_to_index;
  std::vector<uint8_t> text_slab;
  std::vector<uint32_t> children_slab;
  std::vector<uint32_t> dirty_indices;
  std::vector<uint64_t> dirty_marks;
  std::vector<uint32_t> removed_ids;
  uint64_t mark_epoch{0};
  size_t live_text_bytes{0};
  size_t live_children_len{0};
  bool initialized{false};

  void reserve(size_t est_nodes) {
    nodes.reserve(est_nodes);
    dirty_marks.reserve(est_nodes);
    dirty_indices.reserve(est_nodes);
    const size_t dense_cap = std::min<size_t>(kDenseIdCap, est_nodes + 8u);
    if (dense_id_to_index.size() < dense_cap) {
      dense_id_to_index.resize(dense_cap, kInvalidSlot);
    }
    text_slab.reserve(est_nodes * 16u);
    children_slab.reserve(est_nodes);
  }

  uint32_t find_index(uint32_t id) const noexcept {
    if (id < dense_id_to_index.size()) {
      return dense_id_to_index[id];
    }
    if (id < kDenseIdCap) {
      return kInvalidSlot;
    }
    auto it = sparse_id_to_index.find(id);
    return (it != sparse_id_to_index.end()) ? it->second : kInvalidSlot;
  }

  void set_index(uint32_t id, uint32_t index) {
    if (id < kDenseIdCap) {
      if (id >= dense_id_to_index.size()) {
        size_t next_size = std::max<size_t>(64u, dense_id_to_index.size() * 2u);
        while (next_size <= static_cast<size_t>(id)) {
          next_size *= 2u;
        }
        if (next_size > kDenseIdCap) {
          next_size = kDenseIdCap;
        }
        dense_id_to_index.resize(next_size, kInvalidSlot);
      }
      dense_id_to_index[id] = index;
    } else {
      sparse_id_to_index[id] = index;
    }
  }

  void clear_index(uint32_t id) noexcept {
    if (id < dense_id_to_index.size()) {
      dense_id_to_index[id] = kInvalidSlot;
    } else if (id >= kDenseIdCap) {
      sparse_id_to_index.erase(id);
    }
  }
};

class FabricHost {
 public:
  explicit FabricHost(NveSurfaceConfig config);
  ~FabricHost();

  FabricHost(const FabricHost&) = delete;
  FabricHost& operator=(const FabricHost&) = delete;

  // Surface, callbacks & event queue
  void set_surface_size(float width, float height, float scale_factor);
  void set_text_measurer(TextMeasurerCallback measurer);
  void register_custom_component(CustomComponentDescriptor descriptor);
  void enqueue_press(uint32_t node_id, uint64_t ts_us, float x, float y);
  void enqueue_text_change(uint32_t node_id, uint64_t ts_us, std::string_view text);
  void enqueue_custom_event(
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const uint8_t* data,
      size_t len);
  NveBufferSlice drain_events();

  // Frame submission & inspection
  int32_t submit_frame(NveBufferSlice slice, NveFrameStats* out_stats);
  bool get_node_layout(uint32_t node_id, LayoutRect* out_rect) const;
  std::vector<RenderedNodeSnapshot> snapshot_nodes() const;
  std::vector<uint8_t> serialize_frame0(uint64_t frame_id) const;
  NveSceneView scene_view() const noexcept;
  uint32_t node_count() const noexcept { return static_cast<uint32_t>(nodes_.size()); }

  // Internal accessors for Yoga C callbacks (resolved via YGConfigGetContext + node_id)
  const ShadowNodeEntry* find_node(uint32_t id) const noexcept { return nodes_.find(id); }
  ShadowNodeEntry* find_node(uint32_t id) noexcept { return nodes_.find(id); }
  const TextMeasurerCallback& text_measurer() const noexcept { return text_measurer_; }

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
  void attach_rn_uimanager(std::shared_ptr<facebook::react::UIManager> ui_manager);
  void detach_rn_uimanager();
#endif

 private:
  // Shadow tree helpers (`shadow_tree.cpp`)
  void destroy_node_if_exists(uint32_t id);
  void mark_node_scene_dirty(ShadowNodeEntry& entry, uint8_t mask = kSceneDirtyAll);
  void mark_layout_dirty_upward(uint32_t id);
  void propagate_text_styles();
  void flush_custom_component_props();

  // Flexbox layout engine (`yoga_layout.cpp` & `flexbox_solver.cpp`)
  void init_node_yoga(ShadowNodeEntry& entry);
  void sync_children_yoga(ShadowNodeEntry& parent);
  void run_layout_pass();

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  void readback_yoga_subtree(uint32_t node_id, uint64_t ts, uint32_t depth);
  bool readback_single_yoga_node(ShadowNodeEntry& entry, uint64_t ts);
#else
  void run_standalone_layout_pass(ShadowNodeEntry& root);
  static constexpr float kUnconstrained = -1.0f;

  enum class MeasureMode : uint8_t {
    Unconstrained = 0,
    AtMost = 1,
    Exact = 2,
  };

  struct FlexLineResult {
    uint32_t* live_children{nullptr};
    float* child_main{nullptr};
    float* child_cross{nullptr};
    float* child_x{nullptr};
    float* child_y{nullptr};
    size_t count{0};
    float content_main{0.0f};
    float content_cross{0.0f};
    bool is_baseline_layout{false};
  };

  NveIntrinsicSize measure_text_fallback(
      const ShadowNodeEntry& entry,
      float max_w,
      MeasureMode w_mode) const;
  float compute_node_baseline(
      uint32_t node_id,
      float w,
      float h,
      uint64_t measure_gen,
      uint32_t depth);
  FlexLineResult resolve_flex_line(
      const ShadowNodeEntry& node,
      float inner_w,
      MeasureMode inner_w_mode,
      float inner_h,
      MeasureMode inner_h_mode,
      bool compute_positions,
      uint64_t measure_gen,
      uint32_t depth);
  NveIntrinsicSize measure_intrinsic_size(
      uint32_t node_id,
      float avail_w,
      MeasureMode w_mode,
      float avail_h,
      MeasureMode h_mode,
      uint64_t measure_gen,
      uint32_t depth);
  void compute_fallback_layout(
      uint32_t node_id,
      float x,
      float y,
      float w,
      float h,
      uint64_t ts,
      uint64_t measure_gen,
      bool force_subtree,
      uint32_t depth = 0);
#endif

  // Direct incremental scene publisher (`scene_publisher.cpp`)
  void publish_scene_frame(uint64_t frame_id);
  void upsert_scene_node(ShadowNodeEntry& entry);
  void remove_scene_node(uint32_t node_id);
  void mark_scene_index_dirty(size_t idx);
  void maybe_compact_scene_slabs();
  void compact_scene_text_slab();
  void compact_scene_children_slab();

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
  bool intercept_rn_raw_event(const facebook::react::RawEvent& raw_event);
  void commit_rn_shadow_tree(bool sync_mount);
#endif

  struct TextStyleFrame {
    uint32_t node_id;
    uint32_t inherited_color;
    float inherited_font_size;
    bool has_inherited;
  };

  NveSurfaceConfig config_;
  ShadowNodeTable nodes_;
  std::unordered_map<std::string, CustomComponentDescriptor> custom_components_;
  TextMeasurerCallback text_measurer_;
  uint64_t gen_counter_{0};
  uint64_t measure_epoch_{1};
  uint64_t scene_dirty_epoch_{1};
  bool text_style_dirty_{true};
  bool layout_dirty_{true};
  bool force_full_layout_{true};
  mutable bool scene_pending_publish_{false};

  // Granular per-frame tracking vectors (reused across frames, zero steady-state heap traffic).
  std::vector<uint32_t> frame_dirty_nodes_;
  std::vector<uint32_t> frame_removed_nodes_;
  std::vector<uint32_t> dirty_custom_nodes_;
  std::vector<uint32_t> new_nodes_pending_layout_;
  std::vector<uint32_t> tmp_children_;
  std::vector<TextStyleFrame> text_style_stack_;

  ScenePublisherState scene_;

  mutable std::mutex event_mutex_;
  EventRingWriter event_ring_;
  std::vector<uint8_t> drained_snapshot_;

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  YGConfigRef yg_config_{nullptr};
#else
  FlexScratchArena flex_arena_;
#endif

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
  std::shared_ptr<facebook::react::UIManager> rn_ui_manager_;
  std::shared_ptr<const facebook::react::EventListener> rn_event_listener_;
  MockJsiRecorderRuntime jsi_recorder_;
#endif
};

} // namespace nve
