#pragma once

#include <nve/generated/nve_abi.generated.h>
#include "nve_rn_component.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <functional>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve {

inline constexpr std::array<uint8_t, 4> kNativePropsMagic = {'N', 'V', 'N', 'P'};
inline constexpr uint8_t kNativePropsVersion = 1u;
inline constexpr size_t kNativePropsHeaderSize = 7u;
inline constexpr size_t kMaxNativePropFields = 1024u;

inline constexpr std::array<uint8_t, 4> kNativeCmdMagic = {'N', 'V', 'N', 'C'};
inline constexpr uint8_t kNativeCmdVersion = 1u;
inline constexpr size_t kNativeCmdHeaderSize = 8u;
inline constexpr size_t kMaxNativeCmdArgs = 64u;

inline constexpr std::string_view kNativeViewKindPropKey = "nve.native_kind";
inline constexpr std::string_view kNativeViewKindPropKeyAlias = "nve.native.kind";
inline constexpr std::string_view kNativeViewPropsPropKey = "nve.native_props";
inline constexpr std::string_view kNativeViewPropsPropKeyAlias = "nve.native.props";

inline constexpr std::string_view kNativeKindBlurSurface = "nve.blur_surface";
inline constexpr std::string_view kNativeKindBlurAlias = "nve.blur";
inline constexpr std::string_view kNativeKindWebView = "nve.web_view";
inline constexpr std::string_view kNativeKindWebViewAlias = "nve.webview";
inline constexpr std::string_view kNativeKindVideoPlayer = "nve.video_player";
inline constexpr std::string_view kNativeKindVideoAlias = "nve.video";

enum class NativeViewCodecError : uint8_t {
  Ok = 0, Truncated, InvalidMagic, UnsupportedVersion, InvalidTag, InvalidBool, InvalidEnum,
  InvalidUtf8, MissingField, MissingArg, UnexpectedArgCount, TypeMismatch, TrailingBytes,
  UnknownCommandOpcode, TooManyFields,
};

enum class NativeValueTag : uint8_t {
  Null = 0, Bool = 1, U32 = 2, I64 = 3, U64 = 4, F32 = 5, F64 = 6, Str = 7, Bytes = 8,
};

namespace detail {

inline uint16_t read_u16_le(const uint8_t* p) noexcept {
  return static_cast<uint16_t>(p[0]) | static_cast<uint16_t>(static_cast<uint16_t>(p[1]) << 8u);
}

inline uint32_t read_u32_le(const uint8_t* p) noexcept {
  return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8u) |
         (static_cast<uint32_t>(p[2]) << 16u) | (static_cast<uint32_t>(p[3]) << 24u);
}

inline uint64_t read_u64_le(const uint8_t* p) noexcept {
  return static_cast<uint64_t>(read_u32_le(p)) | (static_cast<uint64_t>(read_u32_le(p + 4)) << 32u);
}

inline std::string_view trim_ws(std::string_view s) noexcept {
  auto is_ws = [](char c) { return c == ' ' || c == '\t' || c == '\r' || c == '\n'; };
  while (!s.empty() && is_ws(s.front())) s.remove_prefix(1u);
  while (!s.empty() && is_ws(s.back())) s.remove_suffix(1u);
  return s;
}

} // namespace detail

/// Zero-copy borrowed scalar/string/bytes value decoded from `NVNP` or `NVNC`.
struct NativeValueView {
  NativeValueTag tag{NativeValueTag::Null};
  union { bool bool_val; uint32_t u32_val; int64_t i64_val; uint64_t u64_val; float f32_val; double f64_val; } scalar{};
  const uint8_t* ptr{nullptr};
  size_t len{0u};

  bool is_null() const noexcept { return tag == NativeValueTag::Null; }
  std::optional<bool> as_bool() const noexcept { return tag == NativeValueTag::Bool ? std::optional(scalar.bool_val) : std::nullopt; }
  std::optional<uint32_t> as_u32() const noexcept { return tag == NativeValueTag::U32 ? std::optional(scalar.u32_val) : std::nullopt; }
  std::optional<int64_t> as_i64() const noexcept { return tag == NativeValueTag::I64 ? std::optional(scalar.i64_val) : std::nullopt; }
  std::optional<uint64_t> as_u64() const noexcept { return tag == NativeValueTag::U64 ? std::optional(scalar.u64_val) : std::nullopt; }
  std::optional<float> as_f32() const noexcept { return tag == NativeValueTag::F32 ? std::optional(scalar.f32_val) : std::nullopt; }
  std::optional<double> as_f64() const noexcept { return tag == NativeValueTag::F64 ? std::optional(scalar.f64_val) : std::nullopt; }
  std::optional<std::string_view> as_str() const noexcept {
    if (tag != NativeValueTag::Str) return std::nullopt;
    return std::string_view(reinterpret_cast<const char*>(ptr), len);
  }
  std::optional<std::span<const uint8_t>> as_bytes() const noexcept {
    if (tag != NativeValueTag::Bytes) return std::nullopt;
    return std::span<const uint8_t>(ptr, len);
  }
};

namespace detail {

inline NativeViewCodecError decode_value(std::span<const uint8_t> buf, size_t& pos, NativeValueView& out) noexcept {
  if (pos >= buf.size()) return NativeViewCodecError::Truncated;
  const uint8_t raw_tag = buf[pos++];
  out = NativeValueView{};
  switch (raw_tag) {
    case 0u: out.tag = NativeValueTag::Null; return NativeViewCodecError::Ok;
    case 1u: {
      if (buf.size() - pos < 1u) return NativeViewCodecError::Truncated;
      const uint8_t b = buf[pos++];
      if (b > 1u) return NativeViewCodecError::InvalidBool;
      out.tag = NativeValueTag::Bool; out.scalar.bool_val = (b == 1u);
      return NativeViewCodecError::Ok;
    }
    case 2u:
      if (buf.size() - pos < 4u) return NativeViewCodecError::Truncated;
      out.tag = NativeValueTag::U32; out.scalar.u32_val = read_u32_le(buf.data() + pos); pos += 4u;
      return NativeViewCodecError::Ok;
    case 3u: {
      if (buf.size() - pos < 8u) return NativeViewCodecError::Truncated;
      const uint64_t bits = read_u64_le(buf.data() + pos); pos += 8u;
      out.tag = NativeValueTag::I64; std::memcpy(&out.scalar.i64_val, &bits, sizeof(int64_t));
      return NativeViewCodecError::Ok;
    }
    case 4u:
      if (buf.size() - pos < 8u) return NativeViewCodecError::Truncated;
      out.tag = NativeValueTag::U64; out.scalar.u64_val = read_u64_le(buf.data() + pos); pos += 8u;
      return NativeViewCodecError::Ok;
    case 5u: {
      if (buf.size() - pos < 4u) return NativeViewCodecError::Truncated;
      const uint32_t bits = read_u32_le(buf.data() + pos); pos += 4u;
      out.tag = NativeValueTag::F32; std::memcpy(&out.scalar.f32_val, &bits, sizeof(float));
      return NativeViewCodecError::Ok;
    }
    case 6u: {
      if (buf.size() - pos < 8u) return NativeViewCodecError::Truncated;
      const uint64_t bits = read_u64_le(buf.data() + pos); pos += 8u;
      out.tag = NativeValueTag::F64; std::memcpy(&out.scalar.f64_val, &bits, sizeof(double));
      return NativeViewCodecError::Ok;
    }
    case 7u:
    case 8u: {
      if (buf.size() - pos < 4u) return NativeViewCodecError::Truncated;
      const size_t n = static_cast<size_t>(read_u32_le(buf.data() + pos)); pos += 4u;
      if (buf.size() - pos < n) return NativeViewCodecError::Truncated;
      if (raw_tag == 7u && !is_valid_utf8(buf.data() + pos, n)) return NativeViewCodecError::InvalidUtf8;
      out.tag = raw_tag == 7u ? NativeValueTag::Str : NativeValueTag::Bytes;
      out.ptr = buf.data() + pos; out.len = n; pos += n;
      return NativeViewCodecError::Ok;
    }
    default: return NativeViewCodecError::InvalidTag;
  }
}

inline NativeViewCodecError read_len_utf8(std::span<const uint8_t> raw, size_t& pos, std::string_view& out) noexcept {
  if (raw.size() - pos < 4u) return NativeViewCodecError::Truncated;
  const size_t n = static_cast<size_t>(read_u32_le(raw.data() + pos)); pos += 4u;
  if (raw.size() - pos < n) return NativeViewCodecError::Truncated;
  if (!is_valid_utf8(raw.data() + pos, n)) return NativeViewCodecError::InvalidUtf8;
  out = std::string_view(reinterpret_cast<const char*>(raw.data() + pos), n); pos += n;
  return NativeViewCodecError::Ok;
}

} // namespace detail

/// Zero-allocation validated decoder for binary `NVNP` native view prop buffers.
class NvnpDecoder {
 public:
  static NativeViewCodecError parse(std::span<const uint8_t> bytes, NvnpDecoder& out) noexcept {
    out = NvnpDecoder{};
    if (bytes.size() < kNativePropsHeaderSize) return NativeViewCodecError::Truncated;
    if (std::memcmp(bytes.data(), kNativePropsMagic.data(), 4u) != 0) return NativeViewCodecError::InvalidMagic;
    if (bytes[4] != kNativePropsVersion) return NativeViewCodecError::UnsupportedVersion;
    const uint16_t count = detail::read_u16_le(bytes.data() + 5u);
    if (count > kMaxNativePropFields) return NativeViewCodecError::TooManyFields;
    size_t pos = kNativePropsHeaderSize;
    if (static_cast<size_t>(count) > (bytes.size() - pos) / 3u) return NativeViewCodecError::Truncated;
    for (uint16_t i = 0u; i < count; ++i) {
      if (bytes.size() - pos < 2u) return NativeViewCodecError::Truncated;
      pos += 2u;
      NativeValueView val{};
      if (auto err = detail::decode_value(bytes, pos, val); err != NativeViewCodecError::Ok) return err;
    }
    if (pos != bytes.size()) return NativeViewCodecError::TrailingBytes;
    out.buf_ = bytes; out.field_count_ = count;
    return NativeViewCodecError::Ok;
  }

  size_t field_count() const noexcept { return field_count_; }
  bool empty() const noexcept { return field_count_ == 0u; }

  template <typename Fn>
  void for_each(Fn&& fn) const {
    size_t pos = kNativePropsHeaderSize;
    for (uint16_t i = 0u; i < field_count_; ++i) {
      const uint16_t id = detail::read_u16_le(buf_.data() + pos); pos += 2u;
      NativeValueView val{};
      (void)detail::decode_value(buf_, pos, val);
      fn(id, val);
    }
  }

  std::optional<NativeValueView> find(uint16_t field_id) const noexcept {
    size_t pos = kNativePropsHeaderSize;
    for (uint16_t i = 0u; i < field_count_; ++i) {
      const uint16_t id = detail::read_u16_le(buf_.data() + pos); pos += 2u;
      NativeValueView val{};
      (void)detail::decode_value(buf_, pos, val);
      if (id == field_id) return val;
    }
    return std::nullopt;
  }

  NativeViewCodecError require_bool(uint16_t id, bool& out) const noexcept {
    const auto v = find(id); if (!v) return NativeViewCodecError::MissingField;
    const auto b = v->as_bool(); if (!b) return NativeViewCodecError::TypeMismatch;
    out = *b; return NativeViewCodecError::Ok;
  }
  NativeViewCodecError require_u32(uint16_t id, uint32_t& out) const noexcept {
    const auto v = find(id); if (!v) return NativeViewCodecError::MissingField;
    const auto n = v->as_u32(); if (!n) return NativeViewCodecError::TypeMismatch;
    out = *n; return NativeViewCodecError::Ok;
  }
  NativeViewCodecError require_opt_u32(uint16_t id, std::optional<uint32_t>& out) const noexcept {
    const auto v = find(id); if (!v) return NativeViewCodecError::MissingField;
    if (v->is_null()) { out = std::nullopt; return NativeViewCodecError::Ok; }
    const auto n = v->as_u32(); if (!n) return NativeViewCodecError::TypeMismatch;
    out = *n; return NativeViewCodecError::Ok;
  }
  NativeViewCodecError require_f32(uint16_t id, float& out) const noexcept {
    const auto v = find(id); if (!v) return NativeViewCodecError::MissingField;
    const auto f = v->as_f32(); if (!f) return NativeViewCodecError::TypeMismatch;
    out = *f; return NativeViewCodecError::Ok;
  }
  NativeViewCodecError require_str(uint16_t id, std::string_view& out) const noexcept {
    const auto v = find(id); if (!v) return NativeViewCodecError::MissingField;
    const auto s = v->as_str(); if (!s) return NativeViewCodecError::TypeMismatch;
    out = *s; return NativeViewCodecError::Ok;
  }
  NativeViewCodecError require_bytes(uint16_t id, std::span<const uint8_t>& out) const noexcept {
    const auto v = find(id); if (!v) return NativeViewCodecError::MissingField;
    const auto b = v->as_bytes(); if (!b) return NativeViewCodecError::TypeMismatch;
    out = *b; return NativeViewCodecError::Ok;
  }

 private:
  std::span<const uint8_t> buf_{};
  uint16_t field_count_{0u};
};

/// Zero-allocation validated decoder for binary `NVNC` imperative command payloads.
class NvncDecoder {
 public:
  static NativeViewCodecError parse(std::span<const uint8_t> bytes, NvncDecoder& out) noexcept {
    out = NvncDecoder{};
    if (bytes.size() < kNativeCmdHeaderSize) return NativeViewCodecError::Truncated;
    if (std::memcmp(bytes.data(), kNativeCmdMagic.data(), 4u) != 0) return NativeViewCodecError::InvalidMagic;
    if (bytes[4] != kNativeCmdVersion) return NativeViewCodecError::UnsupportedVersion;
    const uint16_t op = detail::read_u16_le(bytes.data() + 5u);
    const uint8_t count = bytes[7];
    if (count > kMaxNativeCmdArgs) return NativeViewCodecError::TooManyFields;
    size_t pos = kNativeCmdHeaderSize;
    if (static_cast<size_t>(count) > bytes.size() - pos) return NativeViewCodecError::Truncated;
    for (uint8_t i = 0u; i < count; ++i) {
      NativeValueView val{};
      if (auto err = detail::decode_value(bytes, pos, val); err != NativeViewCodecError::Ok) return err;
    }
    if (pos != bytes.size()) return NativeViewCodecError::TrailingBytes;
    out.buf_ = bytes; out.opcode_ = op; out.arg_count_ = count;
    return NativeViewCodecError::Ok;
  }

  uint16_t opcode() const noexcept { return opcode_; }
  size_t arg_count() const noexcept { return arg_count_; }
  NativeViewCodecError expect_arg_count(size_t expected) const noexcept {
    return arg_count_ == expected ? NativeViewCodecError::Ok : NativeViewCodecError::UnexpectedArgCount;
  }

  std::optional<NativeValueView> arg(size_t idx) const noexcept {
    if (idx >= arg_count_) return std::nullopt;
    size_t pos = kNativeCmdHeaderSize;
    NativeValueView val{};
    for (size_t i = 0u; i <= idx; ++i) (void)detail::decode_value(buf_, pos, val);
    return val;
  }

  NativeViewCodecError require_str(size_t idx, std::string_view& out) const noexcept {
    const auto v = arg(idx); if (!v) return NativeViewCodecError::MissingArg;
    const auto s = v->as_str(); if (!s) return NativeViewCodecError::TypeMismatch;
    out = *s; return NativeViewCodecError::Ok;
  }
  NativeViewCodecError require_u64(size_t idx, uint64_t& out) const noexcept {
    const auto v = arg(idx); if (!v) return NativeViewCodecError::MissingArg;
    const auto n = v->as_u64(); if (!n) return NativeViewCodecError::TypeMismatch;
    out = *n; return NativeViewCodecError::Ok;
  }
  NativeViewCodecError require_f32(size_t idx, float& out) const noexcept {
    const auto v = arg(idx); if (!v) return NativeViewCodecError::MissingArg;
    const auto f = v->as_f32(); if (!f) return NativeViewCodecError::TypeMismatch;
    out = *f; return NativeViewCodecError::Ok;
  }

 private:
  std::span<const uint8_t> buf_{};
  uint16_t opcode_{0u};
  uint8_t arg_count_{0u};
};

// =============================================================================
// Typed Starter Spec Decoders (matching packages/core/interop/src/native_view/specs.rs)
// =============================================================================

enum class BlurMaterial : uint8_t { UltraThin = 0, Thin = 1, Regular = 2, Thick = 3, Chrome = 4, SystemMaterial = 5 };
struct BlurSurfaceProps { BlurMaterial material{BlurMaterial::Regular}; float intensity{1.0f}; std::optional<uint32_t> tint{}; };

inline NativeViewCodecError decode_blur_surface_props(const NvnpDecoder& dec, BlurSurfaceProps& out) noexcept {
  uint32_t mat = 0u;
  if (auto e = dec.require_u32(1u, mat); e != NativeViewCodecError::Ok) return e;
  if (mat > static_cast<uint32_t>(BlurMaterial::SystemMaterial)) return NativeViewCodecError::InvalidEnum;
  out.material = static_cast<BlurMaterial>(mat);
  if (auto e = dec.require_f32(2u, out.intensity); e != NativeViewCodecError::Ok) return e;
  return dec.require_opt_u32(3u, out.tint);
}

struct WebViewProps { std::string_view url; bool allows_back_forward{true}; };
enum class WebViewCommandKind : uint16_t { Navigate = 1, Reload = 2, GoBack = 3, GoForward = 4, EvaluateScript = 5 };
struct WebViewCommand { WebViewCommandKind kind{WebViewCommandKind::Reload}; std::string_view text; };

inline NativeViewCodecError decode_web_view_props(const NvnpDecoder& dec, WebViewProps& out) noexcept {
  if (auto e = dec.require_str(1u, out.url); e != NativeViewCodecError::Ok) return e;
  return dec.require_bool(2u, out.allows_back_forward);
}

inline NativeViewCodecError decode_web_view_command(const NvncDecoder& dec, WebViewCommand& out) noexcept {
  out = WebViewCommand{};
  switch (dec.opcode()) {
    case 1u:
      if (auto e = dec.expect_arg_count(1u); e != NativeViewCodecError::Ok) return e;
      out.kind = WebViewCommandKind::Navigate; return dec.require_str(0u, out.text);
    case 2u: out.kind = WebViewCommandKind::Reload; return dec.expect_arg_count(0u);
    case 3u: out.kind = WebViewCommandKind::GoBack; return dec.expect_arg_count(0u);
    case 4u: out.kind = WebViewCommandKind::GoForward; return dec.expect_arg_count(0u);
    case 5u:
      if (auto e = dec.expect_arg_count(1u); e != NativeViewCodecError::Ok) return e;
      out.kind = WebViewCommandKind::EvaluateScript; return dec.require_str(0u, out.text);
    default: return NativeViewCodecError::UnknownCommandOpcode;
  }
}

struct VideoPlayerProps { std::string_view source_uri; bool autoplay{false}; bool muted{false}; bool looping{false}; bool controls{true}; };
enum class VideoPlayerCommandKind : uint16_t { Play = 1, Pause = 2, SeekMs = 3, SetVolume = 4 };
struct VideoPlayerCommand { VideoPlayerCommandKind kind{VideoPlayerCommandKind::Play}; uint64_t seek_ms{0u}; float volume{1.0f}; };

inline NativeViewCodecError decode_video_player_props(const NvnpDecoder& dec, VideoPlayerProps& out) noexcept {
  if (auto e = dec.require_str(1u, out.source_uri); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_bool(2u, out.autoplay); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_bool(3u, out.muted); e != NativeViewCodecError::Ok) return e;
  if (auto e = dec.require_bool(4u, out.looping); e != NativeViewCodecError::Ok) return e;
  return dec.require_bool(5u, out.controls);
}

inline NativeViewCodecError decode_video_player_command(const NvncDecoder& dec, VideoPlayerCommand& out) noexcept {
  out = VideoPlayerCommand{};
  switch (dec.opcode()) {
    case 1u: out.kind = VideoPlayerCommandKind::Play; return dec.expect_arg_count(0u);
    case 2u: out.kind = VideoPlayerCommandKind::Pause; return dec.expect_arg_count(0u);
    case 3u:
      if (auto e = dec.expect_arg_count(1u); e != NativeViewCodecError::Ok) return e;
      out.kind = VideoPlayerCommandKind::SeekMs; return dec.require_u64(0u, out.seek_ms);
    case 4u:
      if (auto e = dec.expect_arg_count(1u); e != NativeViewCodecError::Ok) return e;
      out.kind = VideoPlayerCommandKind::SetVolume; return dec.require_f32(0u, out.volume);
    default: return NativeViewCodecError::UnknownCommandOpcode;
  }
}

template <typename AttrFn>
inline NativeViewCodecError decode_web_element_props(const NvnpDecoder& dec, std::string_view& out_tag, AttrFn&& on_attr) {
  if (auto e = dec.require_str(1u, out_tag); e != NativeViewCodecError::Ok) return e;
  uint32_t count = 0u;
  if (auto e = dec.require_u32(2u, count); e != NativeViewCodecError::Ok) return e;
  std::span<const uint8_t> raw{};
  if (auto e = dec.require_bytes(3u, raw); e != NativeViewCodecError::Ok) return e;
  if (static_cast<size_t>(count) > raw.size() / 8u) return NativeViewCodecError::Truncated;
  size_t pos = 0u;
  for (uint32_t i = 0u; i < count; ++i) {
    std::string_view key, val;
    if (auto e = detail::read_len_utf8(raw, pos, key); e != NativeViewCodecError::Ok) return e;
    if (auto e = detail::read_len_utf8(raw, pos, val); e != NativeViewCodecError::Ok) return e;
    on_attr(key, val);
  }
  return pos == raw.size() ? NativeViewCodecError::Ok : NativeViewCodecError::TrailingBytes;
}

// =============================================================================
// Host-Side NativeViewRegistry (Exact Kinds + "prefix.*" Wildcards)
// =============================================================================

struct NativeViewAdapter {
  std::function<void*(uint32_t node_id, std::string_view kind, const NvnpDecoder* initial_props)> create;
  std::function<void(void* instance, uint32_t node_id, const NvnpDecoder& props)> update_props;
  std::function<bool(void* instance, uint32_t node_id, std::string_view command_name, const NvncDecoder& cmd)> dispatch_command;
  std::function<void(void* instance, uint32_t node_id)> unmount;
};

class NativeViewRegistry {
 public:
  NativeViewRegistry() = default;
  ~NativeViewRegistry() { unmount_all(); }
  NativeViewRegistry(const NativeViewRegistry&) = delete;
  NativeViewRegistry& operator=(const NativeViewRegistry&) = delete;

  void register_adapter(std::string_view pattern, NativeViewAdapter adapter) {
    const std::string_view trimmed = detail::trim_ws(pattern);
    if (trimmed.empty()) return;
    if (trimmed.back() == '*') {
      const std::string prefix(trimmed.substr(0u, trimmed.size() - 1u));
      for (auto& e : wildcards_) {
        if (e.first == prefix) { e.second = std::move(adapter); return; }
      }
      wildcards_.emplace_back(prefix, std::move(adapter));
      std::sort(wildcards_.begin(), wildcards_.end(), [](const auto& a, const auto& b) {
        return a.first.size() != b.first.size() ? a.first.size() > b.first.size() : a.first < b.first;
      });
    } else {
      exact_[std::string(trimmed)] = std::move(adapter);
    }
  }

  bool unregister_adapter(std::string_view pattern) {
    const std::string_view trimmed = detail::trim_ws(pattern);
    if (trimmed.empty()) return false;
    if (trimmed.back() == '*') {
      const std::string_view prefix = trimmed.substr(0u, trimmed.size() - 1u);
      const size_t before = wildcards_.size();
      wildcards_.erase(std::remove_if(wildcards_.begin(), wildcards_.end(),
                                      [&](const auto& e) { return e.first == prefix; }), wildcards_.end());
      return wildcards_.size() != before;
    }
    return exact_.erase(std::string(trimmed)) > 0u;
  }

  const NativeViewAdapter* find_adapter(std::string_view kind) const noexcept {
    const std::string_view trimmed = detail::trim_ws(kind);
    if (trimmed.empty()) return nullptr;
    if (auto it = exact_.find(std::string(trimmed)); it != exact_.end()) return &it->second;
    for (const auto& [prefix, adapter] : wildcards_) {
      if (prefix.empty() || (trimmed.size() > prefix.size() && trimmed.substr(0u, prefix.size()) == prefix)) {
        return &adapter;
      }
    }
    return nullptr;
  }

  bool supports(std::string_view kind) const noexcept { return find_adapter(kind) != nullptr; }

  void* mount_or_update(uint32_t node_id, std::string_view kind, std::span<const uint8_t> nvnp_bytes,
                        NativeViewCodecError* out_err = nullptr) {
    const NativeViewAdapter* adapter = find_adapter(kind);
    if (adapter == nullptr) { if (out_err) *out_err = NativeViewCodecError::Ok; return nullptr; }
    NvnpDecoder dec{};
    const bool has_props = !nvnp_bytes.empty();
    if (has_props) {
      const auto err = NvnpDecoder::parse(nvnp_bytes, dec);
      if (out_err) *out_err = err;
      if (err != NativeViewCodecError::Ok) return nullptr;
    } else if (out_err) { *out_err = NativeViewCodecError::Ok; }

    auto it = mounted_.find(node_id);
    if (it != mounted_.end() && it->second.kind != kind) { unmount(node_id); it = mounted_.end(); }
    if (it == mounted_.end()) {
      void* inst = adapter->create ? adapter->create(node_id, kind, has_props ? &dec : nullptr) : nullptr;
      if (inst == nullptr) return nullptr;
      it = mounted_.emplace(node_id, MountedEntry{std::string(kind), inst, adapter}).first;
    }
    if (has_props && adapter->update_props) adapter->update_props(it->second.instance, node_id, dec);
    return it->second.instance;
  }

  bool dispatch_command(uint32_t node_id, std::string_view command_name, std::span<const uint8_t> nvnc_bytes,
                        NativeViewCodecError* out_err = nullptr) {
    auto it = mounted_.find(node_id);
    if (it == mounted_.end() || !it->second.adapter || !it->second.adapter->dispatch_command) {
      if (out_err) *out_err = NativeViewCodecError::Ok;
      return false;
    }
    NvncDecoder dec{};
    const auto err = NvncDecoder::parse(nvnc_bytes, dec);
    if (out_err) *out_err = err;
    if (err != NativeViewCodecError::Ok) return false;
    return it->second.adapter->dispatch_command(it->second.instance, node_id, command_name, dec);
  }

  bool unmount(uint32_t node_id) {
    auto it = mounted_.find(node_id);
    if (it == mounted_.end()) return false;
    if (it->second.adapter && it->second.adapter->unmount) it->second.adapter->unmount(it->second.instance, node_id);
    mounted_.erase(it);
    return true;
  }

  void unmount_all() {
    for (auto& [node_id, entry] : mounted_) {
      if (entry.adapter && entry.adapter->unmount) entry.adapter->unmount(entry.instance, node_id);
    }
    mounted_.clear();
  }

  void* instance_for(uint32_t node_id) const noexcept {
    auto it = mounted_.find(node_id); return it != mounted_.end() ? it->second.instance : nullptr;
  }
  std::string_view kind_for(uint32_t node_id) const noexcept {
    auto it = mounted_.find(node_id); return it != mounted_.end() ? std::string_view(it->second.kind) : std::string_view{};
  }
  size_t mounted_count() const noexcept { return mounted_.size(); }

  /// Registers a `CustomComponentDescriptor` on `host` forwarding `NVNP` raw props and `NVNC` commands.
  void bind_to_fabric_host(NveFabricHostHandle host, std::string_view kind) {
    if (host == nullptr || kind.empty()) return;
    const std::string kind_owned(kind);
    CustomComponentDescriptor desc{};
    desc.component_name = kind_owned;
    desc.on_update_props = [this, kind_owned](uint32_t node_id, const RawPropMap& props) {
      std::string_view resolved_kind = kind_owned;
      for (std::string_view k : {kNativeViewKindPropKey, kNativeViewKindPropKeyAlias}) {
        if (auto it = props.find(std::string(k)); it != props.end() && std::holds_alternative<std::string>(it->second)) {
          resolved_kind = std::get<std::string>(it->second); break;
        }
      }
      std::span<const uint8_t> prop_bytes{};
      for (std::string_view k : {kNativeViewPropsPropKey, kNativeViewPropsPropKeyAlias}) {
        if (auto it = props.find(std::string(k)); it != props.end() && std::holds_alternative<std::vector<uint8_t>>(it->second)) {
          const auto& v = std::get<std::vector<uint8_t>>(it->second);
          prop_bytes = std::span<const uint8_t>(v.data(), v.size()); break;
        }
      }
      (void)this->mount_or_update(node_id, resolved_kind, prop_bytes);
    };
    desc.on_dispatch_command = [this](uint32_t node_id, std::string_view cmd_name, NveBufferSlice payload) {
      (void)this->dispatch_command(node_id, cmd_name, std::span<const uint8_t>(payload.ptr, payload.len));
    };
    fabric_host_register_custom_component(host, std::move(desc));
  }

 private:
  struct MountedEntry { std::string kind; void* instance{nullptr}; const NativeViewAdapter* adapter{nullptr}; };
  std::unordered_map<std::string, NativeViewAdapter> exact_;
  std::vector<std::pair<std::string, NativeViewAdapter>> wildcards_;
  std::unordered_map<uint32_t, MountedEntry> mounted_;
};

} // namespace nve
