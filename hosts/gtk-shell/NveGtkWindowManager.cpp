#include "NveGtkWindowManager.hpp"

#include <algorithm>

namespace nve::gtk {

NveGtkWindowManager::~NveGtkWindowManager() {
  close_all();
}

gboolean NveGtkWindowManager::on_gtk_close_request(GtkWindow* window, gpointer user_data) {
  (void)window;
  auto* ctx = static_cast<CloseSignalCtx*>(user_data);
  if (ctx == nullptr || ctx->manager == nullptr) {
    return FALSE;
  }
  (void)ctx->manager->request_close(ctx->surface_id);
  // Window destruction is owned by `close_window` (or vetoed if `request_close` returned false).
  return TRUE;
}

NveGtkHost* NveGtkWindowManager::open_window(
    const GtkWindowSpec& spec,
    GtkApplication* app,
    bool present) {
  if (spec.surface_id == 0u || spec.width <= 0 || spec.height <= 0 ||
      entries_.contains(spec.surface_id)) {
    return nullptr;
  }
  GtkWindow* parent_window = nullptr;
  if (spec.parent_surface_id != 0u) {
    auto pit = entries_.find(spec.parent_surface_id);
    if (pit == entries_.end()) {
      return nullptr;
    }
    parent_window = pit->second.window;
  }

  GtkWidget* win_widget =
      app != nullptr ? gtk_application_window_new(app) : gtk_window_new();
  GtkWindow* window = GTK_WINDOW(win_widget);
  gtk_window_set_title(window, spec.title.empty() ? "NVE GTK4 Window" : spec.title.c_str());
  gtk_window_set_default_size(window, spec.width, spec.height);
  if (parent_window != nullptr) {
    gtk_window_set_transient_for(window, parent_window);
    if (spec.modal) {
      gtk_window_set_modal(window, TRUE);
    }
  }

  auto host = std::make_unique<NveGtkHost>(GtkHostConfig{
      .surface_id = spec.surface_id,
      .width = static_cast<float>(spec.width),
      .height = static_cast<float>(spec.height),
      .scale_factor = 1.0f,
  });
  host->attach_window_resize(window);
  gtk_window_set_child(window, host->host_widget());

  auto ctx = std::make_unique<CloseSignalCtx>(CloseSignalCtx{this, spec.surface_id});
  const gulong close_handler = g_signal_connect(
      window, "close-request", gtk_detail::to_gcallback(&on_gtk_close_request), ctx.get());

  NveGtkHost* raw_host = host.get();
  Entry entry{};
  entry.spec = spec;
  entry.spec.cascade_close = spec.cascade_close || spec.modal;
  entry.window = window;
  entry.host = std::move(host);
  entry.signal_ctx = std::move(ctx);
  entry.close_handler = close_handler;

  entries_.emplace(spec.surface_id, std::move(entry));
  if (spec.parent_surface_id != 0u) {
    entries_.at(spec.parent_surface_id).children.push_back(spec.surface_id);
  }
  focused_surface_id_ = spec.surface_id;

  if (present) {
    gtk_window_present(window);
  }
  return raw_host;
}

GtkWindow* NveGtkWindowManager::window_for_surface(uint32_t surface_id) const noexcept {
  auto it = entries_.find(surface_id);
  return it != entries_.end() ? it->second.window : nullptr;
}

NveGtkHost* NveGtkWindowManager::host_for_surface(uint32_t surface_id) noexcept {
  auto it = entries_.find(surface_id);
  return it != entries_.end() ? it->second.host.get() : nullptr;
}

const NveGtkHost* NveGtkWindowManager::host_for_surface(uint32_t surface_id) const noexcept {
  auto it = entries_.find(surface_id);
  return it != entries_.end() ? it->second.host.get() : nullptr;
}

uint32_t NveGtkWindowManager::active_modal_blocker(uint32_t surface_id) const noexcept {
  auto it = entries_.find(surface_id);
  if (it == entries_.end()) {
    return 0u;
  }
  const auto& children = it->second.children;
  for (auto rit = children.rbegin(); rit != children.rend(); ++rit) {
    const uint32_t deeper = active_modal_blocker(*rit);
    if (deeper != 0u) {
      return deeper;
    }
    auto cit = entries_.find(*rit);
    if (cit != entries_.end() && cit->second.spec.modal) {
      return *rit;
    }
  }
  return 0u;
}

bool NveGtkWindowManager::focus_window(uint32_t surface_id) {
  auto it = entries_.find(surface_id);
  if (it == entries_.end() || active_modal_blocker(surface_id) != 0u) {
    return false;
  }
  focused_surface_id_ = surface_id;
  if (it->second.window != nullptr && gtk_widget_get_visible(GTK_WIDGET(it->second.window))) {
    gtk_window_present(it->second.window);
  }
  return true;
}

bool NveGtkWindowManager::set_title(uint32_t surface_id, std::string_view title) {
  auto it = entries_.find(surface_id);
  if (it == entries_.end() || it->second.window == nullptr) {
    return false;
  }
  it->second.spec.title.assign(title.data(), title.size());
  gtk_window_set_title(it->second.window, it->second.spec.title.c_str());
  return true;
}

bool NveGtkWindowManager::set_bounds(uint32_t surface_id, int width, int height) {
  auto it = entries_.find(surface_id);
  if (it == entries_.end() || it->second.window == nullptr || width <= 0 || height <= 0) {
    return false;
  }
  it->second.spec.width = width;
  it->second.spec.height = height;
  gtk_window_set_default_size(it->second.window, width, height);
  if (it->second.host != nullptr) {
    it->second.host->set_surface_size(static_cast<float>(width), static_cast<float>(height), 1.0f);
  }
  return true;
}

void NveGtkWindowManager::collect_cascade_post_order(
    uint32_t surface_id,
    std::vector<uint32_t>& out) const {
  auto it = entries_.find(surface_id);
  if (it == entries_.end()) {
    return;
  }
  for (uint32_t cid : it->second.children) {
    auto cit = entries_.find(cid);
    if (cit != entries_.end() && cit->second.spec.cascade_close) {
      collect_cascade_post_order(cid, out);
    }
  }
  out.push_back(surface_id);
}

bool NveGtkWindowManager::request_close(uint32_t surface_id) {
  if (!entries_.contains(surface_id)) {
    return false;
  }
  if (should_close_ && !should_close_(surface_id)) {
    return false;
  }
  return close_window(surface_id);
}

bool NveGtkWindowManager::close_window(uint32_t surface_id) {
  auto root_it = entries_.find(surface_id);
  if (root_it == entries_.end()) {
    return false;
  }
  const uint32_t fallback_parent = root_it->second.spec.parent_surface_id;
  std::vector<uint32_t> order;
  collect_cascade_post_order(surface_id, order);

  bool focus_closed = false;
  std::vector<uint32_t> closed;
  closed.reserve(order.size());

  for (uint32_t sid : order) {
    auto it = entries_.find(sid);
    if (it == entries_.end()) {
      continue;
    }
    const uint32_t pid = it->second.spec.parent_surface_id;
    if (pid != 0u) {
      if (auto pit = entries_.find(pid); pit != entries_.end()) {
        auto& kids = pit->second.children;
        kids.erase(std::remove(kids.begin(), kids.end(), sid), kids.end());
      }
    }
    for (uint32_t remaining_cid : it->second.children) {
      if (auto cit = entries_.find(remaining_cid); cit != entries_.end()) {
        cit->second.spec.parent_surface_id = 0u;
      }
    }
    GtkWindow* win = it->second.window;
    if (win != nullptr && it->second.close_handler != 0) {
      g_signal_handler_disconnect(win, it->second.close_handler);
      it->second.close_handler = 0;
    }
    if (win != nullptr) {
      gtk_window_set_child(win, nullptr);
    }
    it->second.host.reset();
    if (win != nullptr) {
      gtk_window_destroy(win);
    }
    entries_.erase(it);
    if (focused_surface_id_ == sid) {
      focused_surface_id_ = 0u;
      focus_closed = true;
    }
    closed.push_back(sid);
  }

  if (focus_closed) {
    if (fallback_parent != 0u && entries_.contains(fallback_parent) &&
        active_modal_blocker(fallback_parent) == 0u) {
      focused_surface_id_ = fallback_parent;
    } else if (!entries_.empty()) {
      focused_surface_id_ = entries_.rbegin()->first;
    }
  }

  if (on_window_closed_) {
    for (uint32_t sid : closed) {
      on_window_closed_(sid);
    }
  }
  if (entries_.empty() && on_all_closed_) {
    on_all_closed_();
  }
  return true;
}

void NveGtkWindowManager::close_all() {
  const auto ids = active_surface_ids();
  for (uint32_t sid : ids) {
    if (entries_.contains(sid)) {
      (void)close_window(sid);
    }
  }
}

std::vector<uint32_t> NveGtkWindowManager::active_surface_ids() const {
  std::vector<uint32_t> ids;
  ids.reserve(entries_.size());
  for (const auto& [sid, _] : entries_) {
    ids.push_back(sid);
  }
  return ids;
}

}  // namespace nve::gtk
