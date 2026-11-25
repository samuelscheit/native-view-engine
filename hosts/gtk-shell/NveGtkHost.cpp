#include "NveGtkHost.hpp"

#include <algorithm>
#include <atomic>
#include <cctype>
#include <cmath>
#include <cstring>
#include <limits>
#include <unordered_set>
#include <utility>

namespace nve {

namespace {

std::atomic<uint32_t> g_next_gtk_host_id{1};

std::string ascii_lower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

bool read_raw_string(const RawPropMap& props, const char* key, std::string& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  return false;
}

bool read_raw_value_as_string(const RawPropMap& props, const char* key, std::string& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  if (const auto* i = std::get_if<int64_t>(&it->second)) {
    out = std::to_string(*i);
    return true;
  }
  if (const auto* d = std::get_if<double>(&it->second)) {
    if (std::isfinite(*d)) {
      out = std::to_string(*d);
      return true;
    }
    return false;
  }
  if (const auto* b = std::get_if<bool>(&it->second)) {
    out = *b ? "true" : "false";
    return true;
  }
  return false;
}

bool read_raw_bool(const RawPropMap& props, const char* key, bool& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* b = std::get_if<bool>(&it->second)) {
    out = *b;
    return true;
  }
  if (const auto* i = std::get_if<int64_t>(&it->second)) {
    out = (*i != 0);
    return true;
  }
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    const std::string lower = ascii_lower(*s);
    if (lower == "true" || lower == "1" || lower == "yes") {
      out = true;
      return true;
    }
    if (lower == "false" || lower == "0" || lower == "no") {
      out = false;
      return true;
    }
  }
  return false;
}

bool parse_a11y_role(std::string_view raw, GtkA11yRole& out) {
  const std::string lower = ascii_lower(raw);
  if (lower == "button") {
    out = GtkA11yRole::Button;
    return true;
  }
  if (lower == "text" || lower == "statictext" || lower == "label") {
    out = GtkA11yRole::Text;
    return true;
  }
  if (lower == "header" || lower == "heading") {
    out = GtkA11yRole::Header;
    return true;
  }
  if (lower == "textbox" || lower == "textinput" || lower == "search" || lower == "searchbox") {
    out = GtkA11yRole::TextBox;
    return true;
  }
  if (lower == "scrollarea" || lower == "scrollview") {
    out = GtkA11yRole::ScrollArea;
    return true;
  }
  if (lower == "image" || lower == "img") {
    out = GtkA11yRole::Image;
    return true;
  }
  if (lower == "list") {
    out = GtkA11yRole::List;
    return true;
  }
  if (lower == "listitem") {
    out = GtkA11yRole::ListItem;
    return true;
  }
  if (lower == "checkbox" || lower == "switch") {
    out = GtkA11yRole::CheckBox;
    return true;
  }
  if (lower == "link") {
    out = GtkA11yRole::Link;
    return true;
  }
  if (lower == "group" || lower == "region") {
    out = GtkA11yRole::Group;
    return true;
  }
  if (lower == "none" || lower == "presentation") {
    out = GtkA11yRole::None;
    return true;
  }
  return false;
}

GtkA11yRole infer_default_role(const NveRenderNode& node) noexcept {
  if (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u) {
    return GtkA11yRole::Button;
  }
  switch (node.kind) {
    case NVE_NODE_KIND_TEXT:
      return GtkA11yRole::Text;
    case NVE_NODE_KIND_TEXT_INPUT:
      return GtkA11yRole::TextBox;
    case NVE_NODE_KIND_SCROLL_VIEW:
      return GtkA11yRole::ScrollArea;
    case NVE_NODE_KIND_IMAGE:
      return GtkA11yRole::Image;
    default:
      return GtkA11yRole::Group;
  }
}

GtkAccessibleRole to_gtk_accessible_role(GtkA11yRole role) noexcept {
  switch (role) {
    case GtkA11yRole::Button:
      return GTK_ACCESSIBLE_ROLE_BUTTON;
    case GtkA11yRole::Text:
      return GTK_ACCESSIBLE_ROLE_LABEL;
    case GtkA11yRole::Header:
      return GTK_ACCESSIBLE_ROLE_HEADING;
    case GtkA11yRole::TextBox:
      return GTK_ACCESSIBLE_ROLE_TEXT_BOX;
    case GtkA11yRole::ScrollArea:
      return GTK_ACCESSIBLE_ROLE_GENERIC;
    case GtkA11yRole::Image:
      return GTK_ACCESSIBLE_ROLE_IMG;
    case GtkA11yRole::List:
      return GTK_ACCESSIBLE_ROLE_LIST;
    case GtkA11yRole::ListItem:
      return GTK_ACCESSIBLE_ROLE_LIST_ITEM;
    case GtkA11yRole::CheckBox:
      return GTK_ACCESSIBLE_ROLE_CHECKBOX;
    case GtkA11yRole::Link:
      return GTK_ACCESSIBLE_ROLE_LINK;
    case GtkA11yRole::Group:
      return GTK_ACCESSIBLE_ROLE_GROUP;
    case GtkA11yRole::None:
      return GTK_ACCESSIBLE_ROLE_NONE;
  }
  return GTK_ACCESSIBLE_ROLE_GROUP;
}

std::unordered_map<uint32_t, RawPropMap> extract_raw_props_by_node(
    const std::vector<RenderedNodeSnapshot>& snapshots) {
  std::unordered_map<uint32_t, RawPropMap> result;
  for (const auto& snap : snapshots) {
    if (!snap.raw_props.empty()) {
      result.emplace(snap.node_id, snap.raw_props);
    }
  }
  return result;
}

std::string extract_image_resize_mode(const RawPropMap* raw_props) {
  if (raw_props == nullptr) {
    return "cover";
  }
  std::string mode;
  if (read_raw_string(*raw_props, "resizeMode", mode) ||
      read_raw_string(*raw_props, "resize_mode", mode)) {
    return ascii_lower(mode);
  }
  return "cover";
}

#if GTK_CHECK_VERSION(4, 8, 0)
GtkContentFit parse_gtk_content_fit(std::string_view mode) noexcept {
  if (mode == "contain") {
    return GTK_CONTENT_FIT_CONTAIN;
  }
  if (mode == "stretch" || mode == "fill") {
    return GTK_CONTENT_FIT_FILL;
  }
  if (mode == "center") {
    return GTK_CONTENT_FIT_SCALE_DOWN;
  }
  return GTK_CONTENT_FIT_COVER;
}
#endif

GdkTexture* decode_uri_to_gdk_texture(std::string_view uri) {
  if (uri.empty()) {
    return nullptr;
  }

  GError* err = nullptr;
  GdkTexture* tex = nullptr;

  if (uri.size() > 11u && ascii_lower(uri.substr(0, 11)) == "data:image/") {
    const size_t marker = uri.find(";base64,");
    if (marker == std::string_view::npos) {
      return nullptr;
    }
    const std::string b64(uri.substr(marker + 8u));
    if (b64.empty()) {
      return nullptr;
    }
    gsize decoded_len = 0;
    guchar* decoded = g_base64_decode(b64.c_str(), &decoded_len);
    if (decoded == nullptr || decoded_len == 0) {
      g_free(decoded);
      return nullptr;
    }
    GBytes* bytes = g_bytes_new_take(decoded, decoded_len);
    tex = gdk_texture_new_from_bytes(bytes, &err);
    g_bytes_unref(bytes);
  } else if (uri.size() > 7u && ascii_lower(uri.substr(0, 7)) == "file://") {
    const std::string uri_str(uri);
    gchar* filename = g_filename_from_uri(uri_str.c_str(), nullptr, &err);
    if (err != nullptr) {
      g_clear_error(&err);
    }
    if (filename != nullptr) {
      tex = gdk_texture_new_from_filename(filename, &err);
      g_free(filename);
    } else {
      const std::string fallback_path(uri.substr(7u));
      tex = gdk_texture_new_from_filename(fallback_path.c_str(), &err);
    }
  }

  if (err != nullptr) {
    g_clear_error(&err);
  }
  return tex;
}

GtkNodeSemantics extract_node_semantics(
    const NveRenderNode& node,
    std::string_view text,
    const RawPropMap* raw_props) {
  GtkNodeSemantics sem{};
  sem.role = infer_default_role(node);

  if (raw_props != nullptr) {
    std::string role_str;
    if (read_raw_string(*raw_props, "accessibilityRole", role_str) ||
        read_raw_string(*raw_props, "role", role_str)) {
      GtkA11yRole parsed = sem.role;
      if (parse_a11y_role(role_str, parsed)) {
        sem.role = parsed;
        sem.explicit_role = true;
      }
    }

    if (read_raw_string(*raw_props, "accessibilityLabel", sem.label) ||
        read_raw_string(*raw_props, "aria-label", sem.label)) {
      sem.has_explicit_label = !sem.label.empty();
    }

    (void)read_raw_string(*raw_props, "accessibilityHint", sem.hint);

    if (read_raw_value_as_string(*raw_props, "accessibilityValue", sem.value)) {
      sem.has_explicit_value = true;
    }

    if (!read_raw_bool(*raw_props, "accessibilityDisabled", sem.disabled)) {
      (void)read_raw_bool(*raw_props, "disabled", sem.disabled);
    }

    if (!read_raw_string(*raw_props, "testID", sem.test_id)) {
      (void)read_raw_string(*raw_props, "data-testid", sem.test_id);
    }

    (void)read_raw_string(*raw_props, "placeholder", sem.placeholder);
    (void)read_raw_bool(*raw_props, "secureTextEntry", sem.secure_text_entry);
    (void)read_raw_bool(*raw_props, "multiline", sem.multiline);
  }

  if (!sem.has_explicit_label && (node.flags & NVE_NODE_LEAF_TEXT) != 0u && !text.empty()) {
    sem.label.assign(text.data(), text.size());
  }

  sem.gtk_role = to_gtk_accessible_role(sem.role);
  return sem;
}

std::string format_css_float(double value) {
  if (!std::isfinite(value)) {
    return "0";
  }
  char buf[G_ASCII_DTOSTR_BUF_SIZE];
  g_ascii_formatd(buf, sizeof(buf), "%.4f", value);
  std::string out(buf);
  if (out.find('.') != std::string::npos) {
    while (!out.empty() && out.back() == '0') {
      out.pop_back();
    }
    if (!out.empty() && out.back() == '.') {
      out.pop_back();
    }
  }
  if (out == "-0" || out.empty()) {
    return "0";
  }
  return out;
}

std::string format_css_rgba(uint32_t rgba) {
  const RgbaColor c = unpack_rgba(rgba);
  return "rgba(" + std::to_string(static_cast<unsigned>(c.r8)) + ", " +
         std::to_string(static_cast<unsigned>(c.g8)) + ", " +
         std::to_string(static_cast<unsigned>(c.b8)) + ", " +
         format_css_float(static_cast<double>(c.a)) + ")";
}

bool is_leaf_text_node(const NveRenderNode& node) noexcept {
  return (node.flags & NVE_NODE_LEAF_TEXT) != 0u;
}

std::string build_node_css_rule(
    const std::string& selector_class,
    const NveRenderNode& node) {
  std::string decls;
  decls.reserve(192);

  if (node.bg_rgba != 0u) {
    decls += "  background-color: " + format_css_rgba(node.bg_rgba) + ";\n";
  } else if (is_leaf_text_node(node)) {
    decls += "  background-color: transparent;\n";
  }

  if (std::isfinite(node.border_radius) && node.border_radius > 0.0f) {
    decls += "  border-radius: " + format_css_float(static_cast<double>(node.border_radius)) +
             "px;\n";
  }

  if (std::isfinite(node.border_width) && node.border_width > 0.0f &&
      node.border_rgba != 0u) {
    decls += "  border: " + format_css_float(static_cast<double>(node.border_width)) +
             "px solid " + format_css_rgba(node.border_rgba) + ";\n";
  } else if (node.kind != NVE_NODE_KIND_TEXT_INPUT) {
    decls += "  border: 0px none transparent;\n";
  }

  if (node.text_rgba != 0u) {
    decls += "  color: " + format_css_rgba(node.text_rgba) + ";\n";
  }

  if (std::isfinite(node.font_size) && node.font_size > 0.0f) {
    decls += "  font-size: " + format_css_float(static_cast<double>(node.font_size)) + "px;\n";
  }

  decls += "  min-width: 0px;\n  min-height: 0px;\n";

  return "." + selector_class + " {\n" + decls + "}\n";
}

void load_css_provider_string(GtkCssProvider* provider, const std::string& css) {
  if (provider == nullptr) {
    return;
  }
#if GTK_CHECK_VERSION(4, 12, 0)
  gtk_css_provider_load_from_string(provider, css.c_str());
#else
  gtk_css_provider_load_from_data(provider, css.data(), static_cast<gssize>(css.size()));
#endif
}

void unparent_all_fixed_children(GtkWidget* fixed_widget) {
  if (fixed_widget == nullptr || !GTK_IS_FIXED(fixed_widget)) {
    return;
  }
  while (GtkWidget* child = gtk_widget_get_first_child(fixed_widget)) {
    gtk_fixed_remove(GTK_FIXED(fixed_widget), child);
  }
}

void detach_from_parent(GtkWidget* widget) {
  if (widget == nullptr) {
    return;
  }
  GtkWidget* parent = gtk_widget_get_parent(widget);
  if (parent == nullptr) {
    return;
  }
  if (GTK_IS_FIXED(parent)) {
    gtk_fixed_remove(GTK_FIXED(parent), widget);
  } else {
    gtk_widget_unparent(widget);
  }
}

} // namespace

// -----------------------------------------------------------------------------
// GtkTextureLruCache
// -----------------------------------------------------------------------------

GtkTextureLruCache::GtkTextureLruCache(size_t max_entries)
    : max_entries_(std::max<size_t>(1u, max_entries)) {}

GtkTextureLruCache::~GtkTextureLruCache() {
  clear();
}

GdkTexture* GtkTextureLruCache::get(std::string_view uri) {
  if (uri.empty()) {
    return nullptr;
  }
  const std::string key(uri);
  auto it = entries_.find(key);
  if (it != entries_.end()) {
    lru_.splice(lru_.begin(), lru_, it->second.lru_it);
    it->second.lru_it = lru_.begin();
    ++hits_;
    return it->second.texture;
  }

  ++misses_;
  GdkTexture* decoded = decode_uri_to_gdk_texture(uri);
  if (decoded == nullptr) {
    return nullptr;
  }

  while (entries_.size() >= max_entries_ && !lru_.empty()) {
    const std::string& victim = lru_.back();
    auto vit = entries_.find(victim);
    if (vit != entries_.end()) {
      if (vit->second.texture != nullptr) {
        g_object_unref(vit->second.texture);
      }
      entries_.erase(vit);
    }
    lru_.pop_back();
  }

  lru_.push_front(key);
  Entry entry{};
  entry.texture = decoded;
  entry.lru_it = lru_.begin();
  entries_.emplace(key, std::move(entry));
  return decoded;
}

void GtkTextureLruCache::clear() {
  for (auto& [_, entry] : entries_) {
    if (entry.texture != nullptr) {
      g_object_unref(entry.texture);
      entry.texture = nullptr;
    }
  }
  entries_.clear();
  lru_.clear();
}

// -----------------------------------------------------------------------------
// NveGtkHost
// -----------------------------------------------------------------------------

NveGtkHost::NveGtkHost(NveSurfaceConfig config)
    : host_instance_id_(g_next_gtk_host_id.fetch_add(1u, std::memory_order_relaxed)),
      config_(config) {
  if (config_.surface_id == 0u) {
    config_.surface_id = 1u;
  }
  if (!std::isfinite(config_.width) || config_.width <= 0.0f) {
    config_.width = 640.0f;
  }
  if (!std::isfinite(config_.height) || config_.height <= 0.0f) {
    config_.height = 420.0f;
  }
  if (!std::isfinite(config_.scale_factor) || config_.scale_factor <= 0.0f) {
    config_.scale_factor = 1.0f;
  }

  root_widget_ = gtk_fixed_new();
  g_object_ref_sink(root_widget_);
  gtk_widget_set_hexpand(root_widget_, TRUE);
  gtk_widget_set_vexpand(root_widget_, TRUE);
  gtk_widget_add_css_class(root_widget_, "nve-root");
  const std::string root_cls = scoped_root_class();
  gtk_widget_add_css_class(root_widget_, root_cls.c_str());

  viewport_widget_ = gtk_scrolled_window_new();
  g_object_ref_sink(viewport_widget_);
  gtk_scrolled_window_set_policy(
      GTK_SCROLLED_WINDOW(viewport_widget_), GTK_POLICY_EXTERNAL, GTK_POLICY_EXTERNAL);
  gtk_scrolled_window_set_propagate_natural_width(GTK_SCROLLED_WINDOW(viewport_widget_), FALSE);
  gtk_scrolled_window_set_propagate_natural_height(GTK_SCROLLED_WINDOW(viewport_widget_), FALSE);
  gtk_scrolled_window_set_min_content_width(GTK_SCROLLED_WINDOW(viewport_widget_), 1);
  gtk_scrolled_window_set_min_content_height(GTK_SCROLLED_WINDOW(viewport_widget_), 1);
  gtk_scrolled_window_set_has_frame(GTK_SCROLLED_WINDOW(viewport_widget_), FALSE);
  gtk_widget_set_hexpand(viewport_widget_, TRUE);
  gtk_widget_set_vexpand(viewport_widget_, TRUE);
  gtk_widget_add_css_class(viewport_widget_, root_cls.c_str());
  gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(viewport_widget_), root_widget_);

  css_provider_ = gtk_css_provider_new();
  display_ = gtk_widget_get_display(root_widget_);
  if (display_ == nullptr) {
    display_ = gdk_display_get_default();
  }
  if (display_ != nullptr) {
    g_object_ref(display_);
    gtk_style_context_add_provider_for_display(
        display_, GTK_STYLE_PROVIDER(css_provider_), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  }

  tick_callback_id_ =
      gtk_widget_add_tick_callback(viewport_widget_, &NveGtkHost::on_tick_callback, this, nullptr);

  install_pango_text_measurer();
}

NveGtkHost::~NveGtkHost() {
  detach_window_resize();

  if (viewport_widget_ != nullptr && tick_callback_id_ != 0u) {
    gtk_widget_remove_tick_callback(viewport_widget_, tick_callback_id_);
    tick_callback_id_ = 0u;
  }

  syncing_views_ = true;
  reconciler_.clear(adapter_);
  click_bindings_.clear();
  scroll_bindings_.clear();
  entry_bindings_.clear();
  scroll_contents_.clear();
  node_layouts_.clear();
  node_semantics_.clear();
  node_image_srcs_.clear();
  node_image_resize_modes_.clear();
  node_css_.clear();
  texture_cache_.clear();
  session_.reset();

  if (display_ != nullptr && css_provider_ != nullptr) {
    gtk_style_context_remove_provider_for_display(display_, GTK_STYLE_PROVIDER(css_provider_));
  }
  if (css_provider_ != nullptr) {
    g_object_unref(css_provider_);
    css_provider_ = nullptr;
  }
  if (display_ != nullptr) {
    g_object_unref(display_);
    display_ = nullptr;
  }
  if (viewport_widget_ != nullptr) {
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(viewport_widget_), nullptr);
    g_object_unref(viewport_widget_);
    viewport_widget_ = nullptr;
  }
  if (root_widget_ != nullptr) {
    unparent_all_fixed_children(root_widget_);
    g_object_unref(root_widget_);
    root_widget_ = nullptr;
  }
}

void NveGtkHost::install_pango_text_measurer() {
  session_.set_text_measurer(
      [this](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
        return this->measure_text(text, font_size, max_width);
      });
}

NveIntrinsicSize NveGtkHost::measure_text(
    std::string_view text,
    float font_size,
    float max_width) const {
  if (text.empty() || root_widget_ == nullptr) {
    return NveIntrinsicSize{0.0f, 0.0f};
  }

  PangoLayout* layout = gtk_widget_create_pango_layout(root_widget_, nullptr);
  if (layout == nullptr) {
    return NveIntrinsicSize{0.0f, 0.0f};
  }

  const float effective_font_size =
      (std::isfinite(font_size) && font_size > 0.0f) ? font_size : 16.0f;

  PangoContext* ctx = pango_layout_get_context(layout);
  const PangoFontDescription* base_desc =
      (ctx != nullptr) ? pango_context_get_font_description(ctx) : nullptr;
  PangoFontDescription* desc = (base_desc != nullptr)
                                   ? pango_font_description_copy(base_desc)
                                   : pango_font_description_from_string("Sans");
  pango_font_description_set_absolute_size(
      desc, static_cast<double>(effective_font_size) * static_cast<double>(PANGO_SCALE));
  pango_layout_set_font_description(layout, desc);
  pango_font_description_free(desc);

  const int text_len = static_cast<int>(
      std::min<size_t>(text.size(), static_cast<size_t>(std::numeric_limits<int>::max())));
  pango_layout_set_text(layout, text.data(), text_len);

  if (std::isfinite(max_width) && max_width > 0.0f) {
    const double scaled = static_cast<double>(max_width) * static_cast<double>(PANGO_SCALE);
    const int clamped = static_cast<int>(
        std::min(scaled, static_cast<double>(std::numeric_limits<int>::max())));
    pango_layout_set_width(layout, clamped);
    pango_layout_set_wrap(layout, PANGO_WRAP_WORD_CHAR);
  } else {
    pango_layout_set_width(layout, -1);
  }

  PangoRectangle logical{};
  pango_layout_get_pixel_extents(layout, nullptr, &logical);
  g_object_unref(layout);

  return NveIntrinsicSize{
      static_cast<float>(std::max(0, logical.width)),
      static_cast<float>(std::max(0, logical.height)),
  };
}

bool NveGtkHost::attach_interface(NveGetInterfaceFn get_interface, uint32_t surface_id) {
  NveSurfaceConfig next_cfg = config_;
  if (surface_id != 0u) {
    next_cfg.surface_id = surface_id;
  }
  if (!session_.attach_interface(get_interface, next_cfg)) {
    return false;
  }
  config_ = next_cfg;
  reconciler_.clear(adapter_);
  raw_props_initialized_ = false;
  sync_views(/*check_raw_props=*/true);
  (void)session_.drain_events();
  return true;
}

bool NveGtkHost::load_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg,
    uint32_t surface_id) {
  NveSurfaceConfig next_cfg = config_;
  if (surface_id != 0u) {
    next_cfg.surface_id = surface_id;
  }
  if (!session_.load_dylib(path, next_cfg, out_err_code, out_err_msg)) {
    return false;
  }
  config_ = next_cfg;
  reconciler_.clear(adapter_);
  raw_props_initialized_ = false;
  sync_views(/*check_raw_props=*/true);
  (void)session_.drain_events();
  return true;
}

bool NveGtkHost::reload_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  if (!session_.reload_dylib(path, out_err_code, out_err_msg)) {
    return false;
  }
  raw_props_initialized_ = false;
  sync_views(/*check_raw_props=*/true);
  (void)session_.drain_events();
  return true;
}

bool NveGtkHost::replay_snapshot(
    std::string_view path,
    uint32_t expected_schema_version,
    NveFrameStats* out_stats,
    SnapshotStatus* out_status) {
  session_.set_surface_config(config_);
  if (!session_.replay_frame0_snapshot(path, expected_schema_version, out_stats, out_status)) {
    return false;
  }
  reconciler_.clear(adapter_);
  raw_props_initialized_ = false;
  sync_views(/*check_raw_props=*/true);
  return true;
}

bool NveGtkHost::save_snapshot(
    std::string_view path,
    uint32_t schema_version,
    std::string_view meta,
    SnapshotStatus* out_status) const {
  return session_.save_snapshot(path, schema_version, meta, out_status);
}

bool NveGtkHost::apply_props_json(std::string_view props_json) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.update_props_json(props_json, /*auto_render=*/true);
  if (dirty) {
    sync_views(/*check_raw_props=*/true);
  }
  (void)session_.drain_events();
  return dirty;
}

bool NveGtkHost::apply_hot_patch_json(std::string_view patch_json) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.bridge()->apply_hot_patch_json(patch_json, /*auto_render=*/true);
  if (dirty) {
    sync_views(/*check_raw_props=*/true);
  }
  (void)session_.drain_events();
  return dirty;
}

int32_t NveGtkHost::render_frame(NveFrameStats* out_stats) {
  const int32_t rc = session_.render(out_stats);
  if (rc == 0) {
    sync_views(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return rc;
}

uint32_t NveGtkHost::simulate_press(uint32_t node_id, float x, float y) {
  if (!session_.is_valid()) {
    return 0u;
  }
  const uint32_t dirty = session_.dispatch_press(node_id, x, y, /*auto_render=*/true);
  if (dirty != 0u) {
    sync_views(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return dirty;
}

NveDirectEvent NveGtkHost::make_base_event(
    uint32_t node_id,
    EventKind kind,
    uint8_t payload_tag) const {
  NveDirectEvent ev{};
  ev.surface_id = config_.surface_id;
  ev.node_id = node_id;
  ev.timestamp_us = current_timestamp_us();
  ev.kind = static_cast<uint8_t>(kind);
  ev.payload_tag = payload_tag;
  ev.value = make_null_value();
  return ev;
}

uint32_t NveGtkHost::dispatch_single_direct_event(const NveDirectEvent& ev) {
  if (!session_.is_valid()) {
    return 0u;
  }
  const uint32_t dirty =
      session_.dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/true);
  if (dirty != 0u) {
    sync_views(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return dirty;
}

uint32_t NveGtkHost::dispatch_text_change(uint32_t node_id, std::string_view text) {
  NveDirectEvent ev = make_base_event(node_id, EventKind::TextChange, 3u);
  ev.text = make_str_slice(text);
  return dispatch_single_direct_event(ev);
}

uint32_t NveGtkHost::dispatch_submit(uint32_t node_id, std::string_view text) {
  NveDirectEvent ev = make_base_event(node_id, EventKind::Custom, 5u);
  ev.name = make_str_slice("onSubmit");
  ev.text = make_str_slice(text);
  ev.value = make_str_value(text);
  return dispatch_single_direct_event(ev);
}

uint32_t NveGtkHost::dispatch_focus(uint32_t node_id, bool focused) {
  NveDirectEvent ev = make_base_event(node_id, focused ? EventKind::Focus : EventKind::Blur, 0u);
  return dispatch_single_direct_event(ev);
}

uint32_t NveGtkHost::dispatch_scroll(
    uint32_t node_id,
    float offset_x,
    float offset_y,
    float content_width,
    float content_height) {
  NveDirectEvent ev = make_base_event(node_id, EventKind::Scroll, 2u);
  ev.x = offset_x;
  ev.y = offset_y;
  ev.width = content_width;
  ev.height = content_height;
  return dispatch_single_direct_event(ev);
}

std::optional<GtkNodeSemantics> NveGtkHost::accessibility_info_for_node_id(uint32_t node_id) const {
  if (const GtkNodeSemantics* sem = find_semantics(node_id)) {
    return *sem;
  }
  return std::nullopt;
}

void NveGtkHost::set_surface_size(float width, float height, float scale_factor) {
  if (!std::isfinite(width) || !std::isfinite(height) || width <= 0.0f || height <= 0.0f) {
    return;
  }
  config_.width = width;
  config_.height = height;
  if (std::isfinite(scale_factor) && scale_factor > 0.0f) {
    config_.scale_factor = scale_factor;
  }
  session_.set_surface_size(config_.width, config_.height, config_.scale_factor);
  if (session_.active_fabric_host() != nullptr) {
    sync_views(/*check_raw_props=*/false);
  }
}

void NveGtkHost::sync_surface_size_from_widget() {
  GtkWidget* target = (viewport_widget_ != nullptr) ? viewport_widget_ : root_widget_;
  if (target == nullptr) {
    return;
  }
  const int w = gtk_widget_get_width(target);
  const int h = gtk_widget_get_height(target);
  const int scale = std::max(1, gtk_widget_get_scale_factor(target));
  if (w <= 0 || h <= 0) {
    return;
  }
  if (w == last_observed_width_ && h == last_observed_height_ && scale == last_observed_scale_) {
    return;
  }
  last_observed_width_ = w;
  last_observed_height_ = h;
  last_observed_scale_ = scale;
  set_surface_size(
      static_cast<float>(w), static_cast<float>(h), static_cast<float>(scale));
}

void NveGtkHost::attach_window_resize(GtkWindow* window) {
  detach_window_resize();
  if (window == nullptr) {
    return;
  }
  window_binding_.window = window;
  window_binding_.width_handler = g_signal_connect(
      window,
      "notify::default-width",
      gtk_detail::to_gcallback(&NveGtkHost::on_window_size_notify),
      this);
  window_binding_.height_handler = g_signal_connect(
      window,
      "notify::default-height",
      gtk_detail::to_gcallback(&NveGtkHost::on_window_size_notify),
      this);
  window_binding_.destroy_handler = g_signal_connect(
      window, "destroy", gtk_detail::to_gcallback(&NveGtkHost::on_window_destroy), this);
}

void NveGtkHost::detach_window_resize() {
  if (window_binding_.window == nullptr) {
    return;
  }
  if (window_binding_.width_handler != 0u) {
    g_signal_handler_disconnect(window_binding_.window, window_binding_.width_handler);
  }
  if (window_binding_.height_handler != 0u) {
    g_signal_handler_disconnect(window_binding_.window, window_binding_.height_handler);
  }
  if (window_binding_.destroy_handler != 0u) {
    g_signal_handler_disconnect(window_binding_.window, window_binding_.destroy_handler);
  }
  window_binding_ = WindowBinding{};
}

GtkWidget* NveGtkHost::widget_for_node_id(uint32_t node_id) const noexcept {
  if (node_id == NVE_ROOT_NODE_ID) {
    return root_widget_;
  }
  GtkWidget* const* found = reconciler_.find(node_id);
  return (found != nullptr) ? *found : nullptr;
}

uint32_t NveGtkHost::node_count() const noexcept {
  return session_.node_count();
}

void NveGtkHost::set_event_callback(EventCallback callback) {
  session_.set_event_callback(std::move(callback));
}

void NveGtkHost::apply_picture_fit(GtkWidget* picture, const std::string& resize_mode) {
  if (picture == nullptr || !GTK_IS_PICTURE(picture)) {
    return;
  }
#if GTK_CHECK_VERSION(4, 8, 0)
  gtk_picture_set_content_fit(GTK_PICTURE(picture), parse_gtk_content_fit(resize_mode));
#else
  (void)resize_mode;
#endif
}

void NveGtkHost::refresh_raw_props_from_view(const NveSceneView& view) {
  const auto snapshots = session_.snapshot_nodes();
  const auto raw_props = extract_raw_props_by_node(snapshots);

  std::unordered_map<uint32_t, GtkNodeSemantics> next_semantics;
  next_semantics.reserve(view.node_count);
  std::unordered_map<uint32_t, std::string> next_resize_modes;

  for (uint32_t i = 0u; i < view.node_count; ++i) {
    const NveRenderNode& node = view.nodes[i];
    const auto rp_it = raw_props.find(node.node_id);
    const RawPropMap* rp = (rp_it != raw_props.end()) ? &rp_it->second : nullptr;
    const std::string_view text = scene_node_text(view, node);
    GtkNodeSemantics sem = extract_node_semantics(node, text, rp);

    const auto prev_sem_it = node_semantics_.find(node.node_id);
    const bool sem_changed = (prev_sem_it == node_semantics_.end() || prev_sem_it->second != sem);
    next_semantics.emplace(node.node_id, sem);

    std::string mode;
    bool mode_changed = false;
    if (node.kind == NVE_NODE_KIND_IMAGE) {
      mode = extract_image_resize_mode(rp);
      const auto prev_mode_it = node_image_resize_modes_.find(node.node_id);
      mode_changed =
          (prev_mode_it == node_image_resize_modes_.end() || prev_mode_it->second != mode);
      next_resize_modes.emplace(node.node_id, mode);
    }

    if (sem_changed || mode_changed) {
      if (node.node_id == NVE_ROOT_NODE_ID) {
        apply_accessibility_to_widget(root_widget_, sem);
      } else if (GtkWidget** slot = reconciler_.find(node.node_id);
                 slot != nullptr && *slot != nullptr) {
        GtkWidget* w = *slot;
        if (sem_changed) {
          apply_accessibility_to_widget(w, sem);
          if (GTK_IS_ENTRY(w)) {
            gtk_entry_set_placeholder_text(
                GTK_ENTRY(w), sem.placeholder.empty() ? nullptr : sem.placeholder.c_str());
            gtk_entry_set_visibility(GTK_ENTRY(w), sem.secure_text_entry ? FALSE : TRUE);
          }
          const bool interactive =
              !sem.disabled && !is_leaf_text_node(node) &&
              (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u);
          ensure_click_gesture(node.node_id, w, interactive);
        }
        if (mode_changed && GTK_IS_PICTURE(w)) {
          apply_picture_fit(w, mode);
        }
      }
    }
  }

  node_semantics_ = std::move(next_semantics);
  node_image_resize_modes_ = std::move(next_resize_modes);
}

void NveGtkHost::sync_views(bool check_raw_props) {
  NveFabricHostHandle host = session_.active_fabric_host();
  if (host == nullptr) {
    return;
  }

  const NveSceneView view = session_.scene_view();
  if (view.schema_hash != NVE_SCENE_SCHEMA_HASH) {
    return;
  }

  for (uint32_t i = 0u; i < view.removed_count; ++i) {
    const uint32_t rid = view.removed_ids[i];
    node_semantics_.erase(rid);
    node_image_srcs_.erase(rid);
    node_image_resize_modes_.erase(rid);
  }

  bool need_raw_props = check_raw_props || !raw_props_initialized_;
  if (!need_raw_props && view.nodes != nullptr) {
    for (uint32_t i = 0u; i < view.dirty_count; ++i) {
      const uint32_t idx = view.dirty_indices[i];
      if (idx < view.node_count) {
        const uint8_t k = view.nodes[idx].kind;
        if (k == NVE_NODE_KIND_IMAGE || k == NVE_NODE_KIND_TEXT_INPUT) {
          need_raw_props = true;
          break;
        }
      }
    }
  }

  syncing_views_ = true;
  if (need_raw_props) {
    refresh_raw_props_from_view(view);
    raw_props_initialized_ = true;
  }
  (void)reconciler_.sync(view, adapter_);
  flush_css_if_dirty();
  syncing_views_ = false;
}

const GtkNodeSemantics* NveGtkHost::find_semantics(uint32_t node_id) const noexcept {
  const auto it = node_semantics_.find(node_id);
  return (it != node_semantics_.end()) ? &it->second : nullptr;
}

void NveGtkHost::apply_accessibility_to_widget(GtkWidget* widget, const GtkNodeSemantics& sem) {
  if (widget == nullptr) {
    return;
  }
  GtkAccessible* acc = GTK_ACCESSIBLE(widget);
  if (!sem.label.empty()) {
    gtk_accessible_update_property(acc, GTK_ACCESSIBLE_PROPERTY_LABEL, sem.label.c_str(), -1);
  } else {
    gtk_accessible_reset_property(acc, GTK_ACCESSIBLE_PROPERTY_LABEL);
  }

  if (!sem.hint.empty()) {
    gtk_accessible_update_property(
        acc, GTK_ACCESSIBLE_PROPERTY_DESCRIPTION, sem.hint.c_str(), -1);
  } else {
    gtk_accessible_reset_property(acc, GTK_ACCESSIBLE_PROPERTY_DESCRIPTION);
  }

  if (sem.has_explicit_value) {
    gtk_accessible_update_property(
        acc, GTK_ACCESSIBLE_PROPERTY_VALUE_TEXT, sem.value.c_str(), -1);
  } else {
    gtk_accessible_reset_property(acc, GTK_ACCESSIBLE_PROPERTY_VALUE_TEXT);
  }

  gtk_accessible_update_state(acc, GTK_ACCESSIBLE_STATE_DISABLED, sem.disabled ? TRUE : FALSE, -1);
  gtk_widget_set_sensitive(widget, sem.disabled ? FALSE : TRUE);

  if (!sem.test_id.empty()) {
    gtk_widget_set_name(widget, sem.test_id.c_str());
  }
}

NveGtkHost::ScrollBinding* NveGtkHost::ensure_scroll_binding(uint32_t node_id, GtkWidget* widget) {
  if (widget == nullptr || !GTK_IS_SCROLLED_WINDOW(widget)) {
    return nullptr;
  }
  const auto it = scroll_bindings_.find(node_id);
  if (it != scroll_bindings_.end() && it->second != nullptr && it->second->scrolled == widget) {
    return it->second.get();
  }
  remove_scroll_binding(node_id);

  auto binding = std::make_unique<ScrollBinding>();
  binding->host = this;
  binding->node_id = node_id;
  binding->scrolled = widget;
  binding->hadj = gtk_scrolled_window_get_hadjustment(GTK_SCROLLED_WINDOW(widget));
  binding->vadj = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(widget));
  if (binding->hadj != nullptr) {
    binding->h_handler = g_signal_connect(
        binding->hadj,
        "value-changed",
        gtk_detail::to_gcallback(&NveGtkHost::on_scroll_value_changed),
        binding.get());
  }
  if (binding->vadj != nullptr) {
    binding->v_handler = g_signal_connect(
        binding->vadj,
        "value-changed",
        gtk_detail::to_gcallback(&NveGtkHost::on_scroll_value_changed),
        binding.get());
  }

  ScrollBinding* raw = binding.get();
  scroll_bindings_[node_id] = std::move(binding);
  return raw;
}

void NveGtkHost::remove_scroll_binding(uint32_t node_id) {
  const auto it = scroll_bindings_.find(node_id);
  if (it == scroll_bindings_.end()) {
    return;
  }
  if (it->second != nullptr) {
    if (it->second->hadj != nullptr && it->second->h_handler != 0u &&
        g_signal_handler_is_connected(it->second->hadj, it->second->h_handler)) {
      g_signal_handler_disconnect(it->second->hadj, it->second->h_handler);
    }
    if (it->second->vadj != nullptr && it->second->v_handler != 0u &&
        g_signal_handler_is_connected(it->second->vadj, it->second->v_handler)) {
      g_signal_handler_disconnect(it->second->vadj, it->second->v_handler);
    }
  }
  scroll_bindings_.erase(it);
}

NveGtkHost::EntryBinding* NveGtkHost::ensure_entry_binding(uint32_t node_id, GtkWidget* widget) {
  if (widget == nullptr || !GTK_IS_ENTRY(widget)) {
    return nullptr;
  }
  const auto it = entry_bindings_.find(node_id);
  if (it != entry_bindings_.end() && it->second != nullptr && it->second->widget == widget) {
    return it->second.get();
  }
  remove_entry_binding(node_id, widget);

  auto binding = std::make_unique<EntryBinding>();
  binding->host = this;
  binding->node_id = node_id;
  binding->widget = widget;
  binding->changed_handler = g_signal_connect(
      GTK_EDITABLE(widget),
      "changed",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_changed),
      binding.get());
  binding->activate_handler = g_signal_connect(
      GTK_ENTRY(widget),
      "activate",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_activate),
      binding.get());

  GtkEventController* focus_ctrl = gtk_event_controller_focus_new();
  g_signal_connect(
      focus_ctrl,
      "enter",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_focus_enter),
      binding.get());
  g_signal_connect(
      focus_ctrl,
      "leave",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_focus_leave),
      binding.get());
  gtk_widget_add_controller(widget, focus_ctrl);
  binding->focus_controller = focus_ctrl;

  EntryBinding* raw = binding.get();
  entry_bindings_[node_id] = std::move(binding);
  return raw;
}

void NveGtkHost::remove_entry_binding(uint32_t node_id, GtkWidget* widget) {
  const auto it = entry_bindings_.find(node_id);
  if (it == entry_bindings_.end()) {
    return;
  }
  if (it->second != nullptr) {
    GtkWidget* target = (widget != nullptr) ? widget : it->second->widget;
    if (target != nullptr) {
      if (it->second->changed_handler != 0u &&
          g_signal_handler_is_connected(target, it->second->changed_handler)) {
        g_signal_handler_disconnect(target, it->second->changed_handler);
      }
      if (it->second->activate_handler != 0u &&
          g_signal_handler_is_connected(target, it->second->activate_handler)) {
        g_signal_handler_disconnect(target, it->second->activate_handler);
      }
      if (it->second->focus_controller != nullptr) {
        gtk_widget_remove_controller(target, it->second->focus_controller);
      }
    }
  }
  entry_bindings_.erase(it);
}

LayoutRect NveGtkHost::layout_for_node(uint32_t node_id) const noexcept {
  const auto it = node_layouts_.find(node_id);
  return (it != node_layouts_.end()) ? it->second : LayoutRect{};
}

std::string NveGtkHost::scoped_node_class(uint32_t node_id) const {
  return "nve-host-" + std::to_string(host_instance_id_) + "-node-" + std::to_string(node_id);
}

std::string NveGtkHost::scoped_root_class() const {
  return "nve-host-" + std::to_string(host_instance_id_) + "-root";
}

void NveGtkHost::set_node_css(uint32_t node_id, std::string rule) {
  auto it = node_css_.find(node_id);
  if (it != node_css_.end() && it->second == rule) {
    return;
  }
  node_css_[node_id] = std::move(rule);
  css_dirty_ = true;
}

void NveGtkHost::erase_node_css(uint32_t node_id) {
  if (node_css_.erase(node_id) > 0u) {
    css_dirty_ = true;
  }
}

void NveGtkHost::flush_css_if_dirty() {
  if (!css_dirty_) {
    return;
  }
  std::vector<uint32_t> sorted_ids;
  sorted_ids.reserve(node_css_.size());
  for (const auto& [nid, _] : node_css_) {
    sorted_ids.push_back(nid);
  }
  std::sort(sorted_ids.begin(), sorted_ids.end());

  std::string combined;
  combined.reserve(root_css_.size() + sorted_ids.size() * 160u);
  combined += root_css_;
  for (uint32_t nid : sorted_ids) {
    combined += node_css_[nid];
  }

  active_css_ = std::move(combined);
  load_css_provider_string(css_provider_, active_css_);
  css_dirty_ = false;
}

void NveGtkHost::ensure_click_gesture(uint32_t node_id, GtkWidget* widget, bool interactive) {
  if (!interactive) {
    remove_click_gesture(node_id, widget);
    return;
  }
  const auto it = click_bindings_.find(node_id);
  if (it != click_bindings_.end() && it->second != nullptr && it->second->gesture != nullptr) {
    return;
  }
  auto binding = std::make_unique<ClickBinding>();
  binding->host = this;
  binding->node_id = node_id;
  GtkGesture* click = gtk_gesture_click_new();
  binding->gesture = click;
  g_signal_connect(
      click, "released", gtk_detail::to_gcallback(&NveGtkHost::on_gesture_released), binding.get());
  gtk_widget_add_controller(widget, GTK_EVENT_CONTROLLER(click));
  click_bindings_[node_id] = std::move(binding);
}

void NveGtkHost::remove_click_gesture(uint32_t node_id, GtkWidget* widget) {
  const auto it = click_bindings_.find(node_id);
  if (it == click_bindings_.end()) {
    return;
  }
  if (widget != nullptr && it->second != nullptr && it->second->gesture != nullptr) {
    gtk_widget_remove_controller(widget, GTK_EVENT_CONTROLLER(it->second->gesture));
  }
  click_bindings_.erase(it);
}

void NveGtkHost::GtkViewAdapter::update_root(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
  host->node_layouts_[NVE_ROOT_NODE_ID] = LayoutRect{node.x, node.y, node.width, node.height};
  std::string next_root_css;
  if (node.bg_rgba != 0u) {
    next_root_css = "." + host->scoped_root_class() + " {\n  background-color: " +
                    format_css_rgba(node.bg_rgba) + ";\n}\n";
  }
  if (host->root_css_ != next_root_css) {
    host->root_css_ = std::move(next_root_css);
    host->css_dirty_ = true;
  }
  if (const GtkNodeSemantics* sem = host->find_semantics(node.node_id)) {
    host->apply_accessibility_to_widget(host->root_widget_, *sem);
  }
}

GtkWidget* NveGtkHost::GtkViewAdapter::create_view(
    const NveSceneView& view,
    const NveRenderNode& node) {
  const std::string_view text = scene_node_text(view, node);
  const GtkNodeSemantics default_sem = extract_node_semantics(node, text, nullptr);
  const GtkNodeSemantics& sem =
      host->find_semantics(node.node_id) ? *host->find_semantics(node.node_id) : default_sem;

  GtkWidget* widget = nullptr;
  if (is_leaf_text_node(node)) {
    widget = GTK_WIDGET(g_object_new(
        GTK_TYPE_LABEL, "label", "", "accessible-role", sem.gtk_role, nullptr));
    gtk_label_set_wrap(GTK_LABEL(widget), TRUE);
    gtk_label_set_wrap_mode(GTK_LABEL(widget), PANGO_WRAP_WORD_CHAR);
    gtk_label_set_xalign(GTK_LABEL(widget), 0.0f);
    gtk_label_set_yalign(GTK_LABEL(widget), 0.0f);
    gtk_widget_set_can_target(widget, FALSE);
  } else if (node.kind == NVE_NODE_KIND_IMAGE) {
    widget = GTK_WIDGET(g_object_new(
        GTK_TYPE_PICTURE, "accessible-role", sem.gtk_role, nullptr));
    gtk_picture_set_can_shrink(GTK_PICTURE(widget), TRUE);
  } else if (node.kind == NVE_NODE_KIND_SCROLL_VIEW) {
    widget = GTK_WIDGET(
        g_object_new(GTK_TYPE_SCROLLED_WINDOW, "accessible-role", sem.gtk_role, nullptr));
    GtkWidget* inner_fixed = gtk_fixed_new();
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(widget), inner_fixed);
    host->scroll_contents_[widget] = inner_fixed;
    (void)host->ensure_scroll_binding(node.node_id, widget);
  } else if (node.kind == NVE_NODE_KIND_TEXT_INPUT) {
    widget = GTK_WIDGET(g_object_new(GTK_TYPE_ENTRY, "accessible-role", sem.gtk_role, nullptr));
  } else {
    widget = GTK_WIDGET(g_object_new(GTK_TYPE_FIXED, "accessible-role", sem.gtk_role, nullptr));
  }

  g_object_ref_sink(widget);
  const std::string generic_cls = "nve-node-" + std::to_string(node.node_id);
  const std::string scoped_cls = host->scoped_node_class(node.node_id);
  gtk_widget_add_css_class(widget, generic_cls.c_str());
  gtk_widget_add_css_class(widget, scoped_cls.c_str());
  return widget;
}

void NveGtkHost::GtkViewAdapter::update_view(
    const NveSceneView& view,
    GtkWidget*& widget,
    const NveRenderNode& node) {
  if (widget == nullptr) {
    return;
  }
  ++host->reconcile_update_count_;

  const std::string_view text = scene_node_text(view, node);
  const GtkNodeSemantics default_sem = extract_node_semantics(node, text, nullptr);
  const GtkNodeSemantics& sem =
      host->find_semantics(node.node_id) ? *host->find_semantics(node.node_id) : default_sem;

  host->node_layouts_[node.node_id] = LayoutRect{node.x, node.y, node.width, node.height};

  const int req_w = (std::isfinite(node.width) && node.width > 0.0f)
                        ? static_cast<int>(std::lround(node.width))
                        : -1;
  const int req_h = (std::isfinite(node.height) && node.height > 0.0f)
                        ? static_cast<int>(std::lround(node.height))
                        : -1;
  gtk_widget_set_size_request(widget, req_w, req_h);

  const double clamped_opacity = static_cast<double>(node.opacity_u8) / 255.0;
  gtk_widget_set_opacity(widget, clamped_opacity);

  if (GTK_IS_LABEL(widget)) {
    const char* current = gtk_label_get_text(GTK_LABEL(widget));
    if (current == nullptr || text != std::string_view(current)) {
      const std::string owned(text);
      gtk_label_set_text(GTK_LABEL(widget), owned.c_str());
    }
    gtk_widget_set_can_target(widget, FALSE);
  } else if (GTK_IS_PICTURE(widget)) {
    const auto mode_it = host->node_image_resize_modes_.find(node.node_id);
    const std::string mode = (mode_it != host->node_image_resize_modes_.end())
                                 ? mode_it->second
                                 : std::string("cover");
    host->apply_picture_fit(widget, mode);

    const bool clip = std::isfinite(node.border_radius) && node.border_radius > 0.0f;
    gtk_widget_set_overflow(widget, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);

    const auto src_it = host->node_image_srcs_.find(node.node_id);
    if (src_it == host->node_image_srcs_.end() || src_it->second != text) {
      host->node_image_srcs_[node.node_id] = std::string(text);
      GdkTexture* tex = host->texture_cache_.get(text);
      gtk_picture_set_paintable(
          GTK_PICTURE(widget), tex != nullptr ? GDK_PAINTABLE(tex) : nullptr);
    }
  } else if (GTK_IS_ENTRY(widget)) {
    EntryBinding* binding = host->ensure_entry_binding(node.node_id, widget);
    gtk_entry_set_placeholder_text(
        GTK_ENTRY(widget), sem.placeholder.empty() ? nullptr : sem.placeholder.c_str());
    gtk_entry_set_visibility(GTK_ENTRY(widget), sem.secure_text_entry ? FALSE : TRUE);

    if (binding != nullptr && !binding->user_editing) {
      const char* current = gtk_editable_get_text(GTK_EDITABLE(widget));
      const std::string_view current_sv =
          (current != nullptr) ? std::string_view(current) : std::string_view{};
      if (!binding->has_synced_snapshot ||
          (text != binding->last_snapshot_text && text != current_sv)) {
        binding->updating_programmatically = true;
        const std::string owned(text);
        gtk_editable_set_text(GTK_EDITABLE(widget), owned.c_str());
        binding->last_dispatched_text = owned;
        binding->updating_programmatically = false;
      }
      binding->last_snapshot_text.assign(text.data(), text.size());
      binding->has_synced_snapshot = true;
    }
  } else if (GTK_IS_FIXED(widget)) {
    const bool clip = std::isfinite(node.border_radius) && node.border_radius > 0.0f;
    gtk_widget_set_overflow(widget, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
  } else if (GTK_IS_SCROLLED_WINDOW(widget)) {
    (void)host->ensure_scroll_binding(node.node_id, widget);
    const bool clip = std::isfinite(node.border_radius) && node.border_radius > 0.0f;
    gtk_widget_set_overflow(widget, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
  }

  host->apply_accessibility_to_widget(widget, sem);

  const bool interactive =
      !sem.disabled && !is_leaf_text_node(node) &&
      (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u);
  host->ensure_click_gesture(node.node_id, widget, interactive);

  host->set_node_css(
      node.node_id, build_node_css_rule(host->scoped_node_class(node.node_id), node));
}

void NveGtkHost::GtkViewAdapter::destroy_view(uint32_t node_id, GtkWidget*& view) {
  host->remove_click_gesture(node_id, view);
  host->remove_scroll_binding(node_id);
  host->remove_entry_binding(node_id, view);
  host->node_layouts_.erase(node_id);
  host->node_semantics_.erase(node_id);
  host->node_image_srcs_.erase(node_id);
  host->node_image_resize_modes_.erase(node_id);
  host->erase_node_css(node_id);

  if (view == nullptr) {
    return;
  }

  auto scroll_it = host->scroll_contents_.find(view);
  if (scroll_it != host->scroll_contents_.end()) {
    unparent_all_fixed_children(scroll_it->second);
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(view), nullptr);
    host->scroll_contents_.erase(scroll_it);
  } else if (GTK_IS_FIXED(view)) {
    unparent_all_fixed_children(view);
  }

  detach_from_parent(view);
  g_object_unref(view);
  view = nullptr;
}

void NveGtkHost::sync_children_impl(
    uint32_t parent_id,
    GtkWidget** parent_view,
    const std::vector<std::pair<uint32_t, GtkWidget*>>& resolved_children) {
  GtkWidget* container = nullptr;
  bool is_scroll_container = false;

  if (parent_id == NVE_ROOT_NODE_ID) {
    container = root_widget_;
  } else if (parent_view != nullptr && *parent_view != nullptr) {
    GtkWidget* parent = *parent_view;
    auto scroll_it = scroll_contents_.find(parent);
    if (scroll_it != scroll_contents_.end()) {
      container = scroll_it->second;
      is_scroll_container = true;
    } else if (GTK_IS_FIXED(parent)) {
      container = parent;
    }
  }

  if (container == nullptr || !GTK_IS_FIXED(container)) {
    return;
  }

  std::unordered_set<GtkWidget*> desired_set;
  desired_set.reserve(resolved_children.size());
  for (const auto& [_, child] : resolved_children) {
    desired_set.insert(child);
  }

  GtkWidget* existing = gtk_widget_get_first_child(container);
  while (existing != nullptr) {
    GtkWidget* next = gtk_widget_get_next_sibling(existing);
    if (desired_set.find(existing) == desired_set.end()) {
      gtk_fixed_remove(GTK_FIXED(container), existing);
    }
    existing = next;
  }

  float content_max_w = 0.0f;
  float content_max_h = 0.0f;
  GtkWidget* prev_sibling = nullptr;
  std::unordered_set<GtkWidget*> mounted_this_pass;
  mounted_this_pass.reserve(resolved_children.size());

  for (const auto& [cid, child] : resolved_children) {
    if (!mounted_this_pass.insert(child).second) {
      continue;
    }
    const LayoutRect rect = layout_for_node(cid);
    const double x = std::isfinite(rect.x) ? static_cast<double>(rect.x) : 0.0;
    const double y = std::isfinite(rect.y) ? static_cast<double>(rect.y) : 0.0;

    if (std::isfinite(rect.x) && std::isfinite(rect.width)) {
      content_max_w = std::max(content_max_w, rect.x + rect.width);
    }
    if (std::isfinite(rect.y) && std::isfinite(rect.height)) {
      content_max_h = std::max(content_max_h, rect.y + rect.height);
    }

    GtkWidget* current_parent = gtk_widget_get_parent(child);
    if (current_parent != container) {
      detach_from_parent(child);
      gtk_fixed_put(GTK_FIXED(container), child, x, y);
    } else {
      gtk_fixed_move(GTK_FIXED(container), child, x, y);
    }

    if (gtk_widget_get_prev_sibling(child) != prev_sibling) {
      gtk_widget_insert_after(child, container, prev_sibling);
      gtk_fixed_move(GTK_FIXED(container), child, x, y);
    }
    prev_sibling = child;
  }

  if (is_scroll_container) {
    const LayoutRect parent_rect = layout_for_node(parent_id);
    const float eff_w = std::max(content_max_w, parent_rect.width);
    const float eff_h = std::max(content_max_h, parent_rect.height);
    const int cw = static_cast<int>(std::lround(eff_w));
    const int ch = static_cast<int>(std::lround(eff_h));
    gtk_widget_set_size_request(container, cw > 0 ? cw : -1, ch > 0 ? ch : -1);

    const auto sbit = scroll_bindings_.find(parent_id);
    if (sbit != scroll_bindings_.end() && sbit->second != nullptr) {
      ScrollBinding& sb = *sbit->second;
      sb.content_width = eff_w;
      sb.content_height = eff_h;
      const double view_w = std::max<double>(0.0, static_cast<double>(parent_rect.width));
      const double view_h = std::max<double>(0.0, static_cast<double>(parent_rect.height));
      if (sb.hadj != nullptr) {
        const double cur_x = gtk_adjustment_get_value(sb.hadj);
        gtk_adjustment_configure(
            sb.hadj,
            std::clamp(cur_x, 0.0, std::max(0.0, static_cast<double>(eff_w) - view_w)),
            0.0,
            std::max(static_cast<double>(eff_w), view_w),
            20.0,
            view_w,
            view_w);
      }
      if (sb.vadj != nullptr) {
        const double cur_y = gtk_adjustment_get_value(sb.vadj);
        gtk_adjustment_configure(
            sb.vadj,
            std::clamp(cur_y, 0.0, std::max(0.0, static_cast<double>(eff_h) - view_h)),
            0.0,
            std::max(static_cast<double>(eff_h), view_h),
            20.0,
            view_h,
            view_h);
      }
    }
  }
}

void NveGtkHost::on_gesture_released(
    GtkGestureClick* gesture,
    gint n_press,
    gdouble x,
    gdouble y,
    gpointer user_data) {
  (void)gesture;
  (void)n_press;
  auto* binding = static_cast<ClickBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->simulate_press(node_id, static_cast<float>(x), static_cast<float>(y));
}

void NveGtkHost::on_scroll_value_changed(GtkAdjustment* adjustment, gpointer user_data) {
  (void)adjustment;
  auto* binding = static_cast<ScrollBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr || binding->host->syncing_views_) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  const float ox =
      (binding->hadj != nullptr) ? static_cast<float>(gtk_adjustment_get_value(binding->hadj)) : 0.0f;
  const float oy =
      (binding->vadj != nullptr) ? static_cast<float>(gtk_adjustment_get_value(binding->vadj)) : 0.0f;
  const float cw = std::max(
      binding->content_width,
      (binding->hadj != nullptr) ? static_cast<float>(gtk_adjustment_get_upper(binding->hadj)) : 0.0f);
  const float ch = std::max(
      binding->content_height,
      (binding->vadj != nullptr) ? static_cast<float>(gtk_adjustment_get_upper(binding->vadj)) : 0.0f);
  (void)host->dispatch_scroll(node_id, ox, oy, cw, ch);
}

void NveGtkHost::on_entry_changed(GtkEditable* editable, gpointer user_data) {
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr || binding->updating_programmatically) {
    return;
  }
  const char* raw = gtk_editable_get_text(editable);
  const std::string current = (raw != nullptr) ? std::string(raw) : std::string{};
  if (current == binding->last_dispatched_text) {
    return;
  }
  binding->last_dispatched_text = current;
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  binding->user_editing = true;
  (void)host->dispatch_text_change(node_id, current);
  const auto it = host->entry_bindings_.find(node_id);
  if (it != host->entry_bindings_.end() && it->second != nullptr) {
    it->second->user_editing = false;
  }
}

void NveGtkHost::on_entry_activate(GtkEntry* entry, gpointer user_data) {
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  const char* raw = gtk_editable_get_text(GTK_EDITABLE(entry));
  const std::string current = (raw != nullptr) ? std::string(raw) : std::string{};
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->dispatch_submit(node_id, current);
}

void NveGtkHost::on_entry_focus_enter(GtkEventControllerFocus* controller, gpointer user_data) {
  (void)controller;
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->dispatch_focus(node_id, true);
}

void NveGtkHost::on_entry_focus_leave(GtkEventControllerFocus* controller, gpointer user_data) {
  (void)controller;
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->dispatch_focus(node_id, false);
}

gboolean NveGtkHost::on_tick_callback(
    GtkWidget* widget,
    GdkFrameClock* frame_clock,
    gpointer user_data) {
  (void)widget;
  (void)frame_clock;
  auto* host = static_cast<NveGtkHost*>(user_data);
  if (host != nullptr) {
    host->sync_surface_size_from_widget();
  }
  return G_SOURCE_CONTINUE;
}

void NveGtkHost::on_window_size_notify(
    GObject* object,
    GParamSpec* pspec,
    gpointer user_data) {
  (void)pspec;
  auto* host = static_cast<NveGtkHost*>(user_data);
  if (host == nullptr || object == nullptr) {
    return;
  }
  GtkWindow* win = GTK_WINDOW(object);
  int w = 0;
  int h = 0;
  gtk_window_get_default_size(win, &w, &h);
  if (w > 0 && h > 0) {
    const int scale = std::max(1, gtk_widget_get_scale_factor(GTK_WIDGET(win)));
    host->set_surface_size(
        static_cast<float>(w), static_cast<float>(h), static_cast<float>(scale));
  } else {
    host->sync_surface_size_from_widget();
  }
}

void NveGtkHost::on_window_destroy(GtkWidget* widget, gpointer user_data) {
  (void)widget;
  auto* host = static_cast<NveGtkHost*>(user_data);
  if (host != nullptr) {
    host->window_binding_ = WindowBinding{};
  }
}

} // namespace nve
