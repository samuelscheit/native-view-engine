#include "NveAndroidDirectChannel.hpp"

#include <array>
#include <cstring>
#include <string_view>

namespace nve::android_internal {

namespace {

/// Minimum valid `EventRing` wire frame is the 4-byte little-endian `event_count` header.
constexpr size_t kMinEventRingBytes = 4u;
constexpr size_t kEventHeaderBytes = 18u;
constexpr size_t kStackEventCapacity = 32u;

inline bool read_u16_le(const uint8_t* ptr, size_t len, size_t& pos, uint16_t& out) noexcept {
  if (pos > len || len - pos < 2u) return false;
  std::memcpy(&out, ptr + pos, sizeof(uint16_t));
  pos += 2u;
  return true;
}

inline bool read_u32_le(const uint8_t* ptr, size_t len, size_t& pos, uint32_t& out) noexcept {
  if (pos > len || len - pos < 4u) return false;
  std::memcpy(&out, ptr + pos, sizeof(uint32_t));
  pos += 4u;
  return true;
}

inline bool read_u64_le(const uint8_t* ptr, size_t len, size_t& pos, uint64_t& out) noexcept {
  if (pos > len || len - pos < 8u) return false;
  std::memcpy(&out, ptr + pos, sizeof(uint64_t));
  pos += 8u;
  return true;
}

inline bool read_f32_le(const uint8_t* ptr, size_t len, size_t& pos, float& out) noexcept {
  if (pos > len || len - pos < 4u) return false;
  std::memcpy(&out, ptr + pos, sizeof(float));
  pos += 4u;
  return true;
}

bool convert_bin_value(const nve::BinValueView& view, nve::BumpArena& arena, NveValue& out) noexcept {
  switch (view.tag()) {
    case NVE_VAL_NULL:
      out = nve::make_null_value();
      return true;
    case NVE_VAL_BOOL: {
      const auto v = view.as_bool();
      if (!v) return false;
      out = nve::make_bool_value(*v);
      return true;
    }
    case NVE_VAL_I64: {
      const auto v = view.as_i64();
      if (!v) return false;
      out = nve::make_i64_value(*v);
      return true;
    }
    case NVE_VAL_F64: {
      const auto v = view.as_f64();
      if (!v) return false;
      out = nve::make_f64_value(*v);
      return true;
    }
    case NVE_VAL_STR: {
      const auto v = view.as_str();
      if (!v) return false;
      out = nve::make_str_value(*v);
      return true;
    }
    case NVE_VAL_BYTES: {
      const auto v = view.as_bytes();
      if (!v) return false;
      out = nve::make_bytes_value(*v);
      return true;
    }
    case NVE_VAL_ARRAY: {
      const auto count = view.array_len();
      if (!count) return false;
      NveValue* elems = arena.alloc_array<NveValue>(*count);
      size_t idx = 0u;
      bool ok = true;
      if (!view.for_each_array([&](const nve::BinValueView& elem) {
            if (ok && !convert_bin_value(elem, arena, elems[idx++])) {
              ok = false;
            }
          })) {
        return false;
      }
      if (!ok || idx != *count) return false;
      out = nve::make_array_value(elems, *count);
      return true;
    }
    case NVE_VAL_MAP: {
      const auto count = view.map_len();
      if (!count) return false;
      NveMapEntry* entries = arena.alloc_array<NveMapEntry>(*count);
      size_t idx = 0u;
      bool ok = true;
      if (!view.for_each_map([&](std::string_view key, const nve::BinValueView& val) {
            if (!ok) return;
            entries[idx].key = nve::make_str_slice(key);
            if (!convert_bin_value(val, arena, entries[idx].value)) {
              ok = false;
              return;
            }
            ++idx;
          })) {
        return false;
      }
      if (!ok || idx != *count) return false;
      out = nve::make_map_value(entries, *count);
      return true;
    }
    default:
      return false;
  }
}

jobject ensure_scratch_buffer(
    JNIEnv* env,
    std::vector<uint8_t>& storage,
    CachedDirectBuffer& cached) noexcept {
  if (env == nullptr) {
    return nullptr;
  }
  if (storage.size() < kDefaultDirectScratchBytes) {
    storage.assign(kDefaultDirectScratchBytes, 0u);
  }
  (void)cached.sync(env, storage.data(), storage.size());
  return cached.global_ref != nullptr ? env->NewLocalRef(cached.global_ref) : nullptr;
}

jboolean dispatch_event_ring_slice(
    AndroidSurfaceSession* session,
    const uint8_t* ptr,
    size_t len) noexcept {
  if (!is_session_active(session) || ptr == nullptr || len < kMinEventRingBytes) {
    return JNI_FALSE;
  }
  size_t pos = 0u;
  uint32_t count = 0u;
  if (!read_u32_le(ptr, len, pos, count)) {
    return JNI_FALSE;
  }
  if (count == 0u) {
    return pos == len ? JNI_TRUE : JNI_FALSE;
  }
  if (static_cast<size_t>(count) > (len - pos) / kEventHeaderBytes) {
    return JNI_FALSE;
  }

  std::array<NveDirectEvent, kStackEventCapacity> stack_events{};
  std::vector<NveDirectEvent> heap_events;
  NveDirectEvent* events = stack_events.data();
  if (count > kStackEventCapacity) {
    heap_events.resize(count);
    events = heap_events.data();
  }

  session->props_arena.reset();
  for (uint32_t i = 0u; i < count; ++i) {
    NveDirectEvent ev{};
    if (!read_u32_le(ptr, len, pos, ev.surface_id) ||
        !read_u32_le(ptr, len, pos, ev.node_id) ||
        !read_u64_le(ptr, len, pos, ev.timestamp_us) ||
        len - pos < 2u) {
      return JNI_FALSE;
    }
    ev.kind = ptr[pos++];
    ev.payload_tag = ptr[pos++];
    ev.value = nve::make_null_value();

    switch (ev.payload_tag) {
      case NVE_EVENT_PAYLOAD_EMPTY:
        break;
      case NVE_EVENT_PAYLOAD_POINT:
        if (!read_f32_le(ptr, len, pos, ev.x) || !read_f32_le(ptr, len, pos, ev.y)) {
          return JNI_FALSE;
        }
        break;
      case NVE_EVENT_PAYLOAD_SCROLL:
      case NVE_EVENT_PAYLOAD_LAYOUT:
        if (!read_f32_le(ptr, len, pos, ev.x) ||
            !read_f32_le(ptr, len, pos, ev.y) ||
            !read_f32_le(ptr, len, pos, ev.width) ||
            !read_f32_le(ptr, len, pos, ev.height)) {
          return JNI_FALSE;
        }
        break;
      case NVE_EVENT_PAYLOAD_TEXT: {
        uint32_t tlen = 0u;
        if (!read_u32_le(ptr, len, pos, tlen) || len - pos < tlen) {
          return JNI_FALSE;
        }
        ev.text = NveStrSlice{tlen == 0u ? nullptr : ptr + pos, tlen};
        pos += tlen;
        break;
      }
      case NVE_EVENT_PAYLOAD_CUSTOM: {
        uint16_t nlen = 0u;
        if (!read_u16_le(ptr, len, pos, nlen) || len - pos < nlen) {
          return JNI_FALSE;
        }
        ev.name = NveStrSlice{nlen == 0u ? nullptr : ptr + pos, nlen};
        pos += nlen;
        uint32_t dlen = 0u;
        if (!read_u32_le(ptr, len, pos, dlen) || len - pos < dlen) {
          return JNI_FALSE;
        }
        const uint8_t* data_ptr = dlen == 0u ? nullptr : ptr + pos;
        pos += dlen;
        if (dlen > 0u) {
          if (const auto parsed = nve::BinValueView::parse(data_ptr, dlen)) {
            if (!convert_bin_value(*parsed, session->props_arena, ev.value)) {
              ev.value = nve::make_bytes_value(NveBufferSlice{data_ptr, dlen});
            }
          } else {
            ev.value = nve::make_bytes_value(NveBufferSlice{data_ptr, dlen});
          }
        }
        break;
      }
      case NVE_EVENT_PAYLOAD_KEY: {
        uint16_t klen = 0u;
        if (!read_u16_le(ptr, len, pos, klen) || len - pos < klen) return JNI_FALSE;
        ev.text = NveStrSlice{klen == 0u ? nullptr : ptr + pos, klen};
        pos += klen;
        uint16_t clen = 0u;
        if (!read_u16_le(ptr, len, pos, clen) || len - pos < clen) return JNI_FALSE;
        ev.name = NveStrSlice{clen == 0u ? nullptr : ptr + pos, clen};
        pos += clen;
        if (len - pos < 6u) return JNI_FALSE;
        ev.modifiers = ptr[pos++];
        if (!read_u32_le(ptr, len, pos, ev.key_code)) return JNI_FALSE;
        ev.event_flags = ptr[pos++] != 0u ? NVE_EVENT_FLAG_COMPOSING : 0u;
        break;
      }
      case NVE_EVENT_PAYLOAD_POINTER: {
        if (!read_f32_le(ptr, len, pos, ev.x) || !read_f32_le(ptr, len, pos, ev.y) ||
            len - pos < 2u) {
          return JNI_FALSE;
        }
        ev.event_flags = ptr[pos++];
        ev.modifiers = ptr[pos++];
        break;
      }
      case NVE_EVENT_PAYLOAD_WHEEL: {
        if (!read_f32_le(ptr, len, pos, ev.x) || !read_f32_le(ptr, len, pos, ev.y) ||
            len - pos < 1u) {
          return JNI_FALSE;
        }
        ev.modifiers = ptr[pos++];
        break;
      }
      default:
        return JNI_FALSE;
    }
    events[i] = ev;
  }
  if (pos != len) {
    return JNI_FALSE;
  }

  const NveEventBatch batch{events, count, 0u};
  if (session->session.bridge() != nullptr) {
    (void)session->session.dispatch_events(batch, /*auto_render=*/true);
    (void)session->session.drain_events();
    return JNI_TRUE;
  }
  if (session->session.is_paired()) {
    return session->session.dispatch_events(batch, /*auto_render=*/false) != 0u
               ? JNI_TRUE
               : JNI_FALSE;
  }
  return JNI_FALSE;
}

jboolean apply_props_slice(
    AndroidSurfaceSession* session,
    const uint8_t* ptr,
    size_t len) noexcept {
  if (session == nullptr || !session->session.is_valid() || ptr == nullptr || len == 0u) {
    return JNI_FALSE;
  }
  const auto parsed = nve::BinValueView::parse(ptr, len);
  if (!parsed || parsed->tag() != NVE_VAL_MAP) {
    return JNI_FALSE;
  }
  session->props_arena.reset();
  NveValue map_val{};
  if (!convert_bin_value(*parsed, session->props_arena, map_val) || map_val.tag != NVE_VAL_MAP) {
    return JNI_FALSE;
  }
  const bool applied = session->session.update_props(
      map_val.data.map_val.ptr,
      static_cast<uint32_t>(map_val.data.map_val.len),
      /*auto_render=*/true);
  if (applied) {
    (void)session->session.drain_events();
  }
  return applied ? JNI_TRUE : JNI_FALSE;
}

} // namespace

void release_direct_scratch_buffers(AndroidSurfaceSession* session, JNIEnv* env) noexcept {
  if (!session) {
    return;
  }
  session->event_scratch_buf.release(env);
  session->event_scratch.clear();
  session->event_scratch.shrink_to_fit();

  session->prop_scratch_buf.release(env);
  session->prop_scratch.clear();
  session->prop_scratch.shrink_to_fit();
}

jobject get_event_scratch_buffer(JNIEnv* env, jlong handle_ptr) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || env == nullptr) {
    return nullptr;
  }
  return ensure_scratch_buffer(env, session->event_scratch, session->event_scratch_buf);
}

jobject get_prop_scratch_buffer(JNIEnv* env, jlong handle_ptr) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || env == nullptr) {
    return nullptr;
  }
  return ensure_scratch_buffer(env, session->prop_scratch, session->prop_scratch_buf);
}

jboolean submit_events_direct(jlong handle_ptr, jint byte_len) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || byte_len < static_cast<jint>(kMinEventRingBytes)) {
    return JNI_FALSE;
  }
  const size_t len = static_cast<size_t>(byte_len);
  if (len > session->event_scratch.size()) {
    return JNI_FALSE;
  }
  return dispatch_event_ring_slice(session, session->event_scratch.data(), len);
}

jboolean submit_events_array(JNIEnv* env, jlong handle_ptr, jbyteArray event_ring_bytes) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (env == nullptr || !session || event_ring_bytes == nullptr || !is_session_active(session)) {
    return JNI_FALSE;
  }
  const jsize arr_len = env->GetArrayLength(event_ring_bytes);
  if (arr_len < static_cast<jsize>(kMinEventRingBytes)) {
    return JNI_FALSE;
  }
  jbyte* raw = env->GetByteArrayElements(event_ring_bytes, nullptr);
  if (raw == nullptr) {
    return JNI_FALSE;
  }
  const jboolean ok = dispatch_event_ring_slice(
      session, reinterpret_cast<const uint8_t*>(raw), static_cast<size_t>(arr_len));
  env->ReleaseByteArrayElements(event_ring_bytes, raw, JNI_ABORT);
  return ok;
}

jboolean set_props_direct(jlong handle_ptr, jint byte_len) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || byte_len <= 0) {
    return JNI_FALSE;
  }
  const size_t len = static_cast<size_t>(byte_len);
  if (len > session->prop_scratch.size()) {
    return JNI_FALSE;
  }
  return apply_props_slice(session, session->prop_scratch.data(), len);
}

jboolean set_props_array(JNIEnv* env, jlong handle_ptr, jbyteArray props_bytes) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (env == nullptr || !session || props_bytes == nullptr || !session->session.is_valid()) {
    return JNI_FALSE;
  }
  const jsize arr_len = env->GetArrayLength(props_bytes);
  if (arr_len <= 0) {
    return JNI_FALSE;
  }
  jbyte* raw = env->GetByteArrayElements(props_bytes, nullptr);
  if (raw == nullptr) {
    return JNI_FALSE;
  }
  const jboolean ok =
      apply_props_slice(session, reinterpret_cast<const uint8_t*>(raw), static_cast<size_t>(arr_len));
  env->ReleaseByteArrayElements(props_bytes, raw, JNI_ABORT);
  return ok;
}

} // namespace nve::android_internal

// =============================================================================
// JNI Exports — dev.nve.android.NveMainActivity
// =============================================================================

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetEventScratchBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_event_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetPropScratchBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_prop_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::submit_events_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::set_props_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSubmitEvents(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jbyteArray event_ring_bytes) {
  return nve::android_internal::submit_events_array(env, handle_ptr, event_ring_bytes);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSetPropsBinary(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jbyteArray props_bytes) {
  return nve::android_internal::set_props_array(env, handle_ptr, props_bytes);
}

// =============================================================================
// JNI Exports — dev.nve.compose.NveJniBridge
// =============================================================================

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_compose_NveJniBridge_nativeGetEventScratchBuffer(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_event_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_compose_NveJniBridge_nativeGetPropScratchBuffer(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_prop_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSubmitEventsDirect(
    JNIEnv* /*env*/,
    jobject /*thiz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::submit_events_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSetPropsDirect(
    JNIEnv* /*env*/,
    jobject /*thiz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::set_props_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSubmitEvents(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr,
    jbyteArray event_ring_bytes) {
  return nve::android_internal::submit_events_array(env, handle_ptr, event_ring_bytes);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSetPropsBinary(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr,
    jbyteArray props_bytes) {
  return nve::android_internal::set_props_array(env, handle_ptr, props_bytes);
}
