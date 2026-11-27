#include "NveWinHost.hpp"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <utility>

namespace nve::win {

namespace {

float positive_or(float v, float fallback) noexcept { return std::isfinite(v) && v > 0.0f ? v : fallback; }

bool takes_press_or_link(const WinNodeVisual& v) noexcept {
  return v.wants(NVE_EVENT_PRESS) || (!v.href.empty() && !v.interaction.disabled);
}

bool hover_target(const WinNodeVisual& v) noexcept {
  return v.wants(NVE_EVENT_POINTER_ENTER) || v.wants(NVE_EVENT_POINTER_LEAVE);
}

}  // namespace

NveWinHost::NveWinHost(WinHostConfig config) : config_(config) {
  config_.surface_id = config_.surface_id != 0u ? config_.surface_id : 1u;
  config_.width = positive_or(config_.width, 640.0f);
  config_.height = positive_or(config_.height, 420.0f);
  config_.scale_factor = positive_or(config_.scale_factor, 1.0f);

  NveRuntimeConfig rc = host::RuntimeSession::default_config(config_.surface_id, config_.width, config_.height);
  rc.scale_factor = config_.scale_factor;
  std::string error;
  session_ = host::HostSession::create(rc, &error);
  if (!session_) {  // libnve_runtime is linked in: an ABI mismatch is a build bug
    std::fprintf(stderr, "nve-windows-shell: %s\n", error.c_str());
    std::abort();
  }
#if defined(_WIN32)
  (void)session_->runtime().set_text_measurer(&DirectWriteTextMeasurer::measure_trampoline, &measurer_);
#endif
}

NveWinHost::~NveWinHost() {
  session_->set_wake_poster({});
  session_->set_event_sink({});
  on_change_ = {};
  syncing_ = true;
  tree_.clear();
  (void)session_->runtime().clear_text_measurer();
  session_.reset();
}

// --- App sources -------------------------------------------------------------------------------

void NveWinHost::source_changed() {
  hovered_ = 0u;  // hover belongs to the previous source's nodes
  (void)pump();
}

bool NveWinHost::attach_interface(NveGetInterfaceFn get_interface) {
  if (!session_->attach(get_interface)) return false;
  source_changed();
  return true;
}

bool NveWinHost::load_dylib(std::string_view path, host::ModuleError* out_error, std::string* out_message) {
  if (!session_->load_module(path, out_error, out_message)) return false;
  source_changed();
  return true;
}

bool NveWinHost::reload_dylib(std::string_view path, host::ModuleError* out_error, std::string* out_message) {
  if (!session_->reload_module(path, out_error, out_message)) return false;
  source_changed();
  return true;
}

bool NveWinHost::connect_paired(std::string_view uri, std::string_view token, std::string* out_error) {
  if (!session_->pair(uri, token, out_error)) return false;
  source_changed();
  return true;
}

void NveWinHost::disconnect_paired() {
  session_->unpair();
  source_changed();
}

bool NveWinHost::is_paired() const noexcept { return session_->runtime().pair_is_connected(); }

bool NveWinHost::submit_frame(std::span<const uint8_t> nve2) {
  if (session_->runtime().submit_nve2(nve2) != NVE_RUNTIME_OK) return false;
  (void)pump();
  return true;
}

// --- App state & dev loop ----------------------------------------------------------------------

bool NveWinHost::save_state(std::string_view path, uint32_t schema_version) {
  return session_->save_state(path, schema_version);
}

bool NveWinHost::restore_state(std::string_view path, uint32_t schema_version) {
  if (!session_->restore_state_file(path, schema_version)) return false;
  (void)pump();
  return true;
}

bool NveWinHost::clear_state(std::string_view path) { return session_->clear_state_file(path); }

bool NveWinHost::apply_props(std::span<const NveMapEntry> props) {
  const bool ok = session_->set_props(props);
  sync_views();
  return ok;
}

bool NveWinHost::apply_hot_patch(std::span<const uint8_t> patch_nvp1) {
  const bool changed = session_->apply_hot_patch(patch_nvp1);
  sync_views();
  return changed;
}

void NveWinHost::set_event_callback(EventCallback callback) { session_->set_event_sink(std::move(callback)); }

void NveWinHost::set_wake_poster(host::HostSession::WakePoster poster) { session_->set_wake_poster(std::move(poster)); }

size_t NveWinHost::pump() {
  if (syncing_) return 0u;  // re-entrant wake raised while mirroring
  (void)session_->pump();
  const uint64_t before = tree_.updated_count() + tree_.created_count();
  sync_views();
  return static_cast<size_t>(tree_.updated_count() + tree_.created_count() - before);
}

void NveWinHost::sync_views() {
  if (syncing_) return;
  syncing_ = true;
  const uint64_t before = tree_.revision();
  {
    auto guard = session_->runtime().acquire_scene();
    if (guard) (void)tree_.sync(*guard);
  }
  syncing_ = false;
  notify_if_changed(before);
}

void NveWinHost::notify_if_changed(uint64_t before) {
  if (tree_.revision() != before && on_change_) on_change_();
}

// --- Input -------------------------------------------------------------------------------------

bool NveWinHost::dispatch(const NveDirectEvent& ev) {
  if (syncing_) return false;
  const bool changed = session_->dispatch(ev);
  sync_views();
  return changed;
}

bool NveWinHost::dispatch_pointer(uint32_t node, NveEventKind kind, float x, float y) {
  const auto frame = tree_.frame_of(node);
  const float lx = frame ? x - frame->rect.x : x, ly = frame ? y - frame->rect.y : y;
  return dispatch(session_->events().pointer(node, kind, lx, ly));
}

bool NveWinHost::simulate_press(uint32_t node_id, float x, float y) {
  return dispatch(session_->events().press(node_id, x, y));
}

PressResult NveWinHost::press_at(float x, float y) {
  PressResult r;
  const WinNodeVisual* target = tree_.nearest(tree_.hit_test(x, y), takes_press_or_link);
  if (target == nullptr) return r;
  r.node_id = target->id;
  if (!target->wants(NVE_EVENT_PRESS)) {
    r.href = target->href;
    return r;
  }
  r.dispatched = true;
  r.changed = dispatch_pointer(r.node_id, NVE_EVENT_PRESS, x, y);  // may destroy `target`
  return r;
}

bool NveWinHost::scrolled(const WinNodeVisual& node, float x, float y) {
  const uint32_t id = node.id;
  const auto [cw, ch] = tree_.content_size(node);
  const bool report = node.wants(NVE_EVENT_SCROLL);
  const uint64_t before = tree_.revision();
  tree_.set_scroll_offset(id, x, y);
  notify_if_changed(before);
  if (report) {
    (void)dispatch(session_->events().scroll(id, x, y, cw, ch));
  } else {
    (void)pump();  // republish the runtime's offset
  }
  return true;
}

bool NveWinHost::wheel_at(float x, float y, float dx, float dy) {
  if (syncing_ || !std::isfinite(dx) || !std::isfinite(dy)) return false;
  const auto scrollable = [](const WinNodeVisual& v) { return v.scrollable(); };
  for (const WinNodeVisual* c = tree_.nearest(tree_.hit_test(x, y), scrollable); c != nullptr;
       c = tree_.nearest(tree_.find(c->parent), scrollable)) {
    host::RuntimeSession& rt = session_->runtime();
    float bx = 0.0f, by = 0.0f, nx = 0.0f, ny = 0.0f;
    if (rt.scroll_by(c->id, 0.0f, 0.0f, &bx, &by) != NVE_RUNTIME_OK ||
        rt.scroll_by(c->id, dx, dy, &nx, &ny) != NVE_RUNTIME_OK) {
      continue;
    }
    if (nx != bx || ny != by) return scrolled(*c, nx, ny);  // else at its edge: chain outwards
  }
  return false;
}

bool NveWinHost::dispatch_scroll(uint32_t node_id, float offset_x, float offset_y) {
  const WinNodeVisual* node = tree_.find(node_id);
  if (node == nullptr || syncing_) return false;
  host::RuntimeSession& rt = session_->runtime();
  float x = 0.0f, y = 0.0f;
  if (rt.scroll_by(node_id, 0.0f, 0.0f, &x, &y) != NVE_RUNTIME_OK ||
      rt.scroll_by(node_id, offset_x - x, offset_y - y, &x, &y) != NVE_RUNTIME_OK) {
    return false;
  }
  return scrolled(*node, x, y);
}

void NveWinHost::pointer_move(float x, float y) {
  if (syncing_) return;
  const WinNodeVisual* hit = tree_.hit_test(x, y);
  const WinNodeVisual* target = tree_.nearest(hit, hover_target);
  const uint32_t next = target != nullptr ? target->id : 0u;
  const auto wants = [this](uint32_t id, NveEventKind k) {
    const WinNodeVisual* v = tree_.find(id);
    return v != nullptr && v->wants(k);
  };
  const uint32_t mover = [&] {
    const WinNodeVisual* m = tree_.nearest(hit, [](const WinNodeVisual& v) { return v.wants(NVE_EVENT_POINTER_MOVE); });
    return m != nullptr ? m->id : 0u;
  }();
  if (next != hovered_) {
    const uint32_t prev = std::exchange(hovered_, next);
    if (prev != 0u && wants(prev, NVE_EVENT_POINTER_LEAVE)) (void)dispatch_pointer(prev, NVE_EVENT_POINTER_LEAVE, x, y);
    if (next != 0u && wants(next, NVE_EVENT_POINTER_ENTER)) (void)dispatch_pointer(next, NVE_EVENT_POINTER_ENTER, x, y);
  }
  if (mover != 0u && wants(mover, NVE_EVENT_POINTER_MOVE)) (void)dispatch_pointer(mover, NVE_EVENT_POINTER_MOVE, x, y);
}

void NveWinHost::pointer_leave() {
  const uint32_t prev = std::exchange(hovered_, 0u);
  const WinNodeVisual* v = tree_.find(prev);
  if (v != nullptr && v->wants(NVE_EVENT_POINTER_LEAVE)) {
    (void)dispatch(session_->events().pointer(prev, NVE_EVENT_POINTER_LEAVE, -1.0f, -1.0f));
  }
}

uint8_t NveWinHost::cursor_at(float x, float y) const {
  const WinNodeVisual* hit = tree_.hit_test(x, y);
  if (const WinNodeVisual* c = tree_.nearest(hit, [](const WinNodeVisual& v) {
        return v.interaction.cursor != NVE_CURSOR_KIND_AUTO;
      })) {
    return c->interaction.cursor;
  }
  const auto link = [](const WinNodeVisual& v) { return !v.href.empty() && !v.interaction.disabled; };
  return static_cast<uint8_t>(tree_.nearest(hit, link) != nullptr ? NVE_CURSOR_KIND_POINTER : NVE_CURSOR_KIND_AUTO);
}

bool NveWinHost::dispatch_text_change(uint32_t node_id, std::string_view text) {
  return dispatch(session_->events().text(node_id, NVE_EVENT_TEXT_CHANGE, text));
}

bool NveWinHost::dispatch_submit(uint32_t node_id, std::string_view text) {
  return dispatch(session_->events().text(node_id, NVE_EVENT_SUBMIT, text));
}

bool NveWinHost::dispatch_focus(uint32_t node_id, bool focused) {
  return dispatch(session_->events().focus(node_id, focused));
}

// --- Surface -----------------------------------------------------------------------------------

void NveWinHost::set_surface_size(float width, float height, float scale_factor) {
  if (!std::isfinite(width) || !std::isfinite(height) || width <= 0.0f || height <= 0.0f) return;
  config_.width = width;
  config_.height = height;
  config_.scale_factor = positive_or(scale_factor, config_.scale_factor);
  (void)session_->runtime().set_viewport(config_.width, config_.height, config_.scale_factor);
  (void)pump();
}

}  // namespace nve::win
