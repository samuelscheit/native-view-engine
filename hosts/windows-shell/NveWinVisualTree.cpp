#include "NveWinVisualTree.hpp"

#include <nve/host/xxh64.hpp>

#include <cmath>
#include <cstdio>

namespace nve::win {

namespace {

float unit_opacity(float v) noexcept { return std::isfinite(v) ? std::clamp(v, 0.0f, 1.0f) : 1.0f; }

Rect rect_of(const NveLayoutRect& r) noexcept { return Rect{r.x, r.y, r.width, r.height}; }

WinSemantics semantics_of(const host::NodeData& node) {
  const host::AccessibilityData& a = node.a11y;
  WinSemantics s;
  s.role = host::aria_role(node);
  s.label.assign(host::accessible_label(node));
  s.hint.assign(a.hint);
  s.value_text.assign(a.value_text);
  s.level = a.level;
  s.checked = a.checked;
  s.selected = a.selected;
  s.expanded = a.expanded;
  s.pressed = a.pressed;
  s.value_now = a.value_now;
  s.value_min = a.value_min;
  s.value_max = a.value_max;
  s.disabled = node.interaction.disabled;
  s.hidden = a.hidden;
  return s;
}

}  // namespace

TextPaint TextPaint::from(const host::TextStyle& s) {
  TextPaint p;
  p.color = s.color;
  p.font_size = std::isfinite(s.font_size) && s.font_size > 0.0f ? s.font_size : 16.0f;
  p.line_height = s.line_height;
  p.letter_spacing = std::isfinite(s.letter_spacing) ? s.letter_spacing : 0.0f;
  p.font_family.assign(s.font_family);
  p.max_lines = s.max_lines;
  p.font_weight = static_cast<uint16_t>(std::clamp<unsigned>(s.font_weight, 1u, 1000u));
  p.font_style = s.font_style;
  p.white_space = s.white_space;
  p.text_align = s.text_align;
  p.text_decoration = s.text_decoration;
  return p;
}

WinImageSource WinImageSource::from(const host::ImageSource& src) {
  WinImageSource out;
  if (src.is_url()) {
    out.url.assign(src.url);
    out.key = out.url;
    return out;
  }
  out.encoded.assign(src.encoded.begin(), src.encoded.end());
  char key[48];
  std::snprintf(key, sizeof(key), "bytes:%016llx:%zu", static_cast<unsigned long long>(host::xxh64(src.encoded)),
                src.encoded.size());
  out.key = key;
  return out;
}

PaintFrame PaintFrame::child(const WinNodeVisual& parent, const WinNodeVisual& node) const noexcept {
  PaintFrame f;
  f.rect = Rect{rect.x + node.local.x - parent.scroll_x, rect.y + node.local.y - parent.scroll_y, node.local.width,
                node.local.height};
  f.clip = clip;
  f.clipped = clipped;
  if (parent.clips()) {
    f.clip = clipped ? clip.intersect(rect) : rect;
    f.clipped = true;
  }
  const uint8_t vis = node.visual.visibility;
  f.visible = vis == NVE_VISIBILITY_HIDDEN ? false : vis == NVE_VISIBILITY_VISIBLE ? true : visible;
  f.opacity = opacity * unit_opacity(node.visual.opacity);
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
  const float spread = std::isfinite(s.spread_radius) ? s.spread_radius : 0.0f;
  const float blur = std::isfinite(s.blur_radius) ? std::max(0.0f, s.blur_radius) : 0.0f;
  Rect base = box.inflate(spread);
  base.x += std::isfinite(s.offset_x) ? s.offset_x : 0.0f;
  base.y += std::isfinite(s.offset_y) ? s.offset_y : 0.0f;
  const float r = std::max(0.0f, radius + spread);
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
  return n;
}

void NveWinVisualTree::clear() {
  tree_.clear(adapter_);
  root_children_.clear();
  text_inputs_.clear();
  scrolled_.clear();
  root_id_ = 0u;
  root_background_ = 0u;
  root_rect_ = Rect{};
  root_semantics_ = WinSemantics{};
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

WinNodeVisual NveWinVisualTree::root_view() const {
  WinNodeVisual r;
  r.id = root_id_;
  r.local = root_rect_;
  return r;
}

void NveWinVisualTree::apply_scroll_offsets(const host::RuntimeSceneReader& scene) {
  bool changed = false;
  const auto apply = [&changed](WinNodeVisual& v, const NveScrollOffset& s) {
    const float x = std::isfinite(s.x) ? s.x : 0.0f, y = std::isfinite(s.y) ? s.y : 0.0f;
    if (v.scroll_x == x && v.scroll_y == y) return;
    v.scroll_x = x;
    v.scroll_y = y;
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
  ++revision_;
}

std::optional<PaintFrame> NveWinVisualTree::frame_of(uint32_t id) const {
  std::vector<const WinNodeVisual*> chain;
  for (const WinNodeVisual* n = find(id); n != nullptr && chain.size() < kMaxDepth; n = find(n->parent)) {
    chain.push_back(n);
  }
  if (chain.empty()) return std::nullopt;
  const WinNodeVisual root = root_view();
  const WinNodeVisual* parent = &root;
  PaintFrame f{root_rect_, {}, false, true, 1.0f};
  for (auto it = chain.rbegin(); it != chain.rend(); ++it) {
    f = f.child(*parent, **it);
    parent = *it;
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
    void leave(const WinNodeVisual&, const PaintFrame&) {}
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
  v.parent = node.parent;
  v.local = rect_of(node.local);
  // Resources are re-read on every update: `RetainedTree` updates a view whose resource was
  // republished under the same id, so an id comparison would keep stale bytes.
  auto decoded = host::decode_box_shadows(tree->resource(node.visual.box_shadows, host::kBoxShadowsKind));
  v.shadows = decoded ? std::move(*decoded) : std::vector<host::BoxShadow>{};
  v.visual = node.visual;
  v.text_style = TextPaint::from(node.text_style);
  v.text.assign(node.text);
  if (v.kind == host::NodeKind::Image) {
    v.image_fit = node.image.fit;
    const auto src = host::decode_image_src(tree->resource(node.image.src, host::kImageSrcKind));
    v.image = src ? WinImageSource::from(*src) : WinImageSource{};
  }
  if (v.canvas) {
    const std::span<const uint8_t> bytes = tree->resource(tree->canvas_.at(node.index).draw_list, canvas::kDrawListKind);
    if (!v.synced || !std::ranges::equal(bytes, v.draw_list)) {
      v.draw_list.assign(bytes.begin(), bytes.end());
      ++v.redraw_count;
    }
  }
  if (v.kind == host::NodeKind::TextInput) {
    v.input = WinTextInput{std::string(node.input.value),
                           std::string(node.input.placeholder),
                           node.input.multiline,
                           node.input.secure || node.input.input_kind == NVE_INPUT_KIND_PASSWORD,
                           node.input.read_only,
                           node.input.input_kind,
                           node.input.max_length};
  }
  v.scroll = node.scroll;
  v.interaction = node.interaction;
  v.semantics = semantics_of(node);
  v.href.assign(node.href);
  v.synced = true;
}

void NveWinVisualTree::Adapter::update_root(const host::NodeData& node) {
  tree->root_id_ = node.id;
  tree->root_background_ = node.visual.background;
  tree->root_rect_ = rect_of(node.rect);
  tree->root_semantics_ = semantics_of(node);
  ++tree->revision_;
}

void NveWinVisualTree::Adapter::destroy(uint32_t id, Handle& handle) {
  if (handle != nullptr && handle->kind == host::NodeKind::TextInput) std::erase(tree->text_inputs_, id);
  handle.reset();
  ++tree->destroyed_;
  ++tree->revision_;
}

void NveWinVisualTree::Adapter::set_children(const host::NodeData&, Handle* parent_view, std::span<Handle* const> kids) {
  std::vector<uint32_t>& out = parent_view != nullptr ? (*parent_view)->children : tree->root_children_;
  out.clear();
  for (Handle* kid : kids) out.push_back((*kid)->id);
  ++tree->revision_;
}

}  // namespace nve::win
