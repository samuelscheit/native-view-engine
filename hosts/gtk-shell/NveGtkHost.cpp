#include "NveGtkInternal.hpp"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <limits>
#include <utility>

namespace nve {

namespace {

std::atomic<uint32_t> g_next_gtk_host_id{1};

} // namespace

// -----------------------------------------------------------------------------
// NveGtkHost lifecycle, Pango measurement, session & event dispatch
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
  if (alive_) {
    alive_->store(false, std::memory_order_release);
  }
  session_.disconnect_paired();
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
  canvas_bindings_.clear();
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
    gtk_internal::unparent_all_fixed_children(root_widget_);
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

bool NveGtkHost::connect_paired_endpoint(
    std::string_view endpoint,
    std::string_view token,
    std::string_view transport,
    std::string* out_error,
    uint32_t surface_id) {
  PairClientConfig cfg{};
  if (!parse_pair_endpoint(endpoint, transport, cfg, out_error)) {
    return false;
  }
  cfg.pairing_token.assign(token.data(), token.size());
  return connect_paired(cfg, out_error, surface_id);
}

bool NveGtkHost::connect_paired(
    const PairClientConfig& config,
    std::string* out_error,
    uint32_t surface_id) {
  NveSurfaceConfig next_cfg = config_;
  if (surface_id != 0u) {
    next_cfg.surface_id = surface_id;
  }

  struct GtkPairWakePayload {
    NveGtkHost* host{nullptr};
    std::shared_ptr<std::atomic<bool>> alive;
  };
  std::shared_ptr<std::atomic<bool>> alive_ref = alive_;
  auto wake_cb = [this, alive_ref]() {
    if (!alive_ref || !alive_ref->load(std::memory_order_acquire)) {
      return;
    }
    auto* payload = new GtkPairWakePayload{this, alive_ref};
    g_main_context_invoke(
        nullptr,
        [](gpointer data) -> gboolean {
          std::unique_ptr<GtkPairWakePayload> p(static_cast<GtkPairWakePayload*>(data));
          if (p->alive && p->alive->load(std::memory_order_acquire) && p->host != nullptr) {
            (void)p->host->poll_paired_frames();
          }
          return G_SOURCE_REMOVE;
        },
        payload);
  };

  if (!session_.connect_paired(config, next_cfg, std::move(wake_cb), out_error)) {
    return false;
  }
  config_ = next_cfg;
  reconciler_.clear(adapter_);
  raw_props_initialized_ = false;
  sync_views(/*check_raw_props=*/true);
  return true;
}

void NveGtkHost::disconnect_paired() noexcept {
  session_.disconnect_paired();
}

size_t NveGtkHost::poll_paired_frames() {
  if (!session_.is_paired()) {
    return 0u;
  }
  const size_t applied = session_.poll_paired_frames(nullptr);
  if (applied > 0u) {
    sync_views(/*check_raw_props=*/false);
  }
  return applied;
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

bool NveGtkHost::apply_props(const PropMap& props) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.update_props(props, /*auto_render=*/true);
  if (dirty) {
    sync_views(/*check_raw_props=*/true);
  }
  (void)session_.drain_events();
  return dirty;
}

bool NveGtkHost::apply_props(const NveMapEntry* entries, uint32_t count) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.update_props(entries, count, /*auto_render=*/true);
  if (dirty) {
    sync_views(/*check_raw_props=*/true);
  }
  (void)session_.drain_events();
  return dirty;
}

bool NveGtkHost::apply_hot_patch(std::span<const uint8_t> patch_nvp1) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.bridge()->apply_hot_patch(patch_nvp1, /*auto_render=*/true);
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
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0u;
  }
  const uint32_t dirty = session_.dispatch_press(node_id, x, y, /*auto_render=*/true);
  if (dirty != 0u && !session_.is_paired()) {
    sync_views(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return dirty;
}

uint32_t NveGtkHost::dispatch_event_batch(NveEventBatch batch) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0u;
  }
  const uint32_t dirty = session_.dispatch_events(batch, /*auto_render=*/true);
  if (dirty != 0u && !session_.is_paired()) {
    sync_views(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return dirty;
}

uint32_t NveGtkHost::dispatch_text_change(uint32_t node_id, std::string_view text) {
  const NveEvent ev = make_text_event(
      config_.surface_id,
      node_id,
      current_timestamp_us(),
      NVE_TEXT_INPUT_EVENT_CHANGE,
      static_cast<uint32_t>(text.size()));
  return dispatch_event_batch(make_single_event_batch(
      ev,
      text.empty() ? nullptr : reinterpret_cast<const uint8_t*>(text.data()),
      static_cast<uint32_t>(text.size())));
}

uint32_t NveGtkHost::dispatch_submit(uint32_t node_id, std::string_view text) {
  const NveEvent ev = make_text_event(
      config_.surface_id,
      node_id,
      current_timestamp_us(),
      NVE_TEXT_INPUT_EVENT_SUBMIT,
      static_cast<uint32_t>(text.size()));
  return dispatch_event_batch(make_single_event_batch(
      ev,
      text.empty() ? nullptr : reinterpret_cast<const uint8_t*>(text.data()),
      static_cast<uint32_t>(text.size())));
}

uint32_t NveGtkHost::dispatch_focus(uint32_t node_id, bool focused) {
  const uint64_t ts = current_timestamp_us();
  const NveEvent ev = focused ? make_focus_event(config_.surface_id, node_id, ts)
                              : make_blur_event(config_.surface_id, node_id, ts);
  return dispatch_event_batch(make_single_event_batch(ev));
}

uint32_t NveGtkHost::dispatch_scroll(
    uint32_t node_id,
    float offset_x,
    float offset_y,
    float content_width,
    float content_height) {
  const NveEvent ev = make_scroll_event(
      config_.surface_id,
      node_id,
      current_timestamp_us(),
      offset_x,
      offset_y,
      content_width,
      content_height);
  return dispatch_event_batch(make_single_event_batch(ev));
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
