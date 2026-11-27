#include "NveQtHost.hpp"

#include "NveQtText.hpp"

#include <QDesktopServices>
#include <QMetaObject>
#include <QObject>
#include <QUrl>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <utility>

namespace nve::qt {

namespace {

float positive_or(float v, float fallback) noexcept { return std::isfinite(v) && v > 0.0f ? v : fallback; }

}  // namespace

QtHost::QtHost(QObject* ui, QtHostConfig config) : ui_(ui), config_(config) {
  config_.surface_id = config_.surface_id != 0u ? config_.surface_id : 1u;
  config_.width = positive_or(config_.width, 640.0f);
  config_.height = positive_or(config_.height, 420.0f);
  config_.scale_factor = positive_or(config_.scale_factor, 1.0f);
  create_session();
}

QtHost::~QtHost() { session_.reset(); }

void QtHost::create_session() {
  NveRuntimeConfig rc = host::RuntimeSession::default_config(config_.surface_id, config_.width, config_.height);
  rc.scale_factor = config_.scale_factor;
  std::string error;
  session_ = host::HostSession::create(rc, &error);
  if (!session_) {  // libnve_runtime is linked in: an ABI mismatch is a build bug
    std::fprintf(stderr, "nve-qt-shell: %s\n", error.c_str());
    std::abort();
  }
  (void)session_->runtime().set_text_measurer(&QtHost::measure_trampoline, this);
  session_->set_event_sink(sink_);
  // Runtime wakes (paired frames, timers) arrive on any thread, also the owner thread in the
  // middle of `attach` / `restore_state`: always queue a pump on the UI thread (never run it
  // inline, which would re-enter the session), dropped once the host is gone.
  std::weak_ptr<std::atomic<bool>> alive = alive_;
  QObject* ui = ui_;
  session_->set_wake_poster([alive, ui, this] {
    QMetaObject::invokeMethod(
        ui,
        [alive, this] {
          if (const auto a = alive.lock(); a && a->load(std::memory_order_acquire)) (void)pump();
        },
        Qt::QueuedConnection);
  });
}

void QtHost::shutdown() noexcept {
  alive_->store(false, std::memory_order_release);
  if (!session_) return;
  session_->set_wake_poster({});  // drains in-flight wakes
  session_->set_event_sink({});
  syncing_ = true;
  clear_views();
  (void)session_->runtime().clear_text_measurer();
  session_.reset();
}

// --- App sources -------------------------------------------------------------------------------

// Every source change bumps `HostSession::source_generation()`: the next sync is a full pass that
// keeps views whose node still matches (focus survives a hot reload) and destroys the rest.

bool QtHost::attach_interface(NveGetInterfaceFn get_interface) {
  if (!session_->attach(get_interface)) return false;
  (void)pump();
  return true;
}

bool QtHost::load_dylib(std::string_view path, host::ModuleError* out_error, std::string* out_message) {
  if (!session_->load_module(path, out_error, out_message)) return false;
  (void)pump();
  return true;
}

bool QtHost::reload_dylib(std::string_view path, host::ModuleError* out_error, std::string* out_message) {
  if (!session_->reload_module(path, out_error, out_message)) return false;
  (void)pump();
  return true;
}

bool QtHost::connect_paired(std::string_view uri, std::string_view token, std::string* out_error) {
  if (!session_->pair(uri, token, out_error)) return false;
  (void)pump();
  return true;
}

void QtHost::disconnect_paired() {
  session_->unpair();
  (void)pump();
}

bool QtHost::is_paired() const noexcept { return session_->runtime().pair_is_connected(); }

bool QtHost::submit_frame(std::span<const uint8_t> nve2) {
  if (session_->runtime().submit_nve2(nve2) != NVE_RUNTIME_OK) return false;
  (void)pump();
  return true;
}

// --- App state & dev loop ----------------------------------------------------------------------

bool QtHost::save_state(std::string_view path, uint32_t schema_version) {
  return session_->save_state(path, schema_version);
}

bool QtHost::restore_state(std::string_view path, uint32_t schema_version) {
  if (!session_->restore_state_file(path, schema_version)) return false;
  (void)pump();
  return true;
}

bool QtHost::clear_state(std::string_view path) { return session_->clear_state_file(path); }

bool QtHost::apply_props(std::span<const NveMapEntry> props) {
  const bool ok = session_->set_props(props);
  (void)sync();
  return ok;
}

bool QtHost::apply_hot_patch(std::span<const uint8_t> patch_nvp1) {
  const bool changed = session_->apply_hot_patch(patch_nvp1);
  (void)sync();
  return changed;
}

void QtHost::set_event_callback(EventCallback callback) {
  sink_ = std::move(callback);
  session_->set_event_sink(sink_);
}

void QtHost::set_text_measurer(TextMeasurer measurer) {
  measurer_ = std::move(measurer);
  (void)session_->runtime().set_text_measurer(&QtHost::measure_trampoline, this);  // invalidates measured text
  (void)pump();
}

size_t QtHost::pump() {
  if (syncing_) return 0u;  // re-entrant wake from a widget signal raised during sync
  (void)session_->pump();
  return sync();
}

size_t QtHost::sync() {
  if (syncing_) return 0u;
  syncing_ = true;
  size_t touched = 0u;
  {
    const auto guard = session_->runtime().acquire_scene();
    if (guard) {
      scene_ = &*guard;
      touched = sync_views(*guard);
      scene_ = nullptr;
    }
  }
  syncing_ = false;
  after_sync();
  return touched;
}

// --- Input -------------------------------------------------------------------------------------

bool QtHost::dispatch(const NveDirectEvent& ev) {
  if (syncing_) return false;
  const bool changed = session_->dispatch(ev);
  (void)sync();
  return changed;
}

bool QtHost::simulate_press(uint32_t node_id, float x, float y) {
  return dispatch(session_->events().press(node_id, x, y));
}

bool QtHost::dispatch_text_change(uint32_t node_id, std::string_view text) {
  return dispatch(session_->events().text(node_id, NVE_EVENT_TEXT_CHANGE, text));
}

bool QtHost::dispatch_submit(uint32_t node_id, std::string_view text) {
  return dispatch(session_->events().text(node_id, NVE_EVENT_SUBMIT, text));
}

bool QtHost::dispatch_focus(uint32_t node_id, bool focused) {
  return dispatch(session_->events().focus(node_id, focused));
}

bool QtHost::dispatch_scroll(uint32_t node_id, float offset_x, float offset_y, float content_width,
                             float content_height) {
  (void)set_scroll_offset(node_id, offset_x, offset_y);
  return dispatch(session_->events().scroll(node_id, offset_x, offset_y, content_width, content_height));
}

bool QtHost::dispatch_pointer(uint32_t node_id, NveEventKind kind, float x, float y) {
  NveDirectEvent ev = session_->events().base(node_id, kind, NVE_EVENT_PAYLOAD_POINT);
  ev.x = x;
  ev.y = y;
  return dispatch(ev);
}

bool QtHost::activate(uint32_t node_id, bool wants_press, std::string_view href, float x, float y) {
  if (wants_press) return simulate_press(node_id, x, y);
  if (href.empty()) return false;
  const QUrl url = QUrl::fromUserInput(QString::fromUtf8(href.data(), static_cast<qsizetype>(href.size())));
  return url.isValid() && QDesktopServices::openUrl(url);
}

std::optional<QPointF> QtHost::scroll_by(uint32_t node_id, float dx, float dy) {
  float x = 0.0f;
  float y = 0.0f;
  if (session_->runtime().scroll_by(node_id, dx, dy, &x, &y) != NVE_RUNTIME_OK) return std::nullopt;
  return QPointF(x, y);
}

std::optional<QPointF> QtHost::set_scroll_offset(uint32_t node_id, float x, float y) {
  const std::optional<QPointF> current = scroll_by(node_id, 0.0f, 0.0f);
  if (!current) return std::nullopt;
  return scroll_by(node_id, x - static_cast<float>(current->x()), y - static_cast<float>(current->y()));
}

// --- Surface -----------------------------------------------------------------------------------

void QtHost::set_surface_size(float width, float height, float scale_factor) {
  if (!std::isfinite(width) || !std::isfinite(height) || width <= 0.0f || height <= 0.0f) return;
  const float scale = positive_or(scale_factor, config_.scale_factor);
  if (width == config_.width && height == config_.height && scale == config_.scale_factor) return;
  config_.width = width;
  config_.height = height;
  config_.scale_factor = scale;
  (void)session_->runtime().set_viewport(width, height, scale);
  (void)pump();
}

void QtHost::set_surface_id(uint32_t surface_id) {
  if (surface_id == 0u || surface_id == config_.surface_id) return;
  session_->set_wake_poster({});
  syncing_ = true;
  clear_views();
  syncing_ = false;
  session_.reset();
  config_.surface_id = surface_id;
  create_session();
}

std::span<const uint8_t> QtHost::scene_resource(uint32_t id, host::Key64 kind) const noexcept {
  if (scene_ == nullptr || id == 0u) return {};
  const auto res = scene_->resource(id);
  return res && res->kind == kind ? res->bytes : std::span<const uint8_t>{};
}

std::optional<NodeContent> QtHost::resolve_content(NodePaint& paint, ContentIds& ids, const host::NodeData& node,
                                                   const canvas::CanvasTable& canvas) {
  const auto become = [&paint](NodeContent kind) {
    if (kind != NodeContent::Text) paint.text.clear();
    if (kind != NodeContent::Image) paint.image = QImage();
    if (kind != NodeContent::Canvas) paint.draw_list.clear();
    paint.content = kind;
  };
  if (canvas.has(node.index)) {
    const canvas::CanvasData c = canvas.at(node.index);
    if (paint.content == NodeContent::Canvas && c.draw_list == ids.canvas_res && c.generation == ids.canvas_generation) {
      return std::nullopt;
    }
    ids.canvas_res = c.draw_list;
    ids.canvas_generation = c.generation;
    const std::span<const uint8_t> bytes = scene_resource(c.draw_list, canvas::kDrawListKind);
    if (paint.content == NodeContent::Canvas && std::ranges::equal(bytes, paint.draw_list)) return std::nullopt;
    become(NodeContent::Canvas);
    paint.draw_list.assign(bytes.begin(), bytes.end());
    return NodeContent::Canvas;
  }
  ids.canvas_res = ids.canvas_generation = 0u;
  if (node.kind == host::NodeKind::Image) {
    const uint32_t tint = node.image.tint_enabled ? node.image.tint : 0u;
    if (paint.content == NodeContent::Image && ids.image_src == node.image.src && paint.fit == node.image.fit &&
        ids.image_tint == tint) {
      return std::nullopt;
    }
    ids.image_src = node.image.src;
    ids.image_tint = tint;
    QImage image;
    if (const auto src = host::decode_image_src(scene_resource(node.image.src, host::kImageSrcKind))) {
      image = images_.get(*src);
    }
    if (!image.isNull() && node.image.tint_enabled) image = tinted_image(image, qcolor(tint));
    become(NodeContent::Image);
    paint.image = image;
    paint.fit = node.image.fit;
    return NodeContent::Image;
  }
  ids.image_src = ids.image_tint = 0u;
  // Text paints on leaves only (a text wrapper with children is a View holding its runs).
  if (!node.text.empty() && node.children.empty()) {
    QString text = display_text(node.text);
    QtTextStyle style = text_style(node.text_style);
    if (paint.content == NodeContent::Text && text == paint.text && style == paint.style) return std::nullopt;
    become(NodeContent::Text);
    paint.text = std::move(text);
    paint.style = std::move(style);
    return NodeContent::Text;
  }
  if (paint.content == NodeContent::None) return std::nullopt;
  become(NodeContent::None);
  return NodeContent::None;
}

void QtHost::measure_trampoline(void* ctx, NveStrSlice text, float font_size, uint32_t font_weight, float max_width,
                                float* out_width, float* out_height) {
  const auto* self = static_cast<const QtHost*>(ctx);
  if (!self->measurer_) {
    measure_text_callback(nullptr, text, font_size, font_weight, max_width, out_width, out_height);
    return;
  }
  const std::string_view sv(reinterpret_cast<const char*>(text.ptr), text.ptr != nullptr ? text.len : 0u);
  const NveIntrinsicSize size =
      self->measurer_(sv, font_size, static_cast<uint16_t>(std::min<uint32_t>(font_weight, 1000u)), max_width);
  *out_width = size.width;
  *out_height = size.height;
}

}  // namespace nve::qt
