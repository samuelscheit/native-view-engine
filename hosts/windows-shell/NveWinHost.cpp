#include "NveWinHost.hpp"

#include <nve/host/urls.hpp>

#include <algorithm>
#include <cmath>
#include <utility>
#include <vector>

namespace nve::win {

namespace {

float positive_or(float v, float fallback) noexcept { return std::isfinite(v) && v > 0.0f ? v : fallback; }

bool hover_target(const WinNodeVisual& v) noexcept {
  return v.wants(NVE_INTERACTION_EVENT_POINTER_ENTER) || v.wants(NVE_INTERACTION_EVENT_POINTER_LEAVE);
}

bool is_tab_focusable(const WinNodeVisual& v) noexcept {
  const int16_t ti = v.interaction.tab_index.value_or(0);
  if (v.interaction.disabled || ti < 0) return false;
  return v.kind == host::NodeKind::TextInput || v.interaction.focusable ||
         (v.interaction.tab_index.has_value() && *v.interaction.tab_index >= 0) ||
         v.wants(NVE_INTERACTION_EVENT_PRESS) || v.wants(NVE_INTERACTION_EVENT_FOCUS) ||
         v.wants(NVE_INTERACTION_EVENT_KEY_DOWN) || v.wants(NVE_INTERACTION_EVENT_KEY_UP) ||
         host::openable_link(v.href);
}

bool is_node_shown(const WinNodeVisual& v) noexcept {
  return v.visual.visibility != NVE_VISIBILITY_HIDDEN && v.visual.opacity > 0.001f;
}

bool is_click_focusable(const WinNodeVisual& v) noexcept {
  if (v.interaction.disabled) return false;
  return v.kind == host::NodeKind::TextInput || v.interaction.focusable || v.interaction.tab_index.has_value() ||
         v.wants(NVE_INTERACTION_EVENT_PRESS) || v.wants(NVE_INTERACTION_EVENT_FOCUS) ||
         v.wants(NVE_INTERACTION_EVENT_KEY_DOWN) || v.wants(NVE_INTERACTION_EVENT_KEY_UP) ||
         host::openable_link(v.href);
}

struct TabEntry {
  uint32_t id = 0u;
  int16_t tab_index = 0;
  size_t order = 0u;
};

void find_topmost_modal(const NveWinVisualTree& tree, std::span<const uint32_t> ids, bool visible,
                        uint32_t& modal_root) noexcept {
  for (const uint32_t id : ids) {
    const WinNodeVisual* v = tree.find(id);
    if (v == nullptr || v->interaction.disabled) continue;
    const bool v_vis = v->visual.visibility == NVE_VISIBILITY_HIDDEN    ? false
                       : v->visual.visibility == NVE_VISIBILITY_VISIBLE ? true
                                                                        : visible;
    if (!v_vis || v->visual.opacity <= 0.001f) continue;
    if (v->modal) modal_root = id;
    find_topmost_modal(tree, v->children, v_vis, modal_root);
  }
}

void collect_tab_candidates(const NveWinVisualTree& tree, std::span<const uint32_t> ids, bool visible,
                            std::vector<TabEntry>& out) {
  for (const uint32_t id : ids) {
    const WinNodeVisual* v = tree.find(id);
    if (v == nullptr || v->interaction.disabled) continue;
    const bool v_vis = v->visual.visibility == NVE_VISIBILITY_HIDDEN    ? false
                       : v->visual.visibility == NVE_VISIBILITY_VISIBLE ? true
                                                                        : visible;
    if (!v_vis || !is_node_shown(*v)) continue;
    if (is_tab_focusable(*v)) {
      out.push_back(TabEntry{id, v->interaction.tab_index.value_or(0), out.size()});
    }
    collect_tab_candidates(tree, v->children, v_vis, out);
  }
}

}  // namespace

std::unique_ptr<NveWinHost> NveWinHost::create(WinHostConfig config, std::string* error) {
  config.surface_id = config.surface_id != 0u ? config.surface_id : 1u;
  config.width = positive_or(config.width, 640.0f);
  config.height = positive_or(config.height, 420.0f);
  config.scale_factor = positive_or(config.scale_factor, 1.0f);
  NveRuntimeConfig rc = host::RuntimeSession::default_config(config.surface_id, config.width, config.height);
  rc.scale_factor = config.scale_factor;
  std::unique_ptr<host::HostSession> session = host::HostSession::create(rc, error);
  if (!session) return nullptr;
  std::unique_ptr<NveWinHost> host(new NveWinHost(config, std::move(session)));
#if defined(_WIN32)
  if (!host->measurer_.valid()) {
    if (error != nullptr) *error = "DirectWrite is unavailable (DWriteCreateFactory failed)";
    return nullptr;
  }
  (void)host->session_->runtime().set_text_measurer(&DirectWriteTextMeasurer::measure_trampoline, &host->measurer_);
#endif
  return host;
}

NveWinHost::NveWinHost(WinHostConfig config, std::unique_ptr<host::HostSession> session)
    : config_(config), session_(std::move(session)) {}

NveWinHost::~NveWinHost() {
  alive_->store(false, std::memory_order_release);
  session_->set_wake_poster({});
  session_->set_event_sink({});
  on_change_ = {};
  on_scene_sync_ = {};
  syncing_ = true;
  tree_.clear();
  (void)session_->runtime().clear_text_measurer();
  session_.reset();
}

// --- App sources -------------------------------------------------------------------------------

void NveWinHost::source_changed() {
  hovered_ = pressed_ = focused_ = 0u;  // hover / press / focus targets belong to the previous source's nodes
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

bool NveWinHost::save_frame0_file(std::string_view path, uint64_t app_build_id, uint64_t env_hash) {
  return session_->save_frame0_file(path, app_build_id, env_hash);
}

bool NveWinHost::boot_frame0_file(std::string_view path, uint64_t app_build_id, uint64_t env_hash) {
  if (!session_->boot_frame0_file(path, app_build_id, env_hash)) return false;
  hovered_ = pressed_ = focused_ = 0u;
  sync_views();
  return true;
}

bool NveWinHost::boot_frame0_bytes(std::span<const uint8_t> envelope, uint64_t app_build_id, uint64_t env_hash) {
  if (!session_->boot_frame0_bytes(envelope, app_build_id, env_hash)) return false;
  hovered_ = pressed_ = focused_ = 0u;
  sync_views();
  return true;
}

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
  const auto alive = alive_;  // the event sink may destroy this host during the pump
  (void)session_->pump();
  if (!alive->load(std::memory_order_acquire)) return 0u;
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
    if (guard) {
      (void)tree_.sync(*guard);
      if (focused_ != 0u && tree_.find(focused_) == nullptr) focused_ = 0u;
      if (on_scene_sync_) on_scene_sync_(*guard);
    }
  }
  syncing_ = false;
  notify_if_changed(before);
}

void NveWinHost::notify_if_changed(uint64_t before) {
  if (tree_.revision() != before && on_change_) on_change_();
}

// --- Input -------------------------------------------------------------------------------------

bool NveWinHost::dispatch(const NveEvent& ev) {
  if (syncing_) return false;
  const auto alive = alive_;  // the event sink may destroy this host during the dispatch
  const bool changed = session_->dispatch(ev);
  if (alive->load(std::memory_order_acquire)) sync_views();
  return changed;
}

bool NveWinHost::dispatch_text(uint32_t node_id, NveTextInputEvent kind, std::string_view text) {
  if (syncing_) return false;
  const auto alive = alive_;  // the event sink may destroy this host during the dispatch
  const bool changed = session_->dispatch_text(node_id, kind, text);
  if (alive->load(std::memory_order_acquire)) sync_views();
  return changed;
}

bool NveWinHost::dispatch_pointer(uint32_t node, NveInteractionEvent kind, float x, float y) {
  const auto frame = tree_.frame_of(node);
  const float lx = frame ? x - frame->rect.x : x, ly = frame ? y - frame->rect.y : y;
  return dispatch(session_->events().pointer(node, kind, lx, ly));
}

bool NveWinHost::simulate_press(uint32_t node_id, float x, float y) {
  return dispatch(session_->events().press(node_id, x, y));
}

uint32_t NveWinHost::press_target(float x, float y) const {
  uint32_t target = 0u;
  for (const WinNodeVisual* v = tree_.hit_test(x, y); v != nullptr; v = tree_.find(v->parent)) {
    if (v->interaction.disabled) return 0u;  // disables itself and everything inside it
    if (target == 0u && (v->wants(NVE_INTERACTION_EVENT_PRESS) || host::openable_link(v->href))) target = v->id;
  }
  return target;
}

uint32_t NveWinHost::focus_target(float x, float y) const {
  uint32_t target = 0u;
  for (const WinNodeVisual* v = tree_.hit_test(x, y); v != nullptr; v = tree_.find(v->parent)) {
    if (v->interaction.disabled) return 0u;
    if (target == 0u && is_click_focusable(*v)) target = v->id;
  }
  return target;
}

bool NveWinHost::scroll_into_view(uint32_t node_id) {
  if (syncing_ || node_id == 0u || tree_.find(node_id) == nullptr) return false;
  const auto alive = alive_;
  std::vector<uint32_t> ancestors;
  for (const WinNodeVisual* cur = tree_.find(node_id); cur != nullptr; cur = tree_.find(cur->parent)) {
    if (cur->parent == 0u) break;
    if (const WinNodeVisual* p = tree_.find(cur->parent); p != nullptr && p->scrollable()) {
      ancestors.push_back(p->id);
    }
  }
  bool any_scrolled = false;
  for (const uint32_t sc_id : ancestors) {
    if (!alive->load(std::memory_order_acquire)) return any_scrolled;
    const WinNodeVisual* sc = tree_.find(sc_id);
    const auto sc_frame = tree_.frame_of(sc_id);
    const auto tgt_frame = tree_.frame_of(node_id);
    if (sc == nullptr || !sc_frame || !tgt_frame) continue;
    const Rect& vp = sc_frame->rect;
    const Rect& tr = tgt_frame->rect;
    float dx = 0.0f, dy = 0.0f;
    if (tr.x < vp.x) {
      dx = tr.x - vp.x;
    } else if (tr.right() > vp.right()) {
      dx = std::min(tr.right() - vp.right(), tr.x - vp.x);
    }
    if (tr.y < vp.y) {
      dy = tr.y - vp.y;
    } else if (tr.bottom() > vp.bottom()) {
      dy = std::min(tr.bottom() - vp.bottom(), tr.y - vp.y);
    }
    if (std::abs(dx) > 1e-3f || std::abs(dy) > 1e-3f) {
      const float prev_x = sc->scroll_x, prev_y = sc->scroll_y;
      if (dispatch_scroll(sc_id, prev_x + dx, prev_y + dy)) {
        if (!alive->load(std::memory_order_acquire)) return true;
        if (const WinNodeVisual* after = tree_.find(sc_id);
            after != nullptr &&
            (std::abs(after->scroll_x - prev_x) > 1e-4f || std::abs(after->scroll_y - prev_y) > 1e-4f)) {
          any_scrolled = true;
        }
      }
    }
  }
  return any_scrolled;
}

uint32_t NveWinHost::pointer_down(float x, float y) {
  pressed_ = press_target(x, y);
  return pressed_;
}

PressResult NveWinHost::pointer_up(float x, float y) {
  PressResult r;
  const uint32_t down = std::exchange(pressed_, 0u);
  const uint32_t id = press_target(x, y);
  const WinNodeVisual* target = id != 0u && id == down ? tree_.find(id) : nullptr;
  if (target == nullptr || syncing_) return r;
  r.node_id = id;
  if (!target->wants(NVE_INTERACTION_EVENT_PRESS)) {
    r.href = target->href;  // `press_target` admitted it: an openable link
    return r;
  }
  r.dispatched = true;
  r.changed = dispatch_pointer(id, NVE_INTERACTION_EVENT_PRESS, x, y);  // may destroy `target`
  return r;
}

PressResult NveWinHost::press_at(float x, float y) {
  (void)pointer_down(x, y);
  return pointer_up(x, y);
}

bool NveWinHost::scrolled(const WinNodeVisual& node, float x, float y) {
  const uint32_t id = node.id;
  const auto [cw, ch] = tree_.content_size(node);
  const bool report = node.wants(NVE_SCROLL_EVENT_SCROLL);
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
  float x = 0.0f, y = 0.0f;
  if (session_->runtime().scroll_to(node_id, offset_x, offset_y, &x, &y) != NVE_RUNTIME_OK) return false;
  return scrolled(*node, x, y);
}

void NveWinHost::pointer_move(float x, float y) {
  if (syncing_) return;
  const WinNodeVisual* hit = tree_.hit_test(x, y);
  const WinNodeVisual* target = tree_.nearest(hit, hover_target);
  const uint32_t next = target != nullptr ? target->id : 0u;
  const auto wants = [this](uint32_t id, NveInteractionEvent k) {
    const WinNodeVisual* v = tree_.find(id);
    return v != nullptr && v->wants(k);
  };
  const uint32_t mover = [&] {
    const WinNodeVisual* m =
        tree_.nearest(hit, [](const WinNodeVisual& v) { return v.wants(NVE_INTERACTION_EVENT_POINTER_MOVE); });
    return m != nullptr ? m->id : 0u;
  }();
  if (next != hovered_) {
    const uint32_t prev = std::exchange(hovered_, next);
    if (prev != 0u && wants(prev, NVE_INTERACTION_EVENT_POINTER_LEAVE)) {
      (void)dispatch_pointer(prev, NVE_INTERACTION_EVENT_POINTER_LEAVE, x, y);
    }
    if (next != 0u && wants(next, NVE_INTERACTION_EVENT_POINTER_ENTER)) {
      (void)dispatch_pointer(next, NVE_INTERACTION_EVENT_POINTER_ENTER, x, y);
    }
  }
  if (mover != 0u && wants(mover, NVE_INTERACTION_EVENT_POINTER_MOVE)) {
    (void)dispatch_pointer(mover, NVE_INTERACTION_EVENT_POINTER_MOVE, x, y);
  }
}

void NveWinHost::pointer_leave() {
  const uint32_t prev = std::exchange(hovered_, 0u);
  const WinNodeVisual* v = tree_.find(prev);
  if (v != nullptr && v->wants(NVE_INTERACTION_EVENT_POINTER_LEAVE)) {
    (void)dispatch(session_->events().pointer(prev, NVE_INTERACTION_EVENT_POINTER_LEAVE, -1.0f, -1.0f));
  }
}

uint8_t NveWinHost::cursor_at(float x, float y) const {
  const WinNodeVisual* hit = tree_.hit_test(x, y);
  if (const WinNodeVisual* c = tree_.nearest(hit, [](const WinNodeVisual& v) {
        return v.interaction.cursor != NVE_CURSOR_KIND_AUTO;
      })) {
    return c->interaction.cursor;
  }
  const auto link = [](const WinNodeVisual& v) { return !v.interaction.disabled && host::openable_link(v.href); };
  return static_cast<uint8_t>(tree_.nearest(hit, link) != nullptr ? NVE_CURSOR_KIND_POINTER : NVE_CURSOR_KIND_AUTO);
}

bool NveWinHost::dispatch_text_change(uint32_t node_id, std::string_view text) {
  return dispatch_text(node_id, NVE_TEXT_INPUT_EVENT_CHANGE, text);
}

bool NveWinHost::dispatch_submit(uint32_t node_id, std::string_view text) {
  return dispatch_text(node_id, NVE_TEXT_INPUT_EVENT_SUBMIT, text);
}

bool NveWinHost::dispatch_composition(uint32_t node_id, NveTextInputEvent kind, std::string_view text) {
  return dispatch_text(node_id, kind, text);
}

bool NveWinHost::dispatch_focus(uint32_t node_id, bool focused) {
  if (syncing_) return false;
  const auto alive = alive_;
  bool changed = false;
  if (focused) {
    const uint32_t prev = std::exchange(focused_, node_id);
    if (prev != 0u && prev != node_id) {
      if (const WinNodeVisual* pv = tree_.find(prev); pv != nullptr && pv->wants(NVE_INTERACTION_EVENT_BLUR)) {
        changed |= dispatch(session_->events().focus(prev, false));
        if (!alive->load(std::memory_order_acquire)) return changed;
      }
    }
  } else if (focused_ == node_id) {
    focused_ = 0u;
  }
  return dispatch(session_->events().focus(node_id, focused)) || changed;
}

bool NveWinHost::dispatch_key(uint32_t node_id, NveInteractionEvent kind, std::string_view key, std::string_view code,
                              uint32_t flags, uint32_t key_code) {
  if (syncing_ || node_id == 0u) return false;
  const auto alive = alive_;
  const bool changed = session_->dispatch_key(node_id, kind, key, code, flags, key_code);
  if (alive->load(std::memory_order_acquire)) sync_views();
  return changed;
}

bool NveWinHost::is_node_disabled(uint32_t node_id) const noexcept {
  for (const WinNodeVisual* v = tree_.find(node_id); v != nullptr; v = tree_.find(v->parent)) {
    if (v->interaction.disabled) return true;
  }
  return false;
}

uint32_t NveWinHost::key_target(uint32_t start_id, NveInteractionEvent kind) const noexcept {
  if (start_id != 0u) {
    if (is_node_disabled(start_id)) return 0u;
    for (const WinNodeVisual* v = tree_.find(start_id); v != nullptr; v = tree_.find(v->parent)) {
      if (v->wants(kind)) return v->id;
    }
    return start_id;
  }
  return tree_.root_id();
}

bool NveWinHost::set_focused_node(uint32_t node_id) {
  if (syncing_) return false;
  if (node_id != 0u && (tree_.find(node_id) == nullptr || is_node_disabled(node_id))) return false;
  if (focused_ == node_id) return false;
  const uint32_t prev = std::exchange(focused_, node_id);
  const auto alive = alive_;
  bool changed = false;
  if (prev != 0u) {
    if (const WinNodeVisual* pv = tree_.find(prev); pv != nullptr && pv->wants(NVE_INTERACTION_EVENT_BLUR)) {
      changed |= dispatch(session_->events().focus(prev, false));
      if (!alive->load(std::memory_order_acquire)) return changed;
    }
  }
  if (focused_ != 0u) {
    if (const WinNodeVisual* nv = tree_.find(focused_); nv != nullptr && nv->wants(NVE_INTERACTION_EVENT_FOCUS)) {
      changed |= dispatch(session_->events().focus(focused_, true));
    }
  }
  return changed;
}

uint32_t NveWinHost::step_tab_focus(bool reverse) { return step_tab_focus_inner(reverse, nullptr); }

uint32_t NveWinHost::step_tab_focus_inner(bool reverse, bool* modal_trapped) {
  if (modal_trapped != nullptr) *modal_trapped = false;
  // Scope traversal to the topmost visible modal subtree when one is mounted.
  uint32_t modal_root = 0u;
  find_topmost_modal(tree_, tree_.root_children(), true, modal_root);

  std::vector<TabEntry> candidates;
  if (modal_root != 0u) {
    const uint32_t single[] = {modal_root};
    collect_tab_candidates(tree_, single, true, candidates);
  } else {
    collect_tab_candidates(tree_, tree_.root_children(), true, candidates);
  }
  if (candidates.empty()) {
    if (modal_root != 0u && modal_trapped != nullptr) *modal_trapped = true;
    return 0u;
  }

  std::stable_sort(candidates.begin(), candidates.end(), [](const TabEntry& a, const TabEntry& b) {
    const bool a_pos = a.tab_index > 0;
    const bool b_pos = b.tab_index > 0;
    if (a_pos != b_pos) return a_pos;
    if (a_pos && a.tab_index != b.tab_index) return a.tab_index < b.tab_index;
    return a.order < b.order;
  });

  size_t next_idx = reverse ? candidates.size() - 1u : 0u;
  for (size_t i = 0u; i < candidates.size(); ++i) {
    if (candidates[i].id == focused_) {
      next_idx = reverse ? (i == 0u ? candidates.size() - 1u : i - 1u) : ((i + 1u) % candidates.size());
      break;
    }
  }
  const uint32_t next = candidates[next_idx].id;
  (void)set_focused_node(next);
  return next;
}

KeyResult NveWinHost::handle_key(bool pressed, std::string_view key, std::string_view code, uint32_t flags,
                                 uint32_t key_code, bool from_text_input) {
  KeyResult r;
  r.focused_node = focused_;
  if (syncing_) return r;
  const auto alive = alive_;

  const NveInteractionEvent kind = pressed ? NVE_INTERACTION_EVENT_KEY_DOWN : NVE_INTERACTION_EVENT_KEY_UP;
  const uint32_t target = key_target(focused_, kind);
  if (target != 0u) {
    const WinNodeVisual* tv = tree_.find(target);
    const bool listened = tv != nullptr && tv->wants(kind);
    r.changed = dispatch_key(target, kind, key, code, flags, key_code) || r.changed;
    r.dispatched = true;
    r.handled = r.handled || listened || r.changed;
    if (!alive->load(std::memory_order_acquire)) return r;
  }

  const bool no_cmd =
      (flags & (NVE_EVENT_FLAG_MOD_CTRL | NVE_EVENT_FLAG_MOD_ALT | NVE_EVENT_FLAG_MOD_META)) == 0u;
  if (pressed && no_cmd && key == "Tab") {
    bool trapped = false;
    const uint32_t next = step_tab_focus_inner((flags & NVE_EVENT_FLAG_MOD_SHIFT) != 0u, &trapped);
    if (!alive->load(std::memory_order_acquire)) return r;
    r.focused_node = next != 0u ? next : focused_;
    r.handled = r.handled || next != 0u || trapped;
    return r;
  }

  if (pressed && !from_text_input && no_cmd && (key == "Enter" || key == " ") && focused_ != 0u &&
      !is_node_disabled(focused_)) {
    const WinNodeVisual* fv = tree_.find(focused_);
    if (fv != nullptr && fv->kind != host::NodeKind::TextInput) {
      if (fv->wants(NVE_INTERACTION_EVENT_PRESS)) {
        r.changed = simulate_press(fv->id, fv->local.width * 0.5f, fv->local.height * 0.5f) || r.changed;
        r.dispatched = true;
        r.handled = true;
        if (!alive->load(std::memory_order_acquire)) return r;
      } else if (key == "Enter" && host::openable_link(fv->href)) {
        r.href = fv->href;
        r.handled = true;
      }
    }
  }
  r.focused_node = focused_;
  return r;
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
