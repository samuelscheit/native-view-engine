// Test support for `RetainedTree` suites: `Recorder` records every adapter call into plain views
// keyed by node id; `MovingRecorder` adds the optional geometry-only `move`. `TreeHarness` drives a
// live `HostSession` with `Nve2Writer` frames and mirrors each published scene into a tree.
#pragma once

#include <cstdio>
#include <map>
#include <memory>
#include <span>
#include <string>
#include <vector>

#include <nve/host/host_session.hpp>
#include <nve/host/nve2_writer.hpp>
#include <nve/host/retained_tree.hpp>

namespace nve::host::testing {

/// Records every adapter call; views are plain structs keyed by node id.
struct Recorder {
  struct View {
    uint32_t id = 0u;
    NodeKind kind = NodeKind::View;
    std::string text;
    TextStyle style;
    NveLayoutRect local{};
    uint32_t background = 0u;
    uint32_t image_src = 0u;
    std::string value;
    std::string label;
    std::string family;  // owned copy: the scene's text slab moves between frames
    std::vector<uint32_t> children;
    uint32_t attached_to = 0u;  // parent view id the adapter linked this view under (1 = root)
    int updates = 0;
    int moves = 0;
  };
  using Handle = View*;

  std::map<uint32_t, View> views;
  std::vector<uint32_t> root_children;
  int creates = 0;
  int destroys = 0;
  int root_updates = 0;

  uint32_t tag(const NodeData& n) { return static_cast<uint32_t>(n.kind); }
  Handle create(const NodeData& n) {
    ++creates;
    View& v = views[n.id];
    v = View{};
    v.id = n.id;
    return &v;
  }
  void update(Handle& h, const NodeData& n) {
    h->kind = n.kind;
    h->text = std::string(n.text);
    h->style = n.text_style;
    h->family = std::string(n.text_style.font_family);
    h->style.font_family = {};  // do not keep a view into the scene
    h->local = n.local;
    h->background = n.visual.background;
    h->image_src = n.image.src;
    h->value = std::string(n.input.value);
    h->label = std::string(n.a11y.label);
    ++h->updates;
  }
  void update_root(const NodeData&) { ++root_updates; }
  void destroy(uint32_t id, Handle&) {
    ++destroys;
    views.erase(id);
  }
  void set_children(const NodeData&, Handle* parent, std::span<Handle* const> kids) {
    std::vector<uint32_t>& out = parent != nullptr ? (*parent)->children : root_children;
    out.clear();
    for (Handle* k : kids) {
      out.push_back((*k)->id);
      (*k)->attached_to = parent != nullptr ? (*parent)->id : 1u;
    }
  }
};

/// `Recorder` plus `move`: geometry-only changes are recorded apart from `update`s.
struct MovingRecorder : Recorder {
  int moves = 0;
  void move(Handle& h, const NodeData& n) {
    h->local = n.local;
    ++h->moves;
    ++moves;
  }
};

template <class Adapter>
struct TreeHarness {
  std::unique_ptr<HostSession> host = HostSession::create(RuntimeSession::default_config(1u, 400.0f, 300.0f));
  Nve2Writer w;
  SceneTables tables;
  RetainedTree<Adapter> tree;
  Adapter rec;
  uint64_t frame = 0u;

  bool submit() {
    const auto bytes = w.finish(++frame);
    if (host->runtime().submit_nve2(bytes) != NVE_RUNTIME_OK) {
      fprintf(stderr, "submit_nve2: %s\n", host->last_error().c_str());
      return false;
    }
    (void)host->pump();
    return true;
  }
  size_t sync() {
    auto guard = host->runtime().acquire_scene();
    if (!guard) return 0u;
    return tree.sync(guard->reader(), tables, rec, host->source_generation());
  }
};

}  // namespace nve::host::testing
