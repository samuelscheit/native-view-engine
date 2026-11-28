#include "NveGtkInternal.hpp"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <limits>
#include <utility>

namespace nve::gtk {

namespace {

std::atomic<uint32_t> g_next_host_id{1};

float positive_or(float v, float fallback) noexcept { return std::isfinite(v) && v > 0.0f ? v : fallback; }

/// Pending wake on the GLib main context; dropped silently once its host is gone.
struct WakeTicket {
  std::weak_ptr<std::atomic<bool>> alive;
  NveGtkHost* host;
};

gboolean run_wake(gpointer data) {
  auto* ticket = static_cast<WakeTicket*>(data);
  if (const auto alive = ticket->alive.lock(); alive && alive->load(std::memory_order_acquire)) {
    (void)ticket->host->pump();
  }
  return G_SOURCE_REMOVE;
}

void free_wake(gpointer data) { delete static_cast<WakeTicket*>(data); }

}  // namespace

NveGtkHost::NveGtkHost(GtkHostConfig config)
    : host_instance_id_(g_next_host_id.fetch_add(1u, std::memory_order_relaxed)), config_(config) {
  config_.surface_id = config_.surface_id != 0u ? config_.surface_id : 1u;
  config_.width = positive_or(config_.width, 640.0f);
  config_.height = positive_or(config_.height, 420.0f);
  config_.scale_factor = positive_or(config_.scale_factor, 1.0f);

  NveRuntimeConfig rc = host::RuntimeSession::default_config(config_.surface_id, config_.width, config_.height);
  rc.scale_factor = config_.scale_factor;
  std::string error;
  session_ = host::HostSession::create(rc, &error);
  if (!session_) g_error("nve-gtk-shell: %s", error.c_str());  // libnve_runtime is linked in: a bug

  root_widget_ = gtk_fixed_new();
  g_object_ref_sink(root_widget_);
  gtk_widget_set_hexpand(root_widget_, TRUE);
  gtk_widget_set_vexpand(root_widget_, TRUE);
  gtk_widget_add_css_class(root_widget_, "nve-root");

  viewport_widget_ = gtk_scrolled_window_new();
  g_object_ref_sink(viewport_widget_);
  GtkScrolledWindow* sw = GTK_SCROLLED_WINDOW(viewport_widget_);
  gtk_scrolled_window_set_policy(sw, GTK_POLICY_EXTERNAL, GTK_POLICY_EXTERNAL);
  gtk_scrolled_window_set_propagate_natural_width(sw, FALSE);
  gtk_scrolled_window_set_propagate_natural_height(sw, FALSE);
  gtk_scrolled_window_set_min_content_width(sw, 1);
  gtk_scrolled_window_set_min_content_height(sw, 1);
  gtk_scrolled_window_set_has_frame(sw, FALSE);
  gtk_widget_set_hexpand(viewport_widget_, TRUE);
  gtk_widget_set_vexpand(viewport_widget_, TRUE);
  gtk_scrolled_window_set_child(sw, root_widget_);

  css_provider_ = gtk_css_provider_new();
  display_ = gdk_display_get_default();
  if (display_ != nullptr) {
    g_object_ref(display_);
    gtk_style_context_add_provider_for_display(display_, GTK_STYLE_PROVIDER(css_provider_),
                                               GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  }
  tick_id_ = gtk_widget_add_tick_callback(viewport_widget_, &NveGtkHost::on_tick, this, nullptr);

  (void)session_->runtime().set_text_measurer(&NveGtkHost::measure_trampoline, this);
  install_wake_poster();
}

NveGtkHost::~NveGtkHost() {
  alive_->store(false, std::memory_order_release);
  session_->set_wake_poster({});
  session_->set_event_sink({});
  detach_window_resize();
  if (tick_id_ != 0u) gtk_widget_remove_tick_callback(viewport_widget_, tick_id_);
  syncing_ = true;
  tree_.clear(adapter_);
  (void)session_->runtime().set_text_measurer(nullptr, nullptr);
  session_.reset();
  textures_.clear();
  if (display_ != nullptr) {
    gtk_style_context_remove_provider_for_display(display_, GTK_STYLE_PROVIDER(css_provider_));
    g_object_unref(display_);
  }
  g_object_unref(css_provider_);
  gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(viewport_widget_), nullptr);
  g_object_unref(viewport_widget_);
  if (measure_layout_ != nullptr) g_object_unref(measure_layout_);
  gtk_internal::unparent_children(root_widget_);
  g_object_unref(root_widget_);
}

void NveGtkHost::install_wake_poster() {
  // Runtime wakes (paired frames, timers) arrive on any thread, also the owner thread in the
  // middle of `attach` / `restore_state`: always queue onto the GLib main context, never run
  // inline (`g_main_context_invoke` would, re-entering the session or running off-thread).
  std::weak_ptr<std::atomic<bool>> alive = alive_;
  session_->set_wake_poster([alive, this] {
    GSource* source = g_idle_source_new();
    g_source_set_priority(source, G_PRIORITY_DEFAULT);
    g_source_set_callback(source, &run_wake, new WakeTicket{alive, this}, &free_wake);
    g_source_attach(source, g_main_context_default());
    g_source_unref(source);
  });
}

// --- App sources -------------------------------------------------------------------------------

// Every source change starts a new `scene_epoch`: the next sync is a full pass that keeps views
// whose node still matches (focus survives a hot reload) and destroys the rest.

bool NveGtkHost::attach_interface(NveGetInterfaceFn get_interface) {
  if (!session_->attach(get_interface)) return false;
  (void)pump();
  return true;
}

bool NveGtkHost::load_dylib(std::string_view path, host::ModuleError* out_error, std::string* out_message) {
  if (!session_->load_module(path, out_error, out_message)) return false;
  (void)pump();
  return true;
}

bool NveGtkHost::reload_dylib(std::string_view path, host::ModuleError* out_error, std::string* out_message) {
  if (!session_->reload_module(path, out_error, out_message)) return false;
  (void)pump();
  return true;
}

bool NveGtkHost::connect_paired(std::string_view uri, std::string_view token, std::string* out_error) {
  if (!session_->pair(uri, token, out_error)) return false;
  (void)pump();
  return true;
}

void NveGtkHost::disconnect_paired() {
  session_->unpair();
  (void)pump();
}

bool NveGtkHost::is_paired() const noexcept { return session_->runtime().pair_is_connected(); }

bool NveGtkHost::submit_frame(std::span<const uint8_t> nve2) {
  if (session_->runtime().submit_nve2(nve2) != NVE_RUNTIME_OK) return false;
  (void)pump();
  return true;
}

// --- App state & dev loop ----------------------------------------------------------------------

bool NveGtkHost::save_state(std::string_view path, uint32_t schema_version) {
  return session_->save_state(path, schema_version);
}

bool NveGtkHost::restore_state(std::string_view path, uint32_t schema_version) {
  if (!session_->restore_state_file(path, schema_version)) return false;
  (void)pump();
  return true;
}

bool NveGtkHost::clear_state(std::string_view path) { return session_->clear_state_file(path); }

bool NveGtkHost::apply_props(std::span<const NveMapEntry> props) {
  const bool ok = session_->set_props(props);
  sync_views();
  return ok;
}

bool NveGtkHost::apply_hot_patch(std::span<const uint8_t> patch_nvp1) {
  const bool changed = session_->apply_hot_patch(patch_nvp1);
  sync_views();
  return changed;
}

void NveGtkHost::set_event_callback(EventCallback callback) { session_->set_event_sink(std::move(callback)); }

size_t NveGtkHost::pump() {
  if (syncing_) return 0u;  // re-entrant wake from a widget signal raised during sync
  (void)session_->pump();
  const size_t before = static_cast<size_t>(reconcile_update_count_);
  sync_views();
  return static_cast<size_t>(reconcile_update_count_) - before;
}

void NveGtkHost::sync_views() {
  if (syncing_) return;
  syncing_ = true;
  {
    auto guard = session_->runtime().acquire_scene();
    if (guard) {
      canvas_.bind(guard->reader());
      scene_ = &*guard;
      (void)tree_.sync(guard->reader(), tables_, adapter_);
      scene_ = nullptr;
    }
  }
  update_scroll_extents();
  flush_css();
  syncing_ = false;
}

// --- Input -------------------------------------------------------------------------------------

bool NveGtkHost::dispatch(const NveDirectEvent& ev) {
  if (syncing_) return false;
  const bool changed = session_->dispatch(ev);
  sync_views();
  return changed;
}

bool NveGtkHost::simulate_press(uint32_t node_id, float x, float y) {
  return dispatch(session_->events().press(node_id, x, y));
}

bool NveGtkHost::dispatch_text_change(uint32_t node_id, std::string_view text) {
  const bool changed = dispatch(session_->events().text(node_id, NVE_EVENT_TEXT_CHANGE, text));
  // In-process apps answer synchronously: write back a transformed or rejected edit (a rejecting
  // app republishes nothing, so no reconcile would ever revert the field). Paired answers are
  // async and arrive through `app_published`.
  GtkNodeView* const* found = is_paired() ? nullptr : tree_.find(node_id);  // the dispatch may remove it
  GtkNodeView* v = found != nullptr ? *found : nullptr;
  if (v != nullptr && (v->kind == WidgetKind::Entry || v->kind == WidgetKind::TextView) &&
      v->text.app_answered(v->text.app_value())) {
    const auto write = v->text.host_write();
    if (v->kind == WidgetKind::TextView) {
      gtk_text_buffer_set_text(gtk_text_view_get_buffer(GTK_TEXT_VIEW(v->widget)), v->text.app_value().c_str(), -1);
    } else {
      gtk_editable_set_text(GTK_EDITABLE(v->widget), v->text.app_value().c_str());
    }
  }
  return changed;
}

bool NveGtkHost::dispatch_submit(uint32_t node_id, std::string_view text) {
  return dispatch(session_->events().text(node_id, NVE_EVENT_SUBMIT, text));
}

bool NveGtkHost::dispatch_focus(uint32_t node_id, bool focused) {
  return dispatch(session_->events().focus(node_id, focused));
}

bool NveGtkHost::dispatch_scroll(uint32_t node_id, float offset_x, float offset_y, float content_width,
                                 float content_height) {
  // Adjustments GTK clamps while sync resizes content are not user scrolls: the runtime clamps
  // its own offsets, and mutating it mid-sync would race the scene being applied.
  if (syncing_) return false;
  // Keep the runtime's scroll offset (hit testing, programmatic scroll clamping) in step with GTK.
  (void)session_->runtime().scroll_to(node_id, offset_x, offset_y);
  return dispatch(session_->events().scroll(node_id, offset_x, offset_y, content_width, content_height));
}

// --- Surface -----------------------------------------------------------------------------------

void NveGtkHost::set_surface_size(float width, float height, float scale_factor) {
  if (!std::isfinite(width) || !std::isfinite(height) || width <= 0.0f || height <= 0.0f) return;
  config_.width = width;
  config_.height = height;
  config_.scale_factor = positive_or(scale_factor, config_.scale_factor);
  (void)session_->runtime().set_viewport(config_.width, config_.height, config_.scale_factor);
  (void)pump();
}

void NveGtkHost::sync_surface_size_from_widget() {
  const int w = gtk_widget_get_width(viewport_widget_);
  const int h = gtk_widget_get_height(viewport_widget_);
  const int scale = std::max(1, gtk_widget_get_scale_factor(viewport_widget_));
  if (w <= 0 || h <= 0 || (w == observed_w_ && h == observed_h_ && scale == observed_scale_)) return;
  observed_w_ = w;
  observed_h_ = h;
  observed_scale_ = scale;
  set_surface_size(static_cast<float>(w), static_cast<float>(h), static_cast<float>(scale));
}

void NveGtkHost::attach_window_resize(GtkWindow* window) {
  detach_window_resize();
  if (window == nullptr) return;
  window_binding_.window = window;
  const GCallback notify = gtk_detail::to_gcallback(&NveGtkHost::on_window_size_notify);
  window_binding_.width_handler = g_signal_connect(window, "notify::default-width", notify, this);
  window_binding_.height_handler = g_signal_connect(window, "notify::default-height", notify, this);
  window_binding_.destroy_handler =
      g_signal_connect(window, "destroy", gtk_detail::to_gcallback(&NveGtkHost::on_window_destroy), this);
}

void NveGtkHost::detach_window_resize() {
  if (window_binding_.window == nullptr) return;
  g_signal_handler_disconnect(window_binding_.window, window_binding_.width_handler);
  g_signal_handler_disconnect(window_binding_.window, window_binding_.height_handler);
  g_signal_handler_disconnect(window_binding_.window, window_binding_.destroy_handler);
  window_binding_ = WindowBinding{};
}

std::span<const uint8_t> NveGtkHost::scene_resource(uint32_t id, host::Key64 kind) const noexcept {
  if (scene_ == nullptr || id == 0u) return {};
  const auto res = scene_->resource(id);
  return res && res->kind == kind ? res->bytes : std::span<const uint8_t>{};
}

void NveGtkHost::measure_trampoline(void* ctx, NveStrSlice text, const NveRuntimeTextStyle* style, float max_width,
                                    float* out_width, float* out_height) {
  const NveIntrinsicSize size = static_cast<const NveGtkHost*>(ctx)->measure_text(host::str_view(text), *style, max_width);
  *out_width = size.width;
  *out_height = size.height;
}

NveIntrinsicSize NveGtkHost::measure_text(std::string_view text, const NveRuntimeTextStyle& style,
                                          float max_width) const {
  if (text.empty()) return NveIntrinsicSize{0.0f, 0.0f};
  if (measure_layout_ == nullptr) measure_layout_ = gtk_widget_create_pango_layout(root_widget_, nullptr);
  PangoLayout* layout = measure_layout_;
  pango_layout_context_changed(layout);  // the widget's font settings / scale may have changed
  PangoContext* ctx = pango_layout_get_context(layout);
  const PangoFontDescription* base = ctx != nullptr ? pango_context_get_font_description(ctx) : nullptr;
  PangoFontDescription* desc =
      base != nullptr ? pango_font_description_copy(base) : pango_font_description_from_string("Sans");
  const float font_size = positive_or(style.font_size, 16.0f);
  pango_font_description_set_absolute_size(desc, static_cast<double>(font_size) * PANGO_SCALE);
  pango_font_description_set_weight(desc, static_cast<PangoWeight>(std::clamp<int>(style.font_weight, 100, 1000)));
  pango_font_description_set_style(desc, style.font_style == NVE_FONT_STYLE_ITALIC    ? PANGO_STYLE_ITALIC
                                         : style.font_style == NVE_FONT_STYLE_OBLIQUE ? PANGO_STYLE_OBLIQUE
                                                                                      : PANGO_STYLE_NORMAL);
  if (const std::string family = gtk_internal::font_family_list(host::str_view(style.font_family)); !family.empty()) {
    pango_font_description_set_family(desc, family.c_str());
  }
  pango_layout_set_font_description(layout, desc);
  pango_font_description_free(desc);

  // The label's CSS `letter-spacing` / `line-height` reach Pango as these attributes.
  PangoAttrList* attrs = pango_attr_list_new();
  if (style.letter_spacing != 0.0f && std::isfinite(style.letter_spacing)) {
    pango_attr_list_insert(attrs, pango_attr_letter_spacing_new(static_cast<int>(
                                      std::clamp(static_cast<double>(style.letter_spacing) * PANGO_SCALE, -1e6, 1e6))));
  }
  if (const double lh = host::line_height_points(style.line_height_unit, style.line_height, font_size); lh > 0.0) {
    pango_attr_list_insert(attrs, pango_attr_line_height_new_absolute(static_cast<int>(
                                      std::min(lh * PANGO_SCALE, static_cast<double>(std::numeric_limits<int>::max())))));
  }
  pango_layout_set_attributes(layout, attrs);
  pango_attr_list_unref(attrs);
  pango_layout_set_text(layout, text.data(),
                        static_cast<int>(std::min<size_t>(text.size(), std::numeric_limits<int>::max())));

  // Wrapping, lines, and ellipsizing mirror `gtk_label_set_wrap` / `_lines` / `_ellipsize`.
  const bool wraps = host::white_space_wraps(style.white_space) && std::isfinite(max_width) && max_width >= 0.0f;
  pango_layout_set_width(layout, wraps ? static_cast<int>(std::min(static_cast<double>(max_width) * PANGO_SCALE,
                                                                   static_cast<double>(std::numeric_limits<int>::max())))
                                       : -1);
  pango_layout_set_wrap(layout, PANGO_WRAP_WORD_CHAR);
  const bool limited = style.max_lines > 0u;
  pango_layout_set_ellipsize(layout, limited ? PANGO_ELLIPSIZE_END : PANGO_ELLIPSIZE_NONE);
  pango_layout_set_height(
      layout, limited ? -static_cast<int>(std::min<uint32_t>(style.max_lines, std::numeric_limits<int>::max())) : -1);
  PangoRectangle logical{};
  pango_layout_get_pixel_extents(layout, nullptr, &logical);
  return NveIntrinsicSize{static_cast<float>(std::max(0, logical.width)), static_cast<float>(std::max(0, logical.height))};
}

GtkWidget* NveGtkHost::widget_for_node_id(uint32_t node_id) const noexcept {
  if (host::node_index(node_id) == host::kRootIndex) return root_widget_;
  GtkNodeView* const* v = const_cast<host::RetainedTree<Adapter>&>(tree_).find(node_id);
  return v != nullptr && *v != nullptr ? (*v)->widget : nullptr;
}

gboolean NveGtkHost::on_tick(GtkWidget*, GdkFrameClock*, gpointer user_data) {
  static_cast<NveGtkHost*>(user_data)->sync_surface_size_from_widget();
  return G_SOURCE_CONTINUE;
}

void NveGtkHost::on_window_size_notify(GObject* object, GParamSpec*, gpointer user_data) {
  auto* host = static_cast<NveGtkHost*>(user_data);
  int w = 0;
  int h = 0;
  gtk_window_get_default_size(GTK_WINDOW(object), &w, &h);
  if (w > 0 && h > 0) {
    const int scale = std::max(1, gtk_widget_get_scale_factor(GTK_WIDGET(object)));
    host->set_surface_size(static_cast<float>(w), static_cast<float>(h), static_cast<float>(scale));
  } else {
    host->sync_surface_size_from_widget();
  }
}

void NveGtkHost::on_window_destroy(GtkWidget*, gpointer user_data) {
  static_cast<NveGtkHost*>(user_data)->window_binding_ = WindowBinding{};
}

}  // namespace nve::gtk
