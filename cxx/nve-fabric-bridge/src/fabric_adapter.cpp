#include "nve_abi.h"
#include "command_decoder.hpp"
#include "mock_jsi_recorder.hpp"

#include <chrono>
#include <string>
#include <unordered_map>
#include <vector>

#ifdef NVE_WITH_RN_FABRIC
#include <react/renderer/uimanager/UIManager.h>
#include <react/renderer/mounting/ShadowTree.h>
#include <react/renderer/core/EventListener.h>
#endif

namespace nve {

struct ShadowNodeEntry {
  uint32_t node_id{};
  uint16_t kind{};
  std::string custom_name;
  std::string text;
  LayoutProps layout{};
  StyleProps style{};
  uint64_t event_mask{0};
  std::vector<uint32_t> children;
};

class FabricHost {
 public:
  explicit FabricHost(NveSurfaceConfig config) : config_(config) {
    nodes_.emplace(NVE_ROOT_NODE_ID, ShadowNodeEntry{.node_id = NVE_ROOT_NODE_ID, .kind = NVE_COMP_ROOT});
    event_ring_.clear();
  }

  int32_t submit_frame(NveBufferSlice slice, NveFrameStats* out_stats) {
    using Clock = std::chrono::steady_clock;
    const auto t0 = Clock::now();

    BinaryReader r(slice.ptr, slice.len);
    Header h{};
    if (!r.read(h.magic) || !r.read(h.version) || !r.read(h.flags) || !r.read(h.surface_id) ||
        !r.read(h.frame_id) || !r.read(h.op_count)) {
      return -1;
    }
    if (h.magic != NVE_ABI_MAGIC || h.version != NVE_ABI_VERSION) {
      return -2;
    }

    uint32_t mutations = 0;
    bool committed = false;

    for (uint32_t i = 0; i < h.op_count; ++i) {
      uint8_t op = 0;
      if (!r.read(op)) return -3;
      switch (static_cast<NveOpcode>(op)) {
        case NVE_OP_CREATE_NODE: {
          uint32_t id = 0;
          uint16_t kind = 0;
          uint16_t name_len = 0;
          std::string_view name;
          if (!r.read(id) || !r.read(kind) || !r.read(name_len) || !r.read_string(name_len, name)) {
            return -4;
          }
          nodes_[id] = ShadowNodeEntry{
              .node_id = id,
              .kind = kind,
              .custom_name = std::string(name),
          };
          ++mutations;
          break;
        }
        case NVE_OP_REMOVE_NODE: {
          uint32_t id = 0;
          if (!r.read(id)) return -4;
          nodes_.erase(id);
          ++mutations;
          break;
        }
        case NVE_OP_SET_CHILDREN: {
          uint32_t parent_id = 0;
          uint32_t count = 0;
          if (!r.read(parent_id) || !r.read(count)) return -4;
          std::vector<uint32_t> children(count);
          for (uint32_t c = 0; c < count; ++c) {
            if (!r.read(children[c])) return -4;
          }
          auto it = nodes_.find(parent_id);
          if (it == nodes_.end()) return -5;
          it->second.children = std::move(children);
          ++mutations;
          break;
        }
        case NVE_OP_SET_TEXT: {
          uint32_t id = 0;
          uint32_t len = 0;
          std::string_view text;
          if (!r.read(id) || !r.read(len) || !r.read_string(len, text)) return -4;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.text = std::string(text);
          ++mutations;
          break;
        }
        case NVE_OP_SET_LAYOUT: {
          uint32_t id = 0;
          LayoutProps layout{};
          if (!r.read(id) || !r.read_layout(layout)) return -4;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.layout = layout;
          ++mutations;
          break;
        }
        case NVE_OP_SET_STYLE: {
          uint32_t id = 0;
          StyleProps style{};
          if (!r.read(id) || !r.read_style(style)) return -4;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.style = style;
          ++mutations;
          break;
        }
        case NVE_OP_SET_RAW_PROP: {
          uint32_t id = 0;
          uint16_t key_len = 0;
          std::string_view key;
          uint8_t tag = 0;
          if (!r.read(id) || !r.read(key_len) || !r.read_string(key_len, key) || !r.read(tag)) {
            return -4;
          }
          switch (tag) {
            case 0: break;
            case 1: if (!r.skip(1)) return -4; break;
            case 2:
            case 3: if (!r.skip(8)) return -4; break;
            case 4:
            case 5: {
              uint32_t vlen = 0;
              if (!r.read(vlen) || !r.skip(vlen)) return -4;
              break;
            }
            default: return -6;
          }
          ++mutations;
          break;
        }
        case NVE_OP_SET_EVENT_MASK: {
          uint32_t id = 0;
          uint64_t mask = 0;
          if (!r.read(id) || !r.read(mask)) return -4;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.event_mask = mask;
          break;
        }
        case NVE_OP_DISPATCH_COMMAND: {
          uint32_t id = 0;
          uint16_t nlen = 0;
          uint32_t plen = 0;
          std::string_view name;
          if (!r.read(id) || !r.read(nlen) || !r.read_string(nlen, name) || !r.read(plen) ||
              !r.skip(plen)) {
            return -4;
          }
          break;
        }
        case NVE_OP_COMMIT: {
          uint8_t sync_mount = 0;
          if (!r.read(sync_mount)) return -4;
          committed = true;
          break;
        }
        default:
          return -7;
      }
    }

    const auto decode_us =
        static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now() - t0).count());

    uint64_t layout_us = 0;
    if (committed) {
      // Phase 1B hooks directly into facebook::react::ShadowTree::commit + Yoga layout here
      // when NVE_WITH_RN_FABRIC=1 is configured with prebuilt ReactCommon / Yoga.
      layout_us = 0;
    }

    if (out_stats != nullptr) {
      out_stats->op_count = h.op_count;
      out_stats->mutation_count = mutations;
      out_stats->decode_us = decode_us;
      out_stats->layout_us = layout_us;
      out_stats->diff_us = 0;
      out_stats->mount_us = 0;
    }
    return 0;
  }

  NveBufferSlice drain_events() {
    drained_snapshot_ = event_ring_.finalize();
    event_ring_.clear();
    return NveBufferSlice{drained_snapshot_.data(), drained_snapshot_.size()};
  }

  uint32_t node_count() const { return static_cast<uint32_t>(nodes_.size()); }

 private:
  NveSurfaceConfig config_;
  std::unordered_map<uint32_t, ShadowNodeEntry> nodes_;
  EventRingWriter event_ring_;
  std::vector<uint8_t> drained_snapshot_;
};

} // namespace nve

extern "C" {

NveFabricHostHandle nve_fabric_host_create(NveSurfaceConfig config) {
  return new nve::FabricHost(config);
}

void nve_fabric_host_destroy(NveFabricHostHandle host) {
  delete static_cast<nve::FabricHost*>(host);
}

int32_t nve_fabric_host_submit_frame(
    NveFabricHostHandle host,
    NveBufferSlice command_buffer,
    NveFrameStats* out_stats) {
  if (!host) return -100;
  return static_cast<nve::FabricHost*>(host)->submit_frame(command_buffer, out_stats);
}

NveBufferSlice nve_fabric_host_drain_events(NveFabricHostHandle host) {
  if (!host) return NveBufferSlice{nullptr, 0};
  return static_cast<nve::FabricHost*>(host)->drain_events();
}

uint32_t nve_fabric_host_node_count(NveFabricHostHandle host) {
  if (!host) return 0;
  return static_cast<nve::FabricHost*>(host)->node_count();
}

} // extern "C"
