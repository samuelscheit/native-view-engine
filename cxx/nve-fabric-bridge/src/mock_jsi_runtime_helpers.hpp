#pragma once

#if defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)

namespace nve {

inline void MockJsiRecorderRuntime::set_object_prop(
    ArenaNode* obj,
    NveStrSlice key,
    const facebook::jsi::Value& value) {
  BumpArena& target_arena = obj->is_global ? global_arena_ : arena_;
  const std::string_view key_sv = str_slice_view(key);
  const NveStrSlice stored_key = obj->is_global ? global_arena_.copy_str(key_sv) : key;

  ArenaNode* child_container = nullptr;
  const NveValue slot_val = jsi_value_to_slot(value, child_container);

  const int32_t existing = detail::find_jsi_prop_index(obj, key_sv);
  if (existing >= 0) {
    const uint32_t idx = static_cast<uint32_t>(existing);
    obj->map_entries[idx].value = slot_val;
    if (child_container != nullptr) {
      detail::ensure_jsi_child_nodes(obj, target_arena);
      obj->child_nodes[idx] = child_container;
      obj->has_nested_containers = true;
    } else if (obj->child_nodes != nullptr) {
      obj->child_nodes[idx] = nullptr;
    }
    return;
  }

  if (obj->len == std::numeric_limits<uint32_t>::max()) {
    return;
  }
  if (obj->len >= obj->cap) {
    detail::grow_jsi_object_storage(obj, target_arena);
  }

  const uint32_t idx = obj->len++;
  obj->map_entries[idx] = NveMapEntry{stored_key, slot_val};
  if (child_container != nullptr) {
    detail::ensure_jsi_child_nodes(obj, target_arena);
    obj->child_nodes[idx] = child_container;
    obj->has_nested_containers = true;
  } else if (obj->child_nodes != nullptr) {
    obj->child_nodes[idx] = nullptr;
  }
}

inline NveValue MockJsiRecorderRuntime::jsi_value_to_slot(
    const facebook::jsi::Value& v,
    ArenaNode*& out_container) {
  out_container = nullptr;
  if (v.isUndefined() || v.isNull()) return make_null_value();
  if (v.isBool()) return make_bool_value(v.getBool());
  if (v.isNumber()) {
    const double d = v.getNumber();
    return std::isfinite(d) ? make_f64_value(d) : make_null_value();
  }
  const auto* pv = static_cast<const ArenaPointerValue*>(getPointerValue(v));
  if (!pv || !pv->node) return make_null_value();
  ArenaNode* n = pv->node;
  switch (n->kind) {
    case NodeKind::String:
    case NodeKind::Symbol:
      return make_str_value(n->str_val);
    case NodeKind::BigIntSigned:
      return make_i64_value(n->i64_val);
    case NodeKind::BigIntUnsigned:
      return make_i64_value(static_cast<int64_t>(n->u64_val));
    case NodeKind::ArrayBuffer: {
      if (n->side_slot < buffers_.size() && buffers_[n->side_slot]) {
        const auto& buf = buffers_[n->side_slot];
        return make_bytes_value(NveBufferSlice{buf->data(), buf->size()});
      }
      return make_null_value();
    }
    case NodeKind::Array:
      out_container = n;
      return make_array_value(n->array_elems, n->len);
    case NodeKind::Object:
    case NodeKind::HostObject:
      out_container = n;
      return make_map_value(n->map_entries, n->len);
    case NodeKind::Bool:
      return make_bool_value(n->bool_val);
    case NodeKind::Number:
      return std::isfinite(n->num_val) ? make_f64_value(n->num_val) : make_null_value();
    case NodeKind::Undefined:
    case NodeKind::Null:
    case NodeKind::HostFunction:
    default:
      return make_null_value();
  }
}

inline NveValue MockJsiRecorderRuntime::materialize_jsi_value(
    const facebook::jsi::Value& v,
    uint32_t depth) {
  if (v.isUndefined() || v.isNull()) return make_null_value();
  if (v.isBool()) return make_bool_value(v.getBool());
  if (v.isNumber()) {
    const double d = v.getNumber();
    return std::isfinite(d) ? make_f64_value(d) : make_null_value();
  }
  const auto* pv = static_cast<const ArenaPointerValue*>(getPointerValue(v));
  return pv ? detail::materialize_jsi_arena_node(pv->node, buffers_, depth) : make_null_value();
}

inline facebook::jsi::Value MockJsiRecorderRuntime::lookup_prop(
    const ArenaNode* obj,
    std::string_view key) {
  const int32_t idx = detail::find_jsi_prop_index(obj, key);
  if (idx < 0) return facebook::jsi::Value::undefined();
  const uint32_t u = static_cast<uint32_t>(idx);
  if (obj->child_nodes != nullptr && obj->child_nodes[u] != nullptr) {
    return node_to_jsi_value(obj->child_nodes[u]);
  }
  return nve_value_to_jsi_value(obj->map_entries[u].value);
}

inline facebook::jsi::Value MockJsiRecorderRuntime::node_to_jsi_value(ArenaNode* n) {
  if (!n) return facebook::jsi::Value::undefined();
  switch (n->kind) {
    case NodeKind::Undefined:
      return facebook::jsi::Value::undefined();
    case NodeKind::Null:
      return facebook::jsi::Value::null();
    case NodeKind::Bool:
      return facebook::jsi::Value(n->bool_val);
    case NodeKind::Number:
      return facebook::jsi::Value(n->num_val);
    case NodeKind::BigIntSigned:
    case NodeKind::BigIntUnsigned:
      return facebook::jsi::Value(
          make<facebook::jsi::BigInt>(arena_.emplace<ArenaPointerValue>(n)));
    case NodeKind::String:
      return facebook::jsi::Value(
          make<facebook::jsi::String>(arena_.emplace<ArenaPointerValue>(n)));
    case NodeKind::Symbol:
      return facebook::jsi::Value(
          make<facebook::jsi::Symbol>(arena_.emplace<ArenaPointerValue>(n)));
    case NodeKind::Object:
    case NodeKind::Array:
    case NodeKind::ArrayBuffer:
    case NodeKind::HostObject:
    case NodeKind::HostFunction:
      return facebook::jsi::Value(
          make<facebook::jsi::Object>(arena_.emplace<ArenaPointerValue>(n)));
  }
  return facebook::jsi::Value::undefined();
}

inline facebook::jsi::Value MockJsiRecorderRuntime::nve_value_to_jsi_value(const NveValue& val) {
  switch (val.tag) {
    case NVE_VAL_NULL:
      return facebook::jsi::Value::null();
    case NVE_VAL_BOOL:
      return facebook::jsi::Value(val.data.bool_val != 0u);
    case NVE_VAL_I64:
      return facebook::jsi::Value(createBigIntFromInt64(val.data.i64_val));
    case NVE_VAL_F64:
      return facebook::jsi::Value(val.data.f64_val);
    case NVE_VAL_STR:
      return facebook::jsi::Value(
          createStringFromUtf8(val.data.str_val.ptr, val.data.str_val.len));
    default:
      return facebook::jsi::Value::null();
  }
}

} // namespace nve

#endif // defined(NVE_WITH_RN_JSI) || defined(NVE_WITH_RN_FABRIC)
