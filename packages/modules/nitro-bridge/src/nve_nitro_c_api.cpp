#include "nve_nitro_core.hpp"

#include <atomic>
#include <cstring>
#include <memory>
#include <string>

struct NveNitroHandleOpaque {
  std::shared_ptr<nve::nitro::HybridObject> object;
  std::string cached_name;
  std::atomic<uint32_t> ref_count{1u};
};

namespace {

int32_t write_output_bytes(
    const uint8_t* src,
    size_t src_len,
    uint8_t* out_buf,
    size_t out_cap,
    size_t* out_len) {
  if (out_len) *out_len = src_len;
  if (src_len == 0u) return NVE_NITRO_OK;
  if (!out_buf || out_cap < src_len) return NVE_NITRO_ERR_BUFFER_TOO_SMALL;
  std::memcpy(out_buf, src, src_len);
  return NVE_NITRO_OK;
}

} // namespace

extern "C" {

NveNitroHandle nve_nitro_create(const char* name) {
  if (!name || name[0] == '\0') return nullptr;
  auto obj = nve::nitro::HybridObjectRegistry::global().create_raw(name);
  if (!obj) return nullptr;
  auto* h = new NveNitroHandleOpaque();
  h->cached_name = std::string(obj->name());
  h->object = std::move(obj);
  return h;
}

void nve_nitro_retain(NveNitroHandle handle) {
  if (!handle) return;
  handle->ref_count.fetch_add(1u, std::memory_order_relaxed);
}

void nve_nitro_release(NveNitroHandle handle) {
  if (!handle) return;
  if (handle->ref_count.fetch_sub(1u, std::memory_order_acq_rel) == 1u) {
    delete handle;
  }
}

void nve_nitro_dispose(NveNitroHandle handle) {
  if (!handle || !handle->object) return;
  handle->object->dispose();
}

int32_t nve_nitro_is_disposed(NveNitroHandle handle) {
  if (!handle || !handle->object) return 1;
  return handle->object->is_disposed() ? 1 : 0;
}

const char* nve_nitro_get_name(NveNitroHandle handle) {
  if (!handle || !handle->object) return nullptr;
  return handle->cached_name.c_str();
}

size_t nve_nitro_memory_size(NveNitroHandle handle) {
  if (!handle || !handle->object) return 0u;
  return handle->object->memory_size();
}

int32_t nve_nitro_has_registered(const char* name) {
  if (!name || name[0] == '\0') return 0;
  return nve::nitro::HybridObjectRegistry::global().has_hybrid_object(name) ? 1 : 0;
}

int32_t nve_nitro_invoke_bytes(
    NveNitroHandle handle,
    const char* method,
    const uint8_t* arg_bytes,
    size_t arg_len,
    uint8_t* out_buf,
    size_t out_cap,
    size_t* out_len) {
  if (out_len) *out_len = 0u;
  if (!handle || !handle->object || !method) return NVE_NITRO_ERR_INVALID_HANDLE;
  if (handle->object->is_disposed()) return NVE_NITRO_ERR_DISPOSED;

  std::vector<nve::nitro::NitroVariant> args;
  if (arg_bytes && arg_len > 0u) {
    args.push_back(nve::nitro::NitroVariant::from_bytes(
        nve::nitro::ArrayBuffer::wrap(const_cast<uint8_t*>(arg_bytes), arg_len)));
  }

  const auto result = handle->object->invoke_method(method, args);
  if (result.is_error()) {
    const auto* err = result.as_error();
    if (err && err->starts_with("METHOD_NOT_FOUND:")) {
      return NVE_NITRO_ERR_NOT_FOUND;
    }
    return NVE_NITRO_ERR_INVOCATION_FAILED;
  }

  switch (result.kind()) {
    case nve::nitro::NitroValueKind::Null:
      return NVE_NITRO_OK;
    case nve::nitro::NitroValueKind::Bool: {
      const uint8_t b = *result.as_bool() ? 1u : 0u;
      return write_output_bytes(&b, 1u, out_buf, out_cap, out_len);
    }
    case nve::nitro::NitroValueKind::Int64: {
      const int64_t v = *result.as_i64();
      uint8_t tmp[8];
      for (size_t i = 0u; i < 8u; ++i) {
        tmp[i] = static_cast<uint8_t>((static_cast<uint64_t>(v) >> (i * 8u)) & 0xFFu);
      }
      return write_output_bytes(tmp, sizeof(tmp), out_buf, out_cap, out_len);
    }
    case nve::nitro::NitroValueKind::Double: {
      const double v = *result.as_f64();
      uint64_t bits = 0u;
      std::memcpy(&bits, &v, sizeof(bits));
      uint8_t tmp[8];
      for (size_t i = 0u; i < 8u; ++i) {
        tmp[i] = static_cast<uint8_t>((bits >> (i * 8u)) & 0xFFu);
      }
      return write_output_bytes(tmp, sizeof(tmp), out_buf, out_cap, out_len);
    }
    case nve::nitro::NitroValueKind::String: {
      const auto* s = result.as_string();
      return write_output_bytes(
          reinterpret_cast<const uint8_t*>(s->data()), s->size(), out_buf, out_cap, out_len);
    }
    case nve::nitro::NitroValueKind::ArrayBuffer: {
      const auto ab = result.as_array_buffer();
      if (!ab) return NVE_NITRO_OK;
      return write_output_bytes(ab->data(), ab->size(), out_buf, out_cap, out_len);
    }
    case nve::nitro::NitroValueKind::HybridObject:
    case nve::nitro::NitroValueKind::Error:
      return NVE_NITRO_ERR_INVOCATION_FAILED;
  }
  return NVE_NITRO_ERR_INVOCATION_FAILED;
}

} // extern "C"
