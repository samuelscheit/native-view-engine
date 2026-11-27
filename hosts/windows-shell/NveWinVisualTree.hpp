// Portable retained model of one Windows host surface: `RetainedTree` mirrors the runtime scene
// (O(changed)) into owned `WinNodeVisual`s that the Direct2D painter, hit testing, and the native
// `EDIT` overlays read after the scene guard is released. Platform independent (built and tested on
// every OS); only painting and overlays are Win32. Floats arrive sealed by `read_node` /
// `decode_box_shadows`; nothing here re-validates them.
#pragma once

#include <nve/canvas/canvas.hpp>
#include <nve/host/core_resources.hpp>
#include <nve/host/retained_tree.hpp>
#include <nve/host/runtime_session.hpp>
#include <nve/host/scene_nodes.hpp>

#include <algorithm>
#include <cstdint>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <utility>
#include <vector>

namespace nve::win {

/// Axis-aligned rectangle in surface DIPs.
struct Rect {
  float x = 0.0f;
  float y = 0.0f;
  float width = 0.0f;
  float height = 0.0f;

  [[nodiscard]] float right() const noexcept { return x + width; }
  [[nodiscard]] float bottom() const noexcept { return y + height; }
  [[nodiscard]] bool empty() const noexcept { return !(width > 0.0f && height > 0.0f); }
  [[nodiscard]] bool contains(float px, float py) const noexcept {
    return !empty() && px >= x && px <= right() && py >= y && py <= bottom();
  }
  [[nodiscard]] bool intersects(const Rect& o) const noexcept {
    return !empty() && !o.empty() && x < o.right() && o.x < right() && y < o.bottom() && o.y < bottom();
  }
  [[nodiscard]] Rect intersect(const Rect& o) const noexcept {
    const float l = std::max(x, o.x), t = std::max(y, o.y);
    return Rect{l, t, std::max(0.0f, std::min(right(), o.right()) - l), std::max(0.0f, std::min(bottom(), o.bottom()) - t)};
  }
  /// Smallest rect covering both (an empty side contributes nothing).
  [[nodiscard]] Rect unite(const Rect& o) const noexcept {
    if (o.empty()) return *this;
    if (empty()) return o;
    const float l = std::min(x, o.x), t = std::min(y, o.y);
    return Rect{l, t, std::max(right(), o.right()) - l, std::max(bottom(), o.bottom()) - t};
  }
  [[nodiscard]] Rect inflate(float d) const noexcept { return Rect{x - d, y - d, width + 2.0f * d, height + 2.0f * d}; }
  friend bool operator==(const Rect&, const Rect&) noexcept = default;
};

/// Resolved (inherited) typography with an owned font family.
struct TextPaint {
  uint32_t color = 0xFF000000u;
  float font_size = 16.0f;
  host::LengthValue line_height{NVE_LENGTH_AUTO, 0.0f};
  float letter_spacing = 0.0f;
  std::string font_family;  // empty: platform UI font
  uint32_t max_lines = 0u;
  uint16_t font_weight = 400u;
  uint8_t font_style = NVE_FONT_STYLE_NORMAL;
  uint8_t white_space = NVE_WHITE_SPACE_NORMAL;
  uint8_t text_align = NVE_TEXT_ALIGN_START;
  uint8_t text_decoration = 0u;

  [[nodiscard]] static TextPaint from(const host::TextStyle& s);
  /// The runtime measurer's resolved typography (no colour / alignment / decoration: they do
  /// not change the measured size).
  [[nodiscard]] static TextPaint from(const NveRuntimeTextStyle& s);
  [[nodiscard]] bool wraps() const noexcept {
    return white_space != NVE_WHITE_SPACE_NO_WRAP && white_space != NVE_WHITE_SPACE_PRE;
  }
  friend bool operator==(const TextPaint&, const TextPaint&) = default;
};

/// Owned `nve.res.image_src`: a URL or encoded bytes, plus a stable cache key (the URL, or
/// `bytes:<xxh64>:<len>`).
struct WinImageSource {
  std::string url;
  std::vector<uint8_t> encoded;
  std::string key;

  [[nodiscard]] bool empty() const noexcept { return key.empty(); }
  [[nodiscard]] static WinImageSource from(const host::ImageSource& src);
};

/// Owned `nve.text_input` row (a secure field is single-line).
struct WinTextInput {
  std::string value;
  std::string placeholder;
  bool multiline = false;
  bool secure = false;  // `secure` or `InputKind::Password`
  bool read_only = false;
  uint8_t input_kind = NVE_INPUT_KIND_TEXT;
  uint32_t max_length = 0u;
};

/// One retained node: everything painting, hit testing, and native overlays need, owned.
struct WinNodeVisual {
  uint32_t id = 0u;      // packed `NodeId`
  uint32_t parent = 0u;  // packed `NodeId` (the root's id for top-level nodes)
  host::NodeKind kind = host::NodeKind::View;
  bool canvas = false;
  Rect local;  // relative to the parent's (unscrolled) origin
  host::VisualStyle visual;
  std::vector<host::BoxShadow> shadows;
  float shadow_reach = 0.0f;  // how far outer shadows paint beyond the rect
  TextPaint text_style;
  std::string text;
  uint32_t shadows_res = 0u;  // resource ids last decoded (re-read only when changed or republished)
  uint32_t image_res = 0u;
  uint32_t canvas_res = 0u;
  WinImageSource image;
  uint8_t image_fit = NVE_IMAGE_FIT_COVER;
  std::vector<uint8_t> draw_list;  // `NVE_DRAW1`
  uint64_t redraw_count = 0u;
  WinTextInput input;
  host::ScrollData scroll;
  host::InteractionData interaction;
  std::string href;
  std::vector<uint32_t> children;     // packed `NodeId`s, scene order
  std::vector<uint32_t> paint_order;  // `children` stable-sorted by `z_index`
  Rect bounds;                        // painted area at the last sync (damage tracking)
  float scroll_x = 0.0f;              // published visual shift of the descendants
  float scroll_y = 0.0f;
  bool synced = false;

  [[nodiscard]] bool wants(NveEventKind k) const noexcept {
    return !interaction.disabled && (interaction.event_mask & host::event_bit(k)) != 0u;
  }
  [[nodiscard]] bool scrollable() const noexcept {
    return kind == host::NodeKind::Scroll || visual.overflow_x >= NVE_OVERFLOW_SCROLL ||
           visual.overflow_y >= NVE_OVERFLOW_SCROLL;
  }
  [[nodiscard]] bool clips() const noexcept {
    return scrollable() || visual.overflow_x != NVE_OVERFLOW_VISIBLE || visual.overflow_y != NVE_OVERFLOW_VISIBLE;
  }
};

/// Where (and how) a node paints: absolute scroll-shifted rect, inherited clip, visibility, and
/// accumulated opacity.
struct PaintFrame {
  Rect rect;
  Rect clip;  // valid when `clipped`
  bool clipped = false;
  bool visible = true;  // resolved `visibility`
  float opacity = 1.0f; // accumulated

  /// The frame of `node` under a parent painted at `*this`, scrolled by `(sx, sy)`, clipping
  /// its children when `parent_clips`.
  [[nodiscard]] PaintFrame child(float sx, float sy, bool parent_clips, const WinNodeVisual& node) const noexcept;
  [[nodiscard]] bool hit(float px, float py) const noexcept {
    return visible && opacity > 0.0f && rect.contains(px, py) && (!clipped || clip.contains(px, py));
  }
};

/// Destination / source rectangles of an `iw` x `ih` image drawn into `dst` with `NVE_IMAGE_FIT_*`
/// `fit`; `nullopt` when nothing is drawn.
struct ImagePlacement {
  Rect dst;
  Rect src;
};
[[nodiscard]] std::optional<ImagePlacement> place_image(uint8_t fit, const Rect& dst, float iw, float ih) noexcept;

/// Stepped approximation of an outer box shadow: concentric rounded rects drawn back to front,
/// each with `alpha` times the shadow colour's alpha.
struct ShadowPass {
  Rect rect;
  float radius = 0.0f;
  float alpha = 1.0f;
};
[[nodiscard]] std::vector<ShadowPass> outer_shadow_passes(const host::BoxShadow& s, const Rect& box, float radius);

class NveWinVisualTree {
 public:
  /// Mirrors `scene` (resources are copied while its guard is held). Returns views created or updated.
  size_t sync(const host::RuntimeSceneGuard& scene);
  /// Destroys every view.
  void clear();

  [[nodiscard]] const WinNodeVisual* find(uint32_t id) const noexcept;
  [[nodiscard]] size_t size() const noexcept { return tree_.size(); }
  [[nodiscard]] uint32_t root_id() const noexcept { return root_id_; }
  [[nodiscard]] uint32_t root_background() const noexcept { return root_background_; }
  [[nodiscard]] const Rect& root_rect() const noexcept { return root_rect_; }
  [[nodiscard]] std::span<const uint32_t> root_children() const noexcept { return root_children_; }
  /// Live `TextInput` nodes (any order).
  [[nodiscard]] std::span<const uint32_t> text_inputs() const noexcept { return text_inputs_; }
  /// Bumped on every visible change (views, scroll offsets); hosts repaint when it moves.
  [[nodiscard]] uint64_t revision() const noexcept { return revision_; }
  [[nodiscard]] uint64_t created_count() const noexcept { return created_; }
  [[nodiscard]] uint64_t updated_count() const noexcept { return updated_; }
  [[nodiscard]] uint64_t destroyed_count() const noexcept { return destroyed_; }
  /// Surface area changed since the last call (old and new bounds of every changed view; the
  /// whole surface after structural root changes); empty when nothing needs repainting.
  [[nodiscard]] Rect take_damage() noexcept;

  /// Visits every node in paint order: `auto state = enter(const WinNodeVisual&, const PaintFrame&)`
  /// (`bool(state)` false skips the descendants), then always `leave(node, frame, state)` after
  /// them. Fully transparent subtrees are skipped. Allocation-free (paint order is precomputed).
  template <class Visitor>
  void walk(Visitor&& visitor) const {
    walk_children(PaintFrame{root_rect_, {}, false, true, 1.0f}, 0.0f, 0.0f, false, root_paint_order_, visitor, 0u);
  }

  /// Paint frame of node `id` (`nullopt` when unknown).
  [[nodiscard]] std::optional<PaintFrame> frame_of(uint32_t id) const;
  /// Topmost painted node containing `(x, y)`.
  [[nodiscard]] const WinNodeVisual* hit_test(float x, float y) const;
  /// Nearest ancestor-or-self of `node` matching `pred` (`nullptr` when none).
  template <class Pred>
  [[nodiscard]] const WinNodeVisual* nearest(const WinNodeVisual* node, Pred&& pred) const {
    for (uint32_t hops = 0u; node != nullptr && hops < kMaxDepth; ++hops, node = find(node->parent)) {
      if (pred(*node)) return node;
    }
    return nullptr;
  }
  /// Scroll content extent: the union of the children's rects, at least the container's size.
  [[nodiscard]] std::pair<float, float> content_size(const WinNodeVisual& node) const;
  /// Shows a new scroll offset at once (the runtime republishes the same clamped value).
  void set_scroll_offset(uint32_t id, float x, float y);

  struct Adapter {
    using Handle = std::unique_ptr<WinNodeVisual>;
    NveWinVisualTree* tree = nullptr;
    uint32_t tag(const host::NodeData& node) const;
    Handle create(const host::NodeData& node);
    void update(Handle& view, const host::NodeData& node);
    /// Geometry-only change (`moved_indices`): the parent-relative rect. Absolute rects are never
    /// cached, so a moved container costs one update (its descendants follow in the walk).
    void move(Handle& view, const host::NodeData& node);
    void update_root(const host::NodeData& node);
    void destroy(uint32_t id, Handle& view);
    void set_children(const host::NodeData& parent, Handle* parent_view, std::span<Handle* const> kids);
    /// Canvas draw list: a republish under the same id re-runs `update`.
    uint32_t module_resource(const host::NodeData& node) const { return tree->canvas_.at(node.index).draw_list; }
  };

 private:
  static constexpr uint32_t kMaxDepth = 512u;

  [[nodiscard]] WinNodeVisual* find_mut(uint32_t id) noexcept;
  [[nodiscard]] std::span<const uint8_t> resource(uint32_t id, host::Key64 kind) const noexcept;
  /// Whether a view must re-read resource `id` it last decoded as `prev` (new id, republished
  /// bytes, or a non-incremental sync: `RetainedTree::resource_unchanged`).
  [[nodiscard]] bool reread(const WinNodeVisual& v, uint32_t prev, uint32_t id) const noexcept {
    return !v.synced || !tree_.resource_unchanged(scene_->reader(), id, prev);
  }
  void apply_scroll_offsets(const host::RuntimeSceneReader& scene);
  void resort(std::vector<uint32_t>& paint_order, std::span<const uint32_t> children) const;
  void settle_damage();

  template <class Visitor>
  void walk_children(const PaintFrame& frame, float sx, float sy, bool clips, std::span<const uint32_t> ids,
                     Visitor& visitor, uint32_t depth) const {
    if (depth > kMaxDepth) return;
    for (const uint32_t id : ids) {
      const WinNodeVisual* k = find(id);
      if (k == nullptr) continue;
      const PaintFrame f = frame.child(sx, sy, clips, *k);
      if (f.opacity <= 0.0f) continue;
      auto state = visitor.enter(*k, f);
      if (static_cast<bool>(state)) walk_children(f, k->scroll_x, k->scroll_y, k->clips(), k->paint_order, visitor, depth + 1u);
      visitor.leave(*k, f, state);
    }
  }

  host::SceneTables tables_;
  canvas::CanvasTable canvas_;
  host::RetainedTree<Adapter> tree_;
  Adapter adapter_{this};
  const host::RuntimeSceneGuard* scene_ = nullptr;  // set during `sync`
  uint32_t root_id_ = 0u;
  uint32_t root_background_ = 0u;
  Rect root_rect_;
  std::vector<uint32_t> root_children_;
  std::vector<uint32_t> root_paint_order_;
  std::vector<uint32_t> text_inputs_;
  std::vector<uint32_t> scrolled_;  // nodes with a non-zero published offset
  std::vector<uint32_t> touched_;   // views whose bounds must be recomputed (damage)
  std::vector<uint32_t> resort_;    // parents whose children's z-index changed (0: root)
  Rect damage_;
  bool full_damage_ = false;
  uint64_t revision_ = 0u;
  uint64_t created_ = 0u;
  uint64_t updated_ = 0u;
  uint64_t destroyed_ = 0u;
};

}  // namespace nve::win
