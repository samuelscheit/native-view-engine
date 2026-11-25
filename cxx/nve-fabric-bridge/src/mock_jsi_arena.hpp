#pragma once

#include "nve_abi.generated.h"
#include "nve_value.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <memory>
#include <string_view>
#include <type_traits>
#include <vector>

#if defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)
#include <jsi/jsi.h>

namespace nve::detail {

inline constexpr uint32_t kJsiNoSideSlot = std::numeric_limits<uint32_t>::max();
inline constexpr uint32_t kJsiInitialObjectCap = 8u;

enum class JsiNodeKind : uint8_t {
  Undefined = 0,
  Null = 1,
  Bool = 2,
  Number = 3,
  BigIntSigned = 4,
  BigIntUnsigned = 5,
  String = 6,
  Symbol = 7,
  Object = 8,
  Array = 9,
  ArrayBuffer = 10,
  HostObject = 11,
  HostFunction = 12,
};

/// Trivially-destructible arena node storing `NveMapEntry[]` / `NveValue[]` directly in-place.
struct JsiArenaNode {
  JsiNodeKind kind{JsiNodeKind::Undefined};
  bool is_global{false};
  bool has_nested_containers{false};
  uint32_t len{0};
  uint32_t cap{0};
  uint32_t side_slot{kJsiNoSideSlot};
  uint32_t native_state_slot{kJsiNoSideSlot};

  union {
    bool bool_val;
    double num_val;
    int64_t i64_val;
    uint64_t u64_val;
    NveStrSlice str_val;
    NveMapEntry* map_entries;
    NveValue* array_elems;
  };

  /// Lazily allocated parallel child-node pointers only when an Object/Array stores nested
  /// JSI containers (allows O(1) finalize for flat objects and handles post-assignment mutation).
  JsiArenaNode** child_nodes{nullptr};

  JsiArenaNode() noexcept : u64_val(0) {}
};

static_assert(std::is_trivially_destructible_v<JsiArenaNode>);

inline JsiArenaNode* alloc_jsi_object_node(BumpArena& target_arena, bool is_global) {
  JsiArenaNode* n = target_arena.make<JsiArenaNode>();
  n->kind = JsiNodeKind::Object;
  n->is_global = is_global;
  n->len = 0;
  n->cap = kJsiInitialObjectCap;
  n->map_entries = target_arena.alloc_array<NveMapEntry>(kJsiInitialObjectCap);
  return n;
}

inline JsiArenaNode* alloc_jsi_string_node(
    BumpArena& target_arena,
    const char* str,
    size_t length) {
  JsiArenaNode* n = target_arena.make<JsiArenaNode>();
  n->kind = JsiNodeKind::String;
  if (length > 0 && str != nullptr) {
    auto* dst = static_cast<uint8_t*>(target_arena.alloc(length, 1u));
    std::memcpy(dst, str, length);
    n->str_val = NveStrSlice{dst, length};
  } else {
    n->str_val = NveStrSlice{nullptr, 0};
  }
  return n;
}

inline void ensure_jsi_child_nodes(JsiArenaNode* n, BumpArena& target_arena) {
  if (n->child_nodes == nullptr && n->cap > 0) {
    n->child_nodes = target_arena.alloc_array<JsiArenaNode*>(n->cap);
    std::memset(n->child_nodes, 0, sizeof(JsiArenaNode*) * n->cap);
  }
}

inline int32_t find_jsi_prop_index(const JsiArenaNode* obj, std::string_view key) noexcept {
  if (!obj || (obj->kind != JsiNodeKind::Object && obj->kind != JsiNodeKind::HostObject)) {
    return -1;
  }
  for (uint32_t i = 0; i < obj->len; ++i) {
    if (str_slice_view(obj->map_entries[i].key) == key) {
      return static_cast<int32_t>(i);
    }
  }
  return -1;
}

inline void grow_jsi_array_storage(JsiArenaNode* n, BumpArena& arena, size_t required) {
  constexpr size_t kMaxCap = static_cast<size_t>(std::numeric_limits<uint32_t>::max());
  const size_t doubled = (n->cap == 0u) ? size_t{4u} : (static_cast<size_t>(n->cap) * 2u);
  const uint32_t new_cap = static_cast<uint32_t>(std::min(kMaxCap, std::max(doubled, required)));
  auto* new_elems = arena.alloc_array<NveValue>(new_cap);
  if (n->len > 0 && n->array_elems != nullptr) {
    std::memcpy(new_elems, n->array_elems, sizeof(NveValue) * n->len);
  }
  for (uint32_t j = n->len; j < new_cap; ++j) {
    new_elems[j] = make_null_value();
  }
  if (n->child_nodes != nullptr) {
    auto** new_children = arena.alloc_array<JsiArenaNode*>(new_cap);
    std::memcpy(new_children, n->child_nodes, sizeof(JsiArenaNode*) * n->len);
    std::memset(new_children + n->len, 0, sizeof(JsiArenaNode*) * (new_cap - n->len));
    n->child_nodes = new_children;
  }
  n->array_elems = new_elems;
  n->cap = new_cap;
}

inline void grow_jsi_object_storage(JsiArenaNode* obj, BumpArena& target_arena) {
  constexpr size_t kMaxCap = static_cast<size_t>(std::numeric_limits<uint32_t>::max());
  const size_t grown = (obj->cap == 0u)
                           ? static_cast<size_t>(kJsiInitialObjectCap)
                           : (static_cast<size_t>(obj->cap) * 2u);
  const uint32_t new_cap = static_cast<uint32_t>(std::min(kMaxCap, grown));
  auto* new_entries = target_arena.alloc_array<NveMapEntry>(new_cap);
  if (obj->len > 0 && obj->map_entries != nullptr) {
    std::memcpy(new_entries, obj->map_entries, sizeof(NveMapEntry) * obj->len);
  }
  if (obj->child_nodes != nullptr) {
    auto** new_children = target_arena.alloc_array<JsiArenaNode*>(new_cap);
    std::memcpy(new_children, obj->child_nodes, sizeof(JsiArenaNode*) * obj->len);
    std::memset(new_children + obj->len, 0, sizeof(JsiArenaNode*) * (new_cap - obj->len));
    obj->child_nodes = new_children;
  }
  obj->map_entries = new_entries;
  obj->cap = new_cap;
}

/// Synchronize any nested container `{ptr, len}` headers (O(1) no-op for flat objects/arrays)
/// with a strict depth guard against cyclic JS objects.
inline NveValue materialize_jsi_arena_node(
    JsiArenaNode* n,
    const std::vector<std::shared_ptr<facebook::jsi::MutableBuffer>>& buffers,
    uint32_t depth) {
  if (!n || depth > NVE_BIN_VALUE_MAX_DEPTH) {
    return make_null_value();
  }
  switch (n->kind) {
    case JsiNodeKind::String:
    case JsiNodeKind::Symbol:
      return make_str_value(n->str_val);
    case JsiNodeKind::BigIntSigned:
      return make_i64_value(n->i64_val);
    case JsiNodeKind::BigIntUnsigned:
      return make_i64_value(static_cast<int64_t>(n->u64_val));
    case JsiNodeKind::ArrayBuffer: {
      if (n->side_slot < buffers.size() && buffers[n->side_slot]) {
        const auto& buf = buffers[n->side_slot];
        return make_bytes_value(NveBufferSlice{buf->data(), buf->size()});
      }
      return make_null_value();
    }
    case JsiNodeKind::Array: {
      if (n->has_nested_containers && n->child_nodes != nullptr) {
        for (uint32_t i = 0; i < n->len; ++i) {
          if (n->child_nodes[i] != nullptr) {
            n->array_elems[i] = materialize_jsi_arena_node(n->child_nodes[i], buffers, depth + 1u);
          }
        }
      }
      return make_array_value(n->array_elems, n->len);
    }
    case JsiNodeKind::Object:
    case JsiNodeKind::HostObject: {
      if (n->has_nested_containers && n->child_nodes != nullptr) {
        for (uint32_t i = 0; i < n->len; ++i) {
          if (n->child_nodes[i] != nullptr) {
            n->map_entries[i].value =
                materialize_jsi_arena_node(n->child_nodes[i], buffers, depth + 1u);
          }
        }
      }
      return make_map_value(n->map_entries, n->len);
    }
    case JsiNodeKind::Bool:
      return make_bool_value(n->bool_val);
    case JsiNodeKind::Number:
      return std::isfinite(n->num_val) ? make_f64_value(n->num_val) : make_null_value();
    case JsiNodeKind::Undefined:
    case JsiNodeKind::Null:
    case JsiNodeKind::HostFunction:
    default:
      return make_null_value();
  }
}

} // namespace nve::detail

#endif // defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)
