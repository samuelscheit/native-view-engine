#pragma once

#include "NveWinHost.hpp"

#include <nve/host/nve2_writer.hpp>
#include <nve/host/testing/scratch_dir.hpp>
#include <nve/host/testing/scripted_app.hpp>

#include <cmath>
#include <cstdlib>
#include <iostream>
#include <memory>
#include <source_location>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace nve::win::test {

using host::testing::ScratchDir;
using host::testing::ScriptedApp;

inline bool approx(double a, double b, double eps = 1.5) { return std::fabs(a - b) <= eps; }

/// A host surface (the factory only fails without `libnve_runtime` / DirectWrite).
inline std::unique_ptr<NveWinHost> make_host(WinHostConfig config = {}) {
  std::string error;
  std::unique_ptr<NveWinHost> host = NveWinHost::create(config, &error);
  if (host == nullptr) {
    std::cerr << "NveWinHost::create failed: " << error << "\n";
    std::abort();
  }
  return host;
}

/// Renders the `ScriptedApp`'s pending ops and mirrors them into the visual tree.
inline void flush(NveWinHost& host) {
  (void)host.session().dispatch(ScriptedApp::flush_event(host.surface_id()));
  (void)host.pump();
}

/// Text node helper for scripted scenes.
inline void text_node(host::Nve2Writer& w, uint32_t id, std::string_view text) {
  w.create(id);
  (void)w.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, text);
}

/// Fixed-size box helper.
inline void sized(host::Nve2Writer& w, uint32_t id, float width, float height) {
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, host::length_points(width));
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, host::length_points(height));
  w.set_f32(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0.0f);
}

inline void events(host::Nve2Writer& w, uint32_t id, std::initializer_list<NveEventKind> kinds) {
  uint32_t mask = 0u;
  for (const NveEventKind k : kinds) mask |= host::event_bit(k);
  w.set(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENT_MASK, mask);
}

inline const WinNodeVisual& node(const NveWinHost& host, uint32_t id,
                                 std::source_location at = std::source_location::current()) {
  const WinNodeVisual* v = host.visual_tree().find(id);
  if (v == nullptr) {
    std::cerr << "no view for node " << id << " at " << at.file_name() << ":" << at.line() << "\n";
    std::abort();
  }
  return *v;
}

inline Rect abs_rect(const NveWinHost& host, uint32_t id) {
  const auto f = host.visual_tree().frame_of(id);
  NVE_CHECK(f.has_value());
  return f->rect;
}

inline std::pair<float, float> center(const NveWinHost& host, uint32_t id) {
  const Rect r = abs_rect(host, id);
  return {r.x + r.width * 0.5f, r.y + r.height * 0.5f};
}

/// Node ids in paint order.
inline std::vector<uint32_t> paint_order(const NveWinHost& host) {
  struct Order {
    std::vector<uint32_t> ids;
    bool enter(const WinNodeVisual& n, const PaintFrame&) {
      ids.push_back(n.id);
      return true;
    }
    void leave(const WinNodeVisual&, const PaintFrame&, bool) {}
  } order;
  host.visual_tree().walk(order);
  return order.ids;
}

}  // namespace nve::win::test
