#include "NveGtkHost.hpp"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstring>
#include <limits>
#include <unordered_set>
#include <utility>

namespace nve {

namespace {

std::atomic<uint32_t> g_next_gtk_host_id{1};

std::string format_css_float(double value) {
  if (!std::isfinite(value)) {
    return "0";
  }
  char buf[G_ASCII_DTOSTR_BUF_SIZE];
  g_ascii_formatd(buf, sizeof(buf), "%.4f", value);
  std::string out(buf);
  // Trim trailing fractional zeroes for compact deterministic CSS output.
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

bool is_leaf_text_snapshot(const RenderedNodeSnapshot& snap) noexcept {
  return snap.kind == NVE_COMP_TEXT && snap.children.empty();
}

std::string build_node_css_rule(
    const std::string& selector_class,
    const RenderedNodeSnapshot& snap) {
  std::string decls;
  decls.reserve(192);

  if (snap.background_rgba != 0u) {
    decls += "  background-color: " + format_css_rgba(snap.background_rgba) + ";\n";
  } else if (is_leaf_text_snapshot(snap)) {
    decls += "  background-color: transparent;\n";
  }

  if (std::isfinite(snap.border_radius) && snap.border_radius > 0.0f) {
    decls += "  border-radius: " + format_css_float(static_cast<double>(snap.border_radius)) +
             "px;\n";
  }

  if (std::isfinite(snap.border_width) && snap.border_width > 0.0f &&
      snap.border_color_rgba != 0u) {
    decls += "  border: " + format_css_float(static_cast<double>(snap.border_width)) +
             "px solid " + format_css_rgba(snap.border_color_rgba) + ";\n";
  } else if (snap.kind != NVE_COMP_TEXT_INPUT) {
    decls += "  border: 0px none transparent;\n";
  }

  if (snap.text_color_rgba != 0u) {
    decls += "  color: " + format_css_rgba(snap.text_color_rgba) + ";\n";
  }

  if (std::isfinite(snap.font_size) && snap.font_size > 0.0f) {
    decls += "  font-size: " + format_css_float(static_cast<double>(snap.font_size)) + "px;\n";
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

  // Decouple GtkFixed's child minimum-size requisition from the hosting GtkWindow so the
  // window can always shrink smaller as well as grow larger without showing scrollbars.
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

  reconciler_.clear(adapter_);
  click_bindings_.clear();
  scroll_contents_.clear();
  node_layouts_.clear();
  node_css_.clear();
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
  sync_views();
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
  sync_views();
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
  sync_views();
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
  sync_views();
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
    sync_views();
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
    sync_views();
  }
  (void)session_.drain_events();
  return dirty;
}

int32_t NveGtkHost::render_frame(NveFrameStats* out_stats) {
  const int32_t rc = session_.render(out_stats);
  if (rc == 0) {
    sync_views();
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
    sync_views();
  }
  (void)session_.drain_events();
  return dirty;
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
    sync_views();
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

void NveGtkHost::sync_views() {
  if (session_.active_fabric_host() == nullptr) {
    return;
  }
  const std::vector<RenderedNodeSnapshot> snapshots = session_.snapshot_nodes();
  reconciler_.sync(snapshots, adapter_);
  flush_css_if_dirty();
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

void NveGtkHost::GtkViewAdapter::update_root(const RenderedNodeSnapshot& snap) {
  host->node_layouts_[NVE_ROOT_NODE_ID] = snap.layout;
  std::string next_root_css;
  if (snap.background_rgba != 0u) {
    next_root_css = "." + host->scoped_root_class() + " {\n  background-color: " +
                    format_css_rgba(snap.background_rgba) + ";\n}\n";
  }
  if (host->root_css_ != next_root_css) {
    host->root_css_ = std::move(next_root_css);
    host->css_dirty_ = true;
  }
}

GtkWidget* NveGtkHost::GtkViewAdapter::create_view(const RenderedNodeSnapshot& snap) {
  GtkWidget* widget = nullptr;
  if (is_leaf_text_snapshot(snap)) {
    widget = gtk_label_new("");
    gtk_label_set_wrap(GTK_LABEL(widget), TRUE);
    gtk_label_set_wrap_mode(GTK_LABEL(widget), PANGO_WRAP_WORD_CHAR);
    gtk_label_set_xalign(GTK_LABEL(widget), 0.0f);
    gtk_label_set_yalign(GTK_LABEL(widget), 0.0f);
    gtk_widget_set_can_target(widget, FALSE);
  } else if (snap.kind == NVE_COMP_SCROLL_VIEW) {
    widget = gtk_scrolled_window_new();
    GtkWidget* inner_fixed = gtk_fixed_new();
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(widget), inner_fixed);
    host->scroll_contents_[widget] = inner_fixed;
  } else if (snap.kind == NVE_COMP_TEXT_INPUT) {
    widget = gtk_entry_new();
  } else {
    // NVE_COMP_VIEW, NVE_COMP_PRESSABLE, wrapper NVE_COMP_TEXT, and container kinds
    widget = gtk_fixed_new();
  }

  g_object_ref_sink(widget);
  const std::string generic_cls = "nve-node-" + std::to_string(snap.node_id);
  const std::string scoped_cls = host->scoped_node_class(snap.node_id);
  gtk_widget_add_css_class(widget, generic_cls.c_str());
  gtk_widget_add_css_class(widget, scoped_cls.c_str());
  return widget;
}

void NveGtkHost::GtkViewAdapter::update_view(
    GtkWidget*& view,
    const RenderedNodeSnapshot& snap) {
  if (view == nullptr) {
    return;
  }

  host->node_layouts_[snap.node_id] = snap.layout;

  const int req_w = (std::isfinite(snap.layout.width) && snap.layout.width > 0.0f)
                        ? static_cast<int>(std::lround(snap.layout.width))
                        : -1;
  const int req_h = (std::isfinite(snap.layout.height) && snap.layout.height > 0.0f)
                        ? static_cast<int>(std::lround(snap.layout.height))
                        : -1;
  gtk_widget_set_size_request(view, req_w, req_h);

  const double clamped_opacity =
      std::isfinite(snap.opacity) ? std::clamp(static_cast<double>(snap.opacity), 0.0, 1.0) : 1.0;
  gtk_widget_set_opacity(view, clamped_opacity);

  if (GTK_IS_LABEL(view)) {
    const char* current = gtk_label_get_text(GTK_LABEL(view));
    if (current == nullptr || snap.text != current) {
      gtk_label_set_text(GTK_LABEL(view), snap.text.c_str());
    }
    gtk_widget_set_can_target(view, FALSE);
  } else if (GTK_IS_ENTRY(view)) {
    const char* current = gtk_editable_get_text(GTK_EDITABLE(view));
    if (current == nullptr || snap.text != current) {
      gtk_editable_set_text(GTK_EDITABLE(view), snap.text.c_str());
    }
  } else if (GTK_IS_FIXED(view)) {
    const bool clip = std::isfinite(snap.border_radius) && snap.border_radius > 0.0f;
    gtk_widget_set_overflow(view, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
  } else if (GTK_IS_SCROLLED_WINDOW(view)) {
    const bool clip = std::isfinite(snap.border_radius) && snap.border_radius > 0.0f;
    gtk_widget_set_overflow(view, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
  }

  // Leaf labels explicitly opt out of pointer targeting so presses hit parent pressables.
  const bool interactive =
      !is_leaf_text_snapshot(snap) &&
      (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & kEventMaskPress) != 0u);
  host->ensure_click_gesture(snap.node_id, view, interactive);

  host->set_node_css(
      snap.node_id, build_node_css_rule(host->scoped_node_class(snap.node_id), snap));
}

void NveGtkHost::GtkViewAdapter::destroy_view(uint32_t node_id, GtkWidget*& view) {
  host->remove_click_gesture(node_id, view);
  host->node_layouts_.erase(node_id);
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
    const int cw = static_cast<int>(std::lround(std::max(content_max_w, parent_rect.width)));
    const int ch = static_cast<int>(std::lround(std::max(content_max_h, parent_rect.height)));
    gtk_widget_set_size_request(container, cw > 0 ? cw : -1, ch > 0 ? ch : -1);
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
