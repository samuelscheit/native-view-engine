#include "NveWinWindowManager.hpp"

#include <algorithm>

namespace nve::win {

NveWinWindowManager::~NveWinWindowManager() {
  close_all();
}

bool NveWinWindowManager::register_entry(const WinManagedWindowSpec& spec, Entry&& entry) {
  if (spec.surface_id == 0u || spec.width <= 0 || spec.height <= 0 ||
      entries_.contains(spec.surface_id)) {
    return false;
  }
  if (spec.parent_surface_id != 0u && !entries_.contains(spec.parent_surface_id)) {
    return false;
  }
  entry.spec = spec;
  entry.spec.cascade_close = spec.cascade_close || spec.modal;
  const uint32_t sid = spec.surface_id;
  const uint32_t pid = spec.parent_surface_id;
  entries_.emplace(sid, std::move(entry));
  if (pid != 0u) {
    entries_.at(pid).children.push_back(sid);
  }
  focused_surface_id_ = sid;
  quit_posted_ = false;
  return true;
}

#if defined(_WIN32)
NveWinHostWindow* NveWinWindowManager::open_window(
    HINSTANCE instance,
    const WinManagedWindowSpec& spec,
    bool show_window) {
  if (spec.surface_id == 0u || spec.width <= 0 || spec.height <= 0 ||
      entries_.contains(spec.surface_id)) {
    return nullptr;
  }
  if (spec.parent_surface_id != 0u && !entries_.contains(spec.parent_surface_id)) {
    return nullptr;
  }

  auto win = std::make_unique<NveWinHostWindow>();
  WinHostWindowOptions opts{};
  opts.title = spec.title;
  opts.width = spec.width;
  opts.height = spec.height;
  opts.surface_id = spec.surface_id;
  opts.quit_on_destroy = false;
  if (!win->create(instance, opts)) {
    return nullptr;
  }
  if (spec.x != 0 || spec.y != 0) {
    win->set_bounds(spec.x, spec.y, spec.width, spec.height);
  }
  win->set_on_close_request([this](uint32_t sid) {
    if (should_close_ && !should_close_(sid)) {
      return false;
    }
    return true;
  });
  win->set_on_destroy([this](uint32_t sid) {
    if (!closing_internal_) {
      (void)close_window(sid);
    }
  });

  NveWinHostWindow* raw = win.get();
  Entry entry{};
  entry.window = std::move(win);
  if (!register_entry(spec, std::move(entry))) {
    return nullptr;
  }
  if (show_window) {
    raw->show(SW_SHOWDEFAULT);
  }
  return raw;
}

NveWinHostWindow* NveWinWindowManager::window_for_surface(uint32_t surface_id) noexcept {
  auto it = entries_.find(surface_id);
  return it != entries_.end() ? it->second.window.get() : nullptr;
}

const NveWinHostWindow* NveWinWindowManager::window_for_surface(uint32_t surface_id) const noexcept {
  auto it = entries_.find(surface_id);
  return it != entries_.end() ? it->second.window.get() : nullptr;
}

int NveWinWindowManager::run_message_loop() {
  MSG msg{};
  while (::GetMessageW(&msg, nullptr, 0, 0) > 0) {
    ::TranslateMessage(&msg);
    ::DispatchMessageW(&msg);
  }
  return static_cast<int>(msg.wParam);
}
#endif

NveRnwComponentHost* NveWinWindowManager::open_headless_window(const WinManagedWindowSpec& spec) {
  Entry entry{};
  entry.portable_host = std::make_unique<NveRnwComponentHost>();
  NveRnwComponentHost* raw = entry.portable_host.get();
  if (!register_entry(spec, std::move(entry))) {
    return nullptr;
  }
  return raw;
}

NveRnwComponentHost* NveWinWindowManager::host_for_surface(uint32_t surface_id) noexcept {
  auto it = entries_.find(surface_id);
  if (it == entries_.end()) {
    return nullptr;
  }
#if defined(_WIN32)
  if (it->second.window != nullptr) {
    return &it->second.window->host();
  }
#endif
  return it->second.portable_host.get();
}

const NveRnwComponentHost* NveWinWindowManager::host_for_surface(uint32_t surface_id) const noexcept {
  auto it = entries_.find(surface_id);
  if (it == entries_.end()) {
    return nullptr;
  }
#if defined(_WIN32)
  if (it->second.window != nullptr) {
    return &it->second.window->host();
  }
#endif
  return it->second.portable_host.get();
}

const WinManagedWindowSpec* NveWinWindowManager::spec_for_surface(uint32_t surface_id) const noexcept {
  auto it = entries_.find(surface_id);
  return it != entries_.end() ? &it->second.spec : nullptr;
}

uint32_t NveWinWindowManager::active_modal_blocker(uint32_t surface_id) const noexcept {
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

bool NveWinWindowManager::focus_window(uint32_t surface_id) {
  auto it = entries_.find(surface_id);
  if (it == entries_.end() || active_modal_blocker(surface_id) != 0u) {
    return false;
  }
  focused_surface_id_ = surface_id;
#if defined(_WIN32)
  if (it->second.window != nullptr) {
    it->second.window->focus();
  }
#endif
  return true;
}

bool NveWinWindowManager::set_title(uint32_t surface_id, std::string_view title) {
  auto it = entries_.find(surface_id);
  if (it == entries_.end()) {
    return false;
  }
  it->second.spec.title.assign(title.data(), title.size());
#if defined(_WIN32)
  if (it->second.window != nullptr) {
    it->second.window->set_title(title);
  }
#endif
  return true;
}

bool NveWinWindowManager::set_bounds(uint32_t surface_id, int x, int y, int width, int height) {
  auto it = entries_.find(surface_id);
  if (it == entries_.end() || width <= 0 || height <= 0) {
    return false;
  }
  it->second.spec.x = x;
  it->second.spec.y = y;
  it->second.spec.width = width;
  it->second.spec.height = height;
#if defined(_WIN32)
  if (it->second.window != nullptr) {
    it->second.window->set_bounds(x, y, width, height);
  }
#endif
  if (auto* host = host_for_surface(surface_id); host != nullptr && (host->is_valid() || host->is_paired())) {
    host->set_surface_size(static_cast<float>(width), static_cast<float>(height), 1.0f);
  }
  return true;
}

void NveWinWindowManager::collect_cascade_post_order(
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

bool NveWinWindowManager::request_close(uint32_t surface_id) {
  if (!entries_.contains(surface_id)) {
    return false;
  }
  if (should_close_ && !should_close_(surface_id)) {
    return false;
  }
  return close_window(surface_id);
}

bool NveWinWindowManager::close_window(uint32_t surface_id) {
  auto root_it = entries_.find(surface_id);
  if (root_it == entries_.end()) {
    return false;
  }
  const uint32_t fallback_parent = root_it->second.spec.parent_surface_id;
  std::vector<uint32_t> order;
  collect_cascade_post_order(surface_id, order);

  const bool prev_closing = closing_internal_;
  closing_internal_ = true;
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
#if defined(_WIN32)
    const bool had_native_win = it->second.window != nullptr;
    if (had_native_win) {
      it->second.window->set_on_destroy({});
      it->second.window->close();
    }
#endif
    entries_.erase(it);
    if (focused_surface_id_ == sid) {
      focused_surface_id_ = 0u;
      focus_closed = true;
    }
    closed.push_back(sid);
  }
  closing_internal_ = prev_closing;

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
  if (entries_.empty()) {
    quit_posted_ = true;
#if defined(_WIN32)
    ::PostQuitMessage(0);
#endif
    if (on_all_closed_) {
      on_all_closed_();
    }
  }
  return true;
}

void NveWinWindowManager::close_all() {
  const auto ids = active_surface_ids();
  for (uint32_t sid : ids) {
    if (entries_.contains(sid)) {
      (void)close_window(sid);
    }
  }
}

std::vector<uint32_t> NveWinWindowManager::active_surface_ids() const {
  std::vector<uint32_t> ids;
  ids.reserve(entries_.size());
  for (const auto& [sid, _] : entries_) {
    ids.push_back(sid);
  }
  return ids;
}

} // namespace nve::win
