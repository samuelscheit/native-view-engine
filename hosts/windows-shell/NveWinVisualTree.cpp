#include "NveWinVisualTree.hpp"

#include <cmath>

namespace nve::win {

namespace {

Rect rect_of(const NveLayoutRect& r) noexcept { return Rect{r.x, r.y, r.width, r.height}; }

/// How far a node's outer shadows paint beyond its rect.
float shadow_reach(std::span<const host::BoxShadow> shadows) noexcept {
  float reach = 0.0f;
  for (const host::BoxShadow& s : shadows) {
    if (!s.inset) {
      reach = std::max(reach, std::max(std::fabs(s.offset_x), std::fabs(s.offset_y)) + s.spread_radius + s.blur_radius);
    }
  }
  return reach;
}

}  // namespace

TextPaint TextPaint::from(const host::TextStyle& s) {
  TextPaint p;
  p.color = s.color;
  p.font_size = s.font_size;
  p.line_height = s.line_height;
  p.letter_spacing = s.letter_spacing;
  p.font_family.assign(s.font_family);
  p.max_lines = s.max_lines;
  p.font_weight = s.font_weight;
  p.font_style = s.font_style;
  p.white_space = s.white_space;
  p.word_break = s.word_break;
  p.overflow_wrap = s.overflow_wrap;
  p.text_align = s.text_align;
  p.text_decoration = s.text_decoration;
  return p;
}

TextPaint TextPaint::from(const NveRuntimeTextStyle& s) {
  host::TextStyle t;
  t.font_size = s.font_size;
  t.line_height = host::LengthValue{s.line_height_unit == NVE_LINE_HEIGHT_POINTS    ? NVE_LENGTH_POINTS
                                    : s.line_height_unit == NVE_LINE_HEIGHT_PERCENT ? NVE_LENGTH_PERCENT
                                                                                    : NVE_LENGTH_AUTO,
                                    s.line_height};
  t.letter_spacing = s.letter_spacing;
  t.font_family = host::str_view(s.font_family);
  t.max_lines = s.max_lines;
  t.font_weight = s.font_weight;
  t.font_style = s.font_style;
  t.white_space = s.white_space;
  t.word_break = s.word_break;
  t.overflow_wrap = s.overflow_wrap;
  return from(t);
}

WinImageSource WinImageSource::from(const host::ImageSource& src) {
  if (src.url.empty() && src.encoded.empty()) return WinImageSource{};
  WinImageSource out;
  out.url.assign(src.url);
  out.encoded.assign(src.encoded.begin(), src.encoded.end());
  out.key = host::image_cache_key(src);
  return out;
}

PaintFrame PaintFrame::child(float sx, float sy, bool parent_clips, const WinNodeVisual& node) const noexcept {
  PaintFrame f;
  f.rect = Rect{rect.x + node.local.x - sx, rect.y + node.local.y - sy, node.local.width, node.local.height};
  f.clip = clip;
  f.clipped = clipped;
  if (parent_clips) {
    f.clip = clipped ? clip.intersect(rect) : rect;
    f.clipped = true;
  }
  const uint8_t vis = node.visual.visibility;
  f.visible = vis == NVE_VISIBILITY_HIDDEN ? false : vis == NVE_VISIBILITY_VISIBLE ? true : visible;
  f.opacity = opacity * node.visual.opacity;
  return f;
}

std::optional<ImagePlacement> place_image(uint8_t fit, const Rect& dst, float iw, float ih) noexcept {
  if (dst.empty() || !(iw > 0.0f) || !(ih > 0.0f) || !std::isfinite(iw) || !std::isfinite(ih)) return std::nullopt;
  const Rect full{0.0f, 0.0f, iw, ih};
  const auto centered = [&](float w, float h) {
    return Rect{dst.x + (dst.width - w) * 0.5f, dst.y + (dst.height - h) * 0.5f, w, h};
  };
  const auto natural = [&] {  // unscaled, centered, cropped to `dst`
    const float w = std::min(dst.width, iw), h = std::min(dst.height, ih);
    return ImagePlacement{centered(w, h), Rect{(iw - w) * 0.5f, (ih - h) * 0.5f, w, h}};
  };
  const auto contain = [&] {
    const float s = std::min(dst.width / iw, dst.height / ih);
    return ImagePlacement{centered(iw * s, ih * s), full};
  };
  switch (fit) {
    case NVE_IMAGE_FIT_FILL: return ImagePlacement{dst, full};
    case NVE_IMAGE_FIT_CONTAIN: return contain();
    case NVE_IMAGE_FIT_NONE: return natural();
    case NVE_IMAGE_FIT_SCALE_DOWN: return iw <= dst.width && ih <= dst.height ? natural() : contain();
    default: {  // cover: fill `dst`, crop the centred source
      const float s = std::max(dst.width / iw, dst.height / ih);
      const float w = std::min(iw, dst.width / s), h = std::min(ih, dst.height / s);
      return ImagePlacement{dst, Rect{(iw - w) * 0.5f, (ih - h) * 0.5f, w, h}};
    }
  }
}

std::vector<ShadowPass> outer_shadow_passes(const host::BoxShadow& s, const Rect& box, float radius) {
  std::vector<ShadowPass> out;
  if (s.inset || (s.color >> 24u) == 0u || box.empty()) return out;
  Rect base = box.inflate(s.spread_radius);
  base.x += s.offset_x;
  base.y += s.offset_y;
  const float r = std::max(0.0f, radius + s.spread_radius);
  const float blur = s.blur_radius;
  if (blur < 0.5f) {
    if (!base.empty()) out.push_back(ShadowPass{base, r, 1.0f});
    return out;
  }
  // Pass k (largest first) spans the edge +-blur; alpha 1/(n-k+1) makes the composited coverage
  // a linear ramp from 0 (outer edge + blur) to 1 (edge - blur), ~ the CSS Gaussian (sigma = blur/2).
  const int n = std::clamp(static_cast<int>(std::ceil(blur / 2.0f)), 2, 8);
  for (int k = 1; k <= n; ++k) {
    const float e = blur * (1.0f - 2.0f * (static_cast<float>(k) - 0.5f) / static_cast<float>(n));
    const Rect pass = base.inflate(e);
    if (pass.empty()) continue;
    out.push_back(ShadowPass{pass, std::max(0.0f, r + e), 1.0f / static_cast<float>(n - k + 1)});
  }
  return out;
}

// --- NveWinVisualTree ------------------------------------------------------------------------

size_t NveWinVisualTree::sync(const host::RuntimeSceneGuard& scene) {
  scene_ = &scene;
  canvas_.bind(scene.reader());
  const size_t n = tree_.sync(scene.reader(), tables_, adapter_);
  apply_scroll_offsets(scene.reader());
  scene_ = nullptr;
  for (const uint32_t parent : resort_) {
    if (parent == root_id_) {
      resort(root_paint_order_, root_children_);
    } else if (WinNodeVisual* p = find_mut(parent)) {
      resort(p->paint_order, p->children);
    }
  }
  resort_.clear();
  settle_damage();
  return n;
}

void NveWinVisualTree::clear() {
  tree_.clear(adapter_);
  root_children_.clear();
  root_paint_order_.clear();
  text_inputs_.clear();
  scrolled_.clear();
  touched_.clear();
  ++touch_pass_;
  resort_.clear();
  root_id_ = 0u;
  root_background_ = 0u;
  full_damage_ = true;
  ++revision_;
}

const WinNodeVisual* NveWinVisualTree::find(uint32_t id) const noexcept {
  return const_cast<NveWinVisualTree*>(this)->find_mut(id);
}

WinNodeVisual* NveWinVisualTree::find_mut(uint32_t id) noexcept {
  Adapter::Handle* h = tree_.find(id);
  return h != nullptr ? h->get() : nullptr;
}

std::span<const uint8_t> NveWinVisualTree::resource(uint32_t id, host::Key64 kind) const noexcept {
  if (scene_ == nullptr || id == 0u) return {};
  const auto res = scene_->resource(id);
  return res && res->kind == kind ? res->bytes : std::span<const uint8_t>{};
}

void NveWinVisualTree::resort(std::vector<uint32_t>& paint_order, std::span<const uint32_t> children) const {
  paint_order.assign(children.begin(), children.end());
  const auto z = [this](uint32_t id) {
    const WinNodeVisual* v = find(id);
    return v != nullptr ? v->visual.z_index : 0;
  };
  const auto by_z = [&z](uint32_t a, uint32_t b) { return z(a) < z(b); };
  if (!std::is_sorted(paint_order.begin(), paint_order.end(), by_z)) {
    std::stable_sort(paint_order.begin(), paint_order.end(), by_z);
  }
}

void NveWinVisualTree::apply_scroll_offsets(const host::SceneReader& scene) {
  bool changed = false;
  const auto apply = [&](WinNodeVisual& v, const NveScrollOffset& s) {
    const float x = std::isfinite(s.x) ? s.x : 0.0f, y = std::isfinite(s.y) ? s.y : 0.0f;
    if (v.scroll_x == x && v.scroll_y == y) return;
    v.scroll_x = x;
    v.scroll_y = y;
    damage_ = damage_.unite(v.bounds);  // the scrolled content is clipped to the container
    touch_descendants(v);               // ... and their bounds moved with it
    changed = true;
  };
  for (const uint32_t id : scrolled_) {  // dropped offsets read {0, 0}
    if (WinNodeVisual* v = find_mut(id)) apply(*v, scene.scroll_offset(id));
  }
  scrolled_.clear();
  for (const NveScrollOffset& s : scene.scroll_offsets()) {
    if (WinNodeVisual* v = find_mut(s.node_id)) {
      apply(*v, s);
      scrolled_.push_back(s.node_id);
    }
  }
  if (changed) ++revision_;
}

void NveWinVisualTree::set_scroll_offset(uint32_t id, float x, float y) {
  WinNodeVisual* v = find_mut(id);
  if (v == nullptr || (v->scroll_x == x && v->scroll_y == y)) return;
  v->scroll_x = x;
  v->scroll_y = y;
  if (std::find(scrolled_.begin(), scrolled_.end(), id) == scrolled_.end()) scrolled_.push_back(id);
  damage_ = damage_.unite(v->bounds);
  touch_descendants(*v);
  settle_damage();  // outside a sync: the bounds are refreshed now
  ++revision_;
}

void NveWinVisualTree::touch(WinNodeVisual& v) {
  if (v.touched_pass == touch_pass_) return;
  v.touched_pass = touch_pass_;
  touched_.push_back(v.id);
}

void NveWinVisualTree::touch_descendants(WinNodeVisual& v) {
  if (v.subtree_pass == touch_pass_) return;
  v.subtree_pass = touch_pass_;
  subtree_.assign(v.children.begin(), v.children.end());
  while (!subtree_.empty()) {
    const uint32_t id = subtree_.back();
    subtree_.pop_back();
    WinNodeVisual* c = find_mut(id);
    if (c == nullptr) continue;
    touch(*c);
    if (c->subtree_pass == touch_pass_) continue;  // an ancestor moved too: already queued
    c->subtree_pass = touch_pass_;
    subtree_.insert(subtree_.end(), c->children.begin(), c->children.end());
  }
}

void NveWinVisualTree::settle_damage() {
  for (const uint32_t id : touched_) {
    WinNodeVisual* v = find_mut(id);
    const auto f = v != nullptr ? frame_of(id) : std::nullopt;
    if (!f) continue;
    const Rect now = f->rect.inflate(v->shadow_reach);
    damage_ = damage_.unite(v->bounds).unite(now);
    v->bounds = now;
  }
  touched_.clear();
  ++touch_pass_;
}

Rect NveWinVisualTree::take_damage() noexcept {
  const Rect out = full_damage_ ? root_rect_.unite(damage_) : damage_;
  damage_ = Rect{};
  full_damage_ = false;
  return out;
}

std::optional<PaintFrame> NveWinVisualTree::frame_of(uint32_t id) const {
  const WinNodeVisual* chain[kMaxDepth];
  uint32_t depth = 0u;
  for (const WinNodeVisual* n = find(id); n != nullptr && depth < kMaxDepth; n = find(n->parent)) chain[depth++] = n;
  if (depth == 0u) return std::nullopt;
  PaintFrame f{root_rect_, {}, false, true, 1.0f};
  float sx = 0.0f, sy = 0.0f;
  bool clips = false;
  while (depth-- > 0u) {
    f = f.child(sx, sy, clips, *chain[depth]);
    sx = chain[depth]->scroll_x;
    sy = chain[depth]->scroll_y;
    clips = chain[depth]->clips();
  }
  return f;
}

const WinNodeVisual* NveWinVisualTree::hit_test(float x, float y) const {
  struct Hit {
    float x, y;
    const WinNodeVisual* top = nullptr;
    bool enter(const WinNodeVisual& n, const PaintFrame& f) {
      if (f.hit(x, y)) top = &n;
      return !n.clips() || f.rect.contains(x, y);  // nothing under a clip outside its box is hit
    }
    void leave(const WinNodeVisual&, const PaintFrame&, bool) {}
  } hit{x, y};
  walk(hit);
  return hit.top;
}

std::pair<float, float> NveWinVisualTree::content_size(const WinNodeVisual& node) const {
  float w = node.local.width, h = node.local.height;
  for (const uint32_t id : node.children) {
    if (const WinNodeVisual* c = find(id)) {
      w = std::max(w, c->local.right());
      h = std::max(h, c->local.bottom());
    }
  }
  return {w, h};
}

// --- RetainedTree adapter --------------------------------------------------------------------

uint32_t NveWinVisualTree::Adapter::tag(const host::NodeData& node) const {
  return static_cast<uint32_t>(node.kind) | (tree->canvas_.has(node.index) ? 0x100u : 0u);
}

NveWinVisualTree::Adapter::Handle NveWinVisualTree::Adapter::create(const host::NodeData& node) {
  auto v = std::make_unique<WinNodeVisual>();
  v->id = node.id;
  v->kind = node.kind;
  v->canvas = tree->canvas_.has(node.index);
  if (v->kind == host::NodeKind::TextInput) tree->text_inputs_.push_back(node.id);
  ++tree->created_;
  return v;
}

void NveWinVisualTree::Adapter::update(Handle& handle, const host::NodeData& node) {
  WinNodeVisual& v = *handle;
  ++tree->updated_;
  ++tree->revision_;
  tree->touch(v);
  if (v.synced && v.visual.z_index != node.visual.z_index) tree->resort_.push_back(node.parent);
  // A visibility / opacity change repaints descendants that may overflow this node's bounds.
  if (v.synced && !v.children.empty() &&
      (v.visual.opacity != node.visual.opacity || v.visual.visibility != node.visual.visibility)) {
    tree->full_damage_ = true;
  }
  const bool reparented = v.synced && v.parent != node.parent;
  v.parent = node.parent;
  const Rect local = rect_of(node.local);
  // Descendants move with a container's origin (or its new parent); their cached damage bounds
  // must follow.
  if (reparented || (v.synced && (local.x != v.local.x || local.y != v.local.y))) tree->touch_descendants(v);
  v.local = local;
  v.content = rect_of(node.content);
  // Resources are re-read when their id changed or the frame republished them (same id, new bytes).
  if (tree->reread(v, v.shadows_res, node.visual.box_shadows)) {
    v.shadows_res = node.visual.box_shadows;
    auto decoded = host::decode_box_shadows(tree->resource(node.visual.box_shadows, host::kBoxShadowsKind));
    v.shadows = decoded ? std::move(*decoded) : std::vector<host::BoxShadow>{};
    v.shadow_reach = shadow_reach(v.shadows);
  }
  v.visual = node.visual;
  v.text_style = TextPaint::from(node.text_style);
  v.text.assign(node.text);
  if (v.kind == host::NodeKind::Image) {
    v.image_fit = node.image.fit;
    if (tree->reread(v, v.image_res, node.image.src)) {
      v.image_res = node.image.src;
      const auto src = host::decode_image_src(tree->resource(node.image.src, host::kImageSrcKind));
      v.image = src ? WinImageSource::from(*src) : WinImageSource{};
    }
  }
  if (const uint32_t list = v.canvas ? tree->canvas_.at(node.index).draw_list : 0u; v.canvas && tree->reread(v, v.canvas_res, list)) {
    v.canvas_res = list;
    const std::span<const uint8_t> bytes = tree->resource(list, canvas::kDrawListKind);
    if (!v.synced || !std::ranges::equal(bytes, v.draw_list)) {
      v.draw_list.assign(bytes.begin(), bytes.end());
      ++v.redraw_count;
    }
  }
  if (v.kind == host::NodeKind::TextInput) {
    const bool secure = node.input.secure || node.input.input_kind == NVE_INPUT_KIND_PASSWORD;
    v.input = WinTextInput{node.input.events,
                           std::string(node.input.value),
                           std::string(node.input.placeholder),
                           node.input.multiline && !secure,  // a password field is single-line
                           secure,
                           node.input.read_only,
                           node.input.input_kind,
                           node.input.max_length};
  }
  v.scroll = node.scroll;
  v.interaction = node.interaction;
  v.layout_events = node.layout_events;
  v.href.assign(node.href);
  v.synced = true;
}

void NveWinVisualTree::Adapter::move(Handle& handle, const host::NodeGeometry& node) {
  WinNodeVisual& v = *handle;
  const Rect local = rect_of(node.local);
  if (local.x != v.local.x || local.y != v.local.y) tree->touch_descendants(v);  // they move with it
  v.local = local;
  v.content = rect_of(node.content);
  tree->touch(v);
  ++tree->revision_;
}

void NveWinVisualTree::Adapter::update_root(const host::NodeData& node) {
  tree->root_id_ = node.id;
  tree->root_background_ = node.visual.background;
  tree->root_rect_ = rect_of(node.rect);
  tree->full_damage_ = true;
  ++tree->revision_;
}

void NveWinVisualTree::Adapter::destroy(uint32_t id, Handle& handle) {
  if (handle != nullptr) {
    if (handle->kind == host::NodeKind::TextInput) std::erase(tree->text_inputs_, id);
    tree->damage_ = tree->damage_.unite(handle->bounds);
  }
  handle.reset();
  ++tree->destroyed_;
  ++tree->revision_;
}

void NveWinVisualTree::Adapter::set_children(const host::NodeGeometry&, Handle* parent_view,
                                             std::span<Handle* const> kids) {
  std::vector<uint32_t>& out = parent_view != nullptr ? (*parent_view)->children : tree->root_children_;
  out.clear();
  for (Handle* kid : kids) out.push_back((*kid)->id);
  if (parent_view != nullptr) {
    tree->resort((*parent_view)->paint_order, out);
    tree->touch(**parent_view);  // reordered children repaint inside the parent
  } else {
    tree->resort(tree->root_paint_order_, out);
    tree->full_damage_ = true;
  }
  ++tree->revision_;
}

}  // namespace nve::win
