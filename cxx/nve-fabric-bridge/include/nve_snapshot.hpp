#pragma once

#include "nve_abi.generated.h"

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <cerrno>
#include <cstdio>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#endif

namespace nve {

/// 8-byte file magic identifying an `NVERELOC` zero-copy relocatable container (`*b"NVERELOC"`).
inline constexpr uint8_t kRelocMagic[8] = {'N', 'V', 'E', 'R', 'E', 'L', 'O', 'C'};
inline constexpr uint8_t kSnapshotMagic[8] = {'N', 'V', 'E', 'R', 'E', 'L', 'O', 'C'};

/// Current `NVERELOC` container format version (matches `packages/core/engine/src/reloc/format.rs`).
inline constexpr uint32_t kRelocFormatVersion = 1u;
inline constexpr uint32_t kSnapshotFormatVersion = kRelocFormatVersion;

/// Fixed byte length of the 64-byte cache-line `RelocHeader`.
inline constexpr size_t kRelocHeaderSize = 64u;
inline constexpr size_t kSnapshotHeaderSize = kRelocHeaderSize;

/// Fixed byte length of `RelocBundleRoot` at offset 64 (`frame0` + `state` + `meta` slices).
inline constexpr size_t kRelocBundleRootSize = 32u;
inline constexpr size_t kRelocBundleMinSize = kRelocHeaderSize + kRelocBundleRootSize;

/// Byte alignment enforced for payload sections inside a bundle.
inline constexpr size_t kSnapshotSectionAlign = 8u;

/// `RelocHeader::content_kind` discriminants matching Rust `nve_engine::reloc`.
inline constexpr uint32_t kRelocKindGeneric = 0u;
inline constexpr uint32_t kRelocKindScene = 1u;
inline constexpr uint32_t kRelocKindBundle = 2u;
inline constexpr uint32_t kRelocKindWorkspace = 3u;

/// Header offset of each fixed field in the 64-byte `NVERELOC` wire header.
inline constexpr size_t kSnapshotMagicOffset = 0u;
inline constexpr size_t kSnapshotFormatVersionOffset = 8u;
inline constexpr size_t kSnapshotFlagsOffset = 12u;
inline constexpr size_t kRelocPayloadLenOffset = 16u;
inline constexpr size_t kRelocRootOffsetField = 20u;
inline constexpr size_t kSnapshotChecksumOffset = 24u;
inline constexpr size_t kSnapshotChecksumEnd = 32u;
inline constexpr size_t kSnapshotSchemaVersionOffset = 32u;
inline constexpr size_t kRelocContentKindOffset = 36u;
inline constexpr size_t kSnapshotReservedOffset = 40u;

/// Canonical XXH64 constants matching `packages/core/engine/src/hash.rs`.
inline constexpr uint64_t kXxh64Prime1 = 0x9E3779B185EBCA87ULL;
inline constexpr uint64_t kXxh64Prime2 = 0xC2B2AE3D27D4EB4FULL;
inline constexpr uint64_t kXxh64Prime3 = 0x165667B19E3779F9ULL;
inline constexpr uint64_t kXxh64Prime4 = 0x85EBCA77C2B2AE63ULL;
inline constexpr uint64_t kXxh64Prime5 = 0x27D4EB2F165667C5ULL;

/// Validation mode for opening an `NVERELOC` container.
enum class VerifyMode : uint8_t {
  Full = 0,
  HeaderOnly = 1,
};

/// Validation & I/O status codes for `MmapSnapshot` (`MmapReloc`) and `write_snapshot_atomic`.
enum class SnapshotStatus : uint8_t {
  Ok = 0,
  IoError = 1,
  Truncated = 2,
  InvalidMagic = 3,
  UnsupportedFormatVersion = 4,
  InvalidReservedHeader = 5,
  SchemaVersionMismatch = 6,
  ChecksumMismatch = 7,
  UnalignedSection = 8,
  SectionOutOfBounds = 9,
  OverlappingSections = 10,
  ContentKindMismatch = 11,
};

/// 8-byte buffer-base-relative `(offset, len)` slice descriptor inside `NVERELOC`.
struct RelocSlice {
  uint32_t offset{0};
  uint32_t len{0};
};

/// 64-byte `NVERELOC` cache-line header matching Rust `nve_engine::reloc::RelocHeader`.
struct RelocHeader {
  uint8_t magic[8]{0, 0, 0, 0, 0, 0, 0, 0};
  uint32_t format_version{0};
  uint32_t flags{0};
  uint32_t payload_len{0};
  uint32_t root_offset{0};
  uint64_t checksum_xxh64{0};
  uint32_t app_schema_version{0};
  uint32_t content_kind{0};
  uint64_t reserved[3]{0, 0, 0};
};

/// 32-byte bundle root at offset 64 matching Rust `nve_engine::reloc::RelocBundleRoot`.
struct RelocBundleRoot {
  RelocSlice frame0{};
  RelocSlice state{};
  RelocSlice meta{};
  uint64_t reserved{0};
};

/// 64-byte pre-rendered scene root matching Rust `nve_engine::reloc::RelocSceneRoot`.
struct RelocSceneRoot {
  float viewport_width{0.0f};
  float viewport_height{0.0f};
  float scale_factor{1.0f};
  uint32_t root_node_index{0};
  uint32_t surface_id{0};
  uint32_t clear_rgba{0};
  uint64_t frame_id{0};
  RelocSlice nodes{};
  RelocSlice layouts{};
  RelocSlice app_tag{};
  uint64_t reserved{0};
};

/// 128-byte pre-rendered scene node matching Rust `nve_engine::reloc::RelocSceneNode`.
struct RelocSceneNode {
  uint32_t node_id{0};
  int32_t parent_index{-1};
  uint64_t event_mask{0};
  float x{0.0f};
  float y{0.0f};
  float width{0.0f};
  float height{0.0f};
  uint32_t bg_rgba{0};
  uint32_t text_rgba{0};
  uint32_t border_rgba{0};
  int32_t z_index{0};
  float border_width{0.0f};
  float border_radius{0.0f};
  float opacity{1.0f};
  float font_size{16.0f};
  uint8_t kind{0};
  uint8_t flags{0};
  uint8_t text_align{0};
  uint8_t image_resize_mode{0};
  uint16_t font_weight{400};
  uint16_t pad0{0};
  RelocSlice text{};
  RelocSlice tag{};
  RelocSlice image_src{};
  RelocSlice box_shadow{};
  RelocSlice draw_list{};
  RelocSlice children{};
  uint64_t reserved{0};
};

using SnapshotHeader = RelocHeader;

// Compile-time layout & wire-offset verification matching Rust `packages/core/engine/src/reloc/`.
static_assert(sizeof(kRelocMagic) == 8u, "NVERELOC magic must be 8 bytes");
static_assert(sizeof(RelocSlice) == 8u, "RelocSlice must be 8 bytes");
static_assert(sizeof(RelocHeader) == kRelocHeaderSize, "RelocHeader must be 64 bytes");
static_assert(offsetof(RelocHeader, magic) == kSnapshotMagicOffset);
static_assert(offsetof(RelocHeader, format_version) == kSnapshotFormatVersionOffset);
static_assert(offsetof(RelocHeader, flags) == kSnapshotFlagsOffset);
static_assert(offsetof(RelocHeader, payload_len) == kRelocPayloadLenOffset);
static_assert(offsetof(RelocHeader, root_offset) == kRelocRootOffsetField);
static_assert(offsetof(RelocHeader, checksum_xxh64) == kSnapshotChecksumOffset);
static_assert(offsetof(RelocHeader, app_schema_version) == kSnapshotSchemaVersionOffset);
static_assert(offsetof(RelocHeader, content_kind) == kRelocContentKindOffset);
static_assert(offsetof(RelocHeader, reserved) == kSnapshotReservedOffset);
static_assert(sizeof(RelocBundleRoot) == kRelocBundleRootSize, "RelocBundleRoot must be 32 bytes");
static_assert(offsetof(RelocBundleRoot, frame0) == 0u);
static_assert(offsetof(RelocBundleRoot, state) == 8u);
static_assert(offsetof(RelocBundleRoot, meta) == 16u);
static_assert(offsetof(RelocBundleRoot, reserved) == 24u);
static_assert(sizeof(RelocSceneRoot) == 64u, "RelocSceneRoot must be 64 bytes");
static_assert(sizeof(RelocSceneNode) == 128u, "RelocSceneNode must be 128 bytes");

namespace detail {

inline constexpr uint64_t xxh64_rotl(uint64_t x, unsigned r) noexcept {
  return (x << r) | (x >> (64u - r));
}

inline constexpr uint64_t xxh64_round(uint64_t acc, uint64_t lane) noexcept {
  acc += lane * kXxh64Prime2;
  acc = xxh64_rotl(acc, 31u);
  acc *= kXxh64Prime1;
  return acc;
}

inline constexpr uint64_t xxh64_merge_round(uint64_t acc, uint64_t val) noexcept {
  val = xxh64_round(0u, val);
  acc ^= val;
  acc = acc * kXxh64Prime1 + kXxh64Prime4;
  return acc;
}

inline constexpr uint64_t xxh64_avalanche(uint64_t h) noexcept {
  h ^= h >> 33u;
  h *= kXxh64Prime2;
  h ^= h >> 29u;
  h *= kXxh64Prime3;
  h ^= h >> 32u;
  return h;
}

inline constexpr size_t align_up_8(size_t n) noexcept {
  return (n + (kSnapshotSectionAlign - 1u)) & ~(kSnapshotSectionAlign - 1u);
}

inline void pad_to_8(std::vector<uint8_t>& buf) {
  const size_t aligned = align_up_8(buf.size());
  if (aligned > buf.size()) {
    buf.resize(aligned, 0u);
  }
}

inline uint32_t read_le32(const uint8_t* p) noexcept {
  return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8u) |
         (static_cast<uint32_t>(p[2]) << 16u) | (static_cast<uint32_t>(p[3]) << 24u);
}

inline uint64_t read_le64(const uint8_t* p) noexcept {
  return static_cast<uint64_t>(p[0]) | (static_cast<uint64_t>(p[1]) << 8u) |
         (static_cast<uint64_t>(p[2]) << 16u) | (static_cast<uint64_t>(p[3]) << 24u) |
         (static_cast<uint64_t>(p[4]) << 32u) | (static_cast<uint64_t>(p[5]) << 40u) |
         (static_cast<uint64_t>(p[6]) << 48u) | (static_cast<uint64_t>(p[7]) << 56u);
}

inline void write_le32(uint8_t* p, uint32_t v) noexcept {
  p[0] = static_cast<uint8_t>(v & 0xFFu);
  p[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
  p[2] = static_cast<uint8_t>((v >> 16u) & 0xFFu);
  p[3] = static_cast<uint8_t>((v >> 24u) & 0xFFu);
}

inline void write_le64(uint8_t* p, uint64_t v) noexcept {
  p[0] = static_cast<uint8_t>(v & 0xFFu);
  p[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
  p[2] = static_cast<uint8_t>((v >> 16u) & 0xFFu);
  p[3] = static_cast<uint8_t>((v >> 24u) & 0xFFu);
  p[4] = static_cast<uint8_t>((v >> 32u) & 0xFFu);
  p[5] = static_cast<uint8_t>((v >> 40u) & 0xFFu);
  p[6] = static_cast<uint8_t>((v >> 48u) & 0xFFu);
  p[7] = static_cast<uint8_t>((v >> 56u) & 0xFFu);
}

inline uint64_t xxh64_finalize(uint64_t h, const uint8_t* rem, size_t len) noexcept {
  size_t i = 0u;
  while (i + 8u <= len) {
    const uint64_t k1 = xxh64_round(0u, read_le64(rem + i));
    h ^= k1;
    h = xxh64_rotl(h, 27u) * kXxh64Prime1 + kXxh64Prime4;
    i += 8u;
  }
  if (i + 4u <= len) {
    h ^= static_cast<uint64_t>(read_le32(rem + i)) * kXxh64Prime1;
    h = xxh64_rotl(h, 23u) * kXxh64Prime2 + kXxh64Prime3;
    i += 4u;
  }
  while (i < len) {
    h ^= static_cast<uint64_t>(rem[i]) * kXxh64Prime5;
    h = xxh64_rotl(h, 11u) * kXxh64Prime1;
    ++i;
  }
  return xxh64_avalanche(h);
}

class Xxh64Hasher {
 public:
  explicit constexpr Xxh64Hasher(uint64_t seed = 0u) noexcept
      : total_len_(0u),
        large_len_(false),
        seed_(seed),
        v1_(seed + kXxh64Prime1 + kXxh64Prime2),
        v2_(seed + kXxh64Prime2),
        v3_(seed),
        v4_(seed - kXxh64Prime1),
        buf_{},
        buf_len_(0u) {}

  void update(const uint8_t* data, size_t len) noexcept {
    if (data == nullptr || len == 0u) {
      return;
    }
    total_len_ += static_cast<uint64_t>(len);
    if (total_len_ >= 32u) {
      large_len_ = true;
    }
    if (buf_len_ + len < 32u) {
      std::memcpy(buf_ + buf_len_, data, len);
      buf_len_ += len;
      return;
    }
    size_t pos = 0u;
    if (buf_len_ > 0u) {
      const size_t take = 32u - buf_len_;
      std::memcpy(buf_ + buf_len_, data, take);
      v1_ = xxh64_round(v1_, read_le64(buf_ + 0u));
      v2_ = xxh64_round(v2_, read_le64(buf_ + 8u));
      v3_ = xxh64_round(v3_, read_le64(buf_ + 16u));
      v4_ = xxh64_round(v4_, read_le64(buf_ + 24u));
      pos += take;
      buf_len_ = 0u;
    }
    while (pos + 32u <= len) {
      v1_ = xxh64_round(v1_, read_le64(data + pos + 0u));
      v2_ = xxh64_round(v2_, read_le64(data + pos + 8u));
      v3_ = xxh64_round(v3_, read_le64(data + pos + 16u));
      v4_ = xxh64_round(v4_, read_le64(data + pos + 24u));
      pos += 32u;
    }
    if (pos < len) {
      buf_len_ = len - pos;
      std::memcpy(buf_, data + pos, buf_len_);
    }
  }

  uint64_t finish() const noexcept {
    uint64_t h64 = 0u;
    if (large_len_) {
      h64 = xxh64_rotl(v1_, 1u) + xxh64_rotl(v2_, 7u) + xxh64_rotl(v3_, 12u) +
            xxh64_rotl(v4_, 18u);
      h64 = xxh64_merge_round(h64, v1_);
      h64 = xxh64_merge_round(h64, v2_);
      h64 = xxh64_merge_round(h64, v3_);
      h64 = xxh64_merge_round(h64, v4_);
    } else {
      h64 = seed_ + kXxh64Prime5;
    }
    h64 += total_len_;
    return xxh64_finalize(h64, buf_, buf_len_);
  }

 private:
  uint64_t total_len_;
  bool large_len_;
  uint64_t seed_;
  uint64_t v1_;
  uint64_t v2_;
  uint64_t v3_;
  uint64_t v4_;
  uint8_t buf_[32];
  size_t buf_len_;
};

inline uint64_t compute_reloc_checksum(const uint8_t* data, size_t len) noexcept {
  Xxh64Hasher h(0u);
  h.update(data, kSnapshotChecksumOffset);
  h.update(data + kSnapshotChecksumEnd, len - kSnapshotChecksumEnd);
  return h.finish();
}

inline SnapshotStatus decode_and_validate_reloc_bundle(
    const uint8_t* data,
    size_t len,
    bool verify_checksum,
    RelocHeader& out_header,
    RelocBundleRoot& out_bundle) noexcept {
  if (data == nullptr || len < kRelocHeaderSize) {
    return SnapshotStatus::Truncated;
  }
  if (std::memcmp(data, kRelocMagic, sizeof(kRelocMagic)) != 0) {
    return SnapshotStatus::InvalidMagic;
  }

  const uint32_t format_version = read_le32(data + kSnapshotFormatVersionOffset);
  if (format_version != kRelocFormatVersion) {
    return SnapshotStatus::UnsupportedFormatVersion;
  }

  RelocHeader h{};
  std::memcpy(h.magic, data, sizeof(h.magic));
  h.format_version = format_version;
  h.flags = read_le32(data + kSnapshotFlagsOffset);
  h.payload_len = read_le32(data + kRelocPayloadLenOffset);
  h.root_offset = read_le32(data + kRelocRootOffsetField);
  h.checksum_xxh64 = read_le64(data + kSnapshotChecksumOffset);
  h.app_schema_version = read_le32(data + kSnapshotSchemaVersionOffset);
  h.content_kind = read_le32(data + kRelocContentKindOffset);
  h.reserved[0] = read_le64(data + 40u);
  h.reserved[1] = read_le64(data + 48u);
  h.reserved[2] = read_le64(data + 56u);

  if (h.reserved[0] != 0u || h.reserved[1] != 0u || h.reserved[2] != 0u) {
    return SnapshotStatus::InvalidReservedHeader;
  }
  if (static_cast<size_t>(h.payload_len) != (len - kRelocHeaderSize)) {
    return SnapshotStatus::Truncated;
  }
  if (h.content_kind != kRelocKindBundle) {
    return SnapshotStatus::ContentKindMismatch;
  }
  const uint64_t file_len = static_cast<uint64_t>(len);
  const uint64_t root_off = static_cast<uint64_t>(h.root_offset);
  if ((root_off % 8u) != 0u) {
    return SnapshotStatus::UnalignedSection;
  }
  if (root_off < kRelocHeaderSize || root_off + kRelocBundleRootSize > file_len) {
    return SnapshotStatus::SectionOutOfBounds;
  }
  if (verify_checksum && compute_reloc_checksum(data, len) != h.checksum_xxh64) {
    return SnapshotStatus::ChecksumMismatch;
  }

  const uint8_t* rp = data + static_cast<size_t>(root_off);
  RelocBundleRoot b{};
  b.frame0.offset = read_le32(rp + 0u);
  b.frame0.len = read_le32(rp + 4u);
  b.state.offset = read_le32(rp + 8u);
  b.state.len = read_le32(rp + 12u);
  b.meta.offset = read_le32(rp + 16u);
  b.meta.len = read_le32(rp + 20u);
  b.reserved = read_le64(rp + 24u);
  if (b.reserved != 0u) {
    return SnapshotStatus::InvalidReservedHeader;
  }

  struct Span {
    uint64_t offset;
    uint64_t len;
    uint64_t end;
  };
  Span spans[3] = {
      {b.frame0.offset, b.frame0.len, 0u},
      {b.state.offset, b.state.len, 0u},
      {b.meta.offset, b.meta.len, 0u},
  };

  for (size_t i = 0u; i < 3u; ++i) {
    const uint64_t offset = spans[i].offset;
    const uint64_t slen = spans[i].len;
    if (slen == 0u) {
      if (offset != 0u && ((offset % kSnapshotSectionAlign) != 0u)) {
        return SnapshotStatus::UnalignedSection;
      }
      spans[i].end = offset;
      continue;
    }
    if ((offset % kSnapshotSectionAlign) != 0u) {
      return SnapshotStatus::UnalignedSection;
    }
    if (offset < kRelocHeaderSize) {
      return SnapshotStatus::SectionOutOfBounds;
    }
    if (slen > (std::numeric_limits<uint64_t>::max() - offset)) {
      return SnapshotStatus::SectionOutOfBounds;
    }
    const uint64_t end = offset + slen;
    if (end > file_len) {
      return SnapshotStatus::SectionOutOfBounds;
    }
    spans[i].end = end;
  }

  for (size_t i = 0u; i < 3u; ++i) {
    if (spans[i].len == 0u) {
      continue;
    }
    for (size_t j = i + 1u; j < 3u; ++j) {
      if (spans[j].len == 0u) {
        continue;
      }
      if (spans[i].offset < spans[j].end && spans[j].offset < spans[i].end) {
        return SnapshotStatus::OverlappingSections;
      }
    }
  }

  out_header = h;
  out_bundle = b;
  return SnapshotStatus::Ok;
}

inline uint64_t next_tmp_seq() noexcept {
  static std::atomic<uint64_t> counter{0u};
  return counter.fetch_add(1u, std::memory_order_relaxed);
}

#if defined(_WIN32)
inline bool utf8_to_wide(std::string_view utf8, std::wstring& out) {
  out.clear();
  if (utf8.empty() || utf8.size() > static_cast<size_t>(std::numeric_limits<int>::max())) {
    return false;
  }
  const int wide_len = ::MultiByteToWideChar(
      CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), nullptr, 0);
  if (wide_len <= 0) {
    return false;
  }
  out.assign(static_cast<size_t>(wide_len), L'\0');
  return ::MultiByteToWideChar(
             CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), out.data(), wide_len) > 0;
}
#endif

} // namespace detail

/// Computes the canonical 64-bit XXH64 digest over `data[0..len]` matching Rust `nve_engine::xxh64`.
inline uint64_t xxh64(const uint8_t* data, size_t len, uint64_t seed = 0u) noexcept {
  detail::Xxh64Hasher h(seed);
  h.update(data, len);
  return h.finish();
}

/// Encodes a complete, 8-byte-aligned `NVERELOC` bundle (`RELOC_KIND_BUNDLE`) in memory.
/// Matches Rust `nve_engine::reloc::encode_reloc_bundle` byte-for-byte.
inline std::vector<uint8_t> encode_snapshot(
    uint32_t app_schema_version,
    uint32_t flags,
    NveBufferSlice frame0,
    NveBufferSlice state,
    NveBufferSlice meta = NveBufferSlice{nullptr, 0}) {
  const size_t f0_len = (frame0.ptr != nullptr) ? frame0.len : 0u;
  const size_t st_len = (state.ptr != nullptr) ? state.len : 0u;
  const size_t mt_len = (meta.ptr != nullptr) ? meta.len : 0u;

  std::vector<uint8_t> buf;
  buf.reserve(
      kRelocBundleMinSize + detail::align_up_8(f0_len) + detail::align_up_8(st_len) +
      detail::align_up_8(mt_len));
  buf.resize(kRelocBundleMinSize, 0u);

  uint32_t frame0_offset = 0u;
  if (f0_len > 0u) {
    detail::pad_to_8(buf);
    frame0_offset = static_cast<uint32_t>(buf.size());
    buf.insert(buf.end(), frame0.ptr, frame0.ptr + f0_len);
  }

  uint32_t state_offset = 0u;
  if (st_len > 0u) {
    detail::pad_to_8(buf);
    state_offset = static_cast<uint32_t>(buf.size());
    buf.insert(buf.end(), state.ptr, state.ptr + st_len);
  }

  uint32_t meta_offset = 0u;
  if (mt_len > 0u) {
    detail::pad_to_8(buf);
    meta_offset = static_cast<uint32_t>(buf.size());
    buf.insert(buf.end(), meta.ptr, meta.ptr + mt_len);
  }

  detail::pad_to_8(buf);

  // Write RelocBundleRoot at offset 64.
  uint8_t* rp = buf.data() + kRelocHeaderSize;
  detail::write_le32(rp + 0u, frame0_offset);
  detail::write_le32(rp + 4u, static_cast<uint32_t>(f0_len));
  detail::write_le32(rp + 8u, state_offset);
  detail::write_le32(rp + 12u, static_cast<uint32_t>(st_len));
  detail::write_le32(rp + 16u, meta_offset);
  detail::write_le32(rp + 20u, static_cast<uint32_t>(mt_len));
  detail::write_le64(rp + 24u, 0u);

  // Write 64-byte RelocHeader at offset 0.
  const uint32_t payload_len = static_cast<uint32_t>(buf.size() - kRelocHeaderSize);
  std::memcpy(buf.data(), kRelocMagic, sizeof(kRelocMagic));
  detail::write_le32(buf.data() + kSnapshotFormatVersionOffset, kRelocFormatVersion);
  detail::write_le32(buf.data() + kSnapshotFlagsOffset, flags);
  detail::write_le32(buf.data() + kRelocPayloadLenOffset, payload_len);
  detail::write_le32(buf.data() + kRelocRootOffsetField, static_cast<uint32_t>(kRelocHeaderSize));
  detail::write_le32(buf.data() + kSnapshotSchemaVersionOffset, app_schema_version);
  detail::write_le32(buf.data() + kRelocContentKindOffset, kRelocKindBundle);
  detail::write_le64(buf.data() + 40u, 0u);
  detail::write_le64(buf.data() + 48u, 0u);
  detail::write_le64(buf.data() + 56u, 0u);

  const uint64_t checksum = detail::compute_reloc_checksum(buf.data(), buf.size());
  detail::write_le64(buf.data() + kSnapshotChecksumOffset, checksum);
  return buf;
}

/// Crash-safely writes an `NVERELOC` bundle via sibling temp file + fsync/FlushFileBuffers +
/// atomic rename (`rename` on POSIX, `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)` on Win32).
inline bool write_snapshot_atomic(
    std::string_view path,
    uint32_t schema_version,
    NveBufferSlice frame0,
    NveBufferSlice state,
    NveBufferSlice meta = NveBufferSlice{nullptr, 0},
    uint32_t flags = 0u,
    SnapshotStatus* out_status = nullptr) {
  auto set_status = [out_status](SnapshotStatus st) {
    if (out_status != nullptr) {
      *out_status = st;
    }
  };

  if (path.empty()) {
    set_status(SnapshotStatus::IoError);
    return false;
  }

  const std::vector<uint8_t> encoded = encode_snapshot(schema_version, flags, frame0, state, meta);

  const size_t sep = path.find_last_of("/\\");
  const std::string parent = (sep == std::string_view::npos) ? std::string{}
                                                             : std::string(path.substr(0u, sep + 1u));
  const std::string_view file_name =
      (sep == std::string_view::npos) ? path : path.substr(sep + 1u);
  if (file_name.empty()) {
    set_status(SnapshotStatus::IoError);
    return false;
  }

#if defined(_WIN32)
  const uint32_t pid = static_cast<uint32_t>(::GetCurrentProcessId());
#else
  const uint32_t pid = static_cast<uint32_t>(::getpid());
#endif
  const uint64_t seq = detail::next_tmp_seq();
  const std::string tmp_path = parent + "." + std::string(file_name) + ".tmp." +
                               std::to_string(pid) + "." + std::to_string(seq);

#if defined(_WIN32)
  std::wstring wtmp;
  std::wstring wdest;
  if (!detail::utf8_to_wide(tmp_path, wtmp) || !detail::utf8_to_wide(path, wdest)) {
    set_status(SnapshotStatus::IoError);
    return false;
  }

  HANDLE hfile = ::CreateFileW(
      wtmp.c_str(),
      GENERIC_WRITE,
      0,
      nullptr,
      CREATE_NEW,
      FILE_ATTRIBUTE_NORMAL,
      nullptr);
  if (hfile == INVALID_HANDLE_VALUE) {
    set_status(SnapshotStatus::IoError);
    return false;
  }

  bool ok = true;
  size_t written_total = 0u;
  while (written_total < encoded.size()) {
    const size_t remaining = encoded.size() - written_total;
    const DWORD chunk =
        static_cast<DWORD>(remaining > 0x40000000u ? 0x40000000u : remaining);
    DWORD bytes_written = 0u;
    if (!::WriteFile(hfile, encoded.data() + written_total, chunk, &bytes_written, nullptr) ||
        bytes_written == 0u) {
      ok = false;
      break;
    }
    written_total += static_cast<size_t>(bytes_written);
  }

  if (ok && !::FlushFileBuffers(hfile)) {
    ok = false;
  }
  ::CloseHandle(hfile);

  if (!ok) {
    (void)::DeleteFileW(wtmp.c_str());
    set_status(SnapshotStatus::IoError);
    return false;
  }

  if (!::MoveFileExW(
          wtmp.c_str(),
          wdest.c_str(),
          MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    (void)::DeleteFileW(wtmp.c_str());
    set_status(SnapshotStatus::IoError);
    return false;
  }

  set_status(SnapshotStatus::Ok);
  return true;
#else
  int open_flags = O_WRONLY | O_CREAT | O_EXCL;
#if defined(O_CLOEXEC)
  open_flags |= O_CLOEXEC;
#endif
  const int fd = ::open(tmp_path.c_str(), open_flags, static_cast<mode_t>(0644));
  if (fd < 0) {
    set_status(SnapshotStatus::IoError);
    return false;
  }

  bool ok = true;
  size_t written_total = 0u;
  while (written_total < encoded.size()) {
    const ssize_t n =
        ::write(fd, encoded.data() + written_total, encoded.size() - written_total);
    if (n < 0) {
      if (errno == EINTR) {
        continue;
      }
      ok = false;
      break;
    }
    if (n == 0) {
      ok = false;
      break;
    }
    written_total += static_cast<size_t>(n);
  }

  if (ok) {
    while (::fsync(fd) < 0) {
      if (errno == EINTR) {
        continue;
      }
      ok = false;
      break;
    }
  }
  if (::close(fd) < 0) {
    ok = false;
  }

  const std::string dest_cstr(path);
  if (!ok || ::rename(tmp_path.c_str(), dest_cstr.c_str()) != 0) {
    (void)::unlink(tmp_path.c_str());
    set_status(SnapshotStatus::IoError);
    return false;
  }

  const std::string dir_path = parent.empty() ? std::string(".") : parent;
  int dir_flags = O_RDONLY;
#if defined(O_CLOEXEC)
  dir_flags |= O_CLOEXEC;
#endif
  const int dir_fd = ::open(dir_path.c_str(), dir_flags);
  if (dir_fd >= 0) {
    (void)::fsync(dir_fd);
    (void)::close(dir_fd);
  }

  set_status(SnapshotStatus::Ok);
  return true;
#endif
}

/// Move-only RAII zero-copy memory-mapped `NVERELOC` bundle handle.
///
/// Section slices (`frame0_slice()`, `state_slice()`, `meta_slice()`) point directly inside the
/// mapped virtual address range with zero heap allocations or payload copies.
class MmapSnapshot {
 public:
  MmapSnapshot() noexcept = default;
  ~MmapSnapshot() { close(); }

  MmapSnapshot(const MmapSnapshot&) = delete;
  MmapSnapshot& operator=(const MmapSnapshot&) = delete;

  MmapSnapshot(MmapSnapshot&& other) noexcept
      : data_(other.data_),
        size_(other.size_),
        is_mmap_(other.is_mmap_),
        owned_(std::move(other.owned_)),
        header_(other.header_),
        bundle_(other.bundle_)
#if defined(_WIN32)
        ,
        mapping_handle_(other.mapping_handle_)
#endif
  {
    if (!is_mmap_ && !owned_.empty()) {
      data_ = owned_.data();
    }
    other.data_ = nullptr;
    other.size_ = 0u;
    other.is_mmap_ = false;
    other.header_ = RelocHeader{};
    other.bundle_ = RelocBundleRoot{};
#if defined(_WIN32)
    other.mapping_handle_ = nullptr;
#endif
  }

  MmapSnapshot& operator=(MmapSnapshot&& other) noexcept {
    if (this != &other) {
      close();
      data_ = other.data_;
      size_ = other.size_;
      is_mmap_ = other.is_mmap_;
      owned_ = std::move(other.owned_);
      header_ = other.header_;
      bundle_ = other.bundle_;
#if defined(_WIN32)
      mapping_handle_ = other.mapping_handle_;
      other.mapping_handle_ = nullptr;
#endif
      if (!is_mmap_ && !owned_.empty()) {
        data_ = owned_.data();
      }
      other.data_ = nullptr;
      other.size_ = 0u;
      other.is_mmap_ = false;
      other.header_ = RelocHeader{};
      other.bundle_ = RelocBundleRoot{};
    }
    return *this;
  }

  void close() noexcept {
    if (is_mmap_ && data_ != nullptr) {
#if defined(_WIN32)
      (void)::UnmapViewOfFile(data_);
      if (mapping_handle_ != nullptr) {
        (void)::CloseHandle(mapping_handle_);
        mapping_handle_ = nullptr;
      }
#else
      if (size_ > 0u) {
        (void)::munmap(const_cast<uint8_t*>(data_), size_);
      }
#endif
    }
    data_ = nullptr;
    size_ = 0u;
    is_mmap_ = false;
    owned_.clear();
    header_ = RelocHeader{};
    bundle_ = RelocBundleRoot{};
  }

  /// Opens and memory-maps an `NVERELOC` file, validating header, alignment, bounds, overlap,
  /// and (when `verify_checksum` is true) the XXH64 checksum.
  static bool open(
      std::string_view path,
      MmapSnapshot& out,
      bool verify_checksum = true,
      SnapshotStatus* out_status = nullptr) {
    out.close();
    auto set_status = [out_status](SnapshotStatus st) {
      if (out_status != nullptr) {
        *out_status = st;
      }
    };

    if (path.empty()) {
      set_status(SnapshotStatus::IoError);
      return false;
    }

    MmapSnapshot candidate;
#if defined(_WIN32)
    std::wstring wpath;
    if (!detail::utf8_to_wide(path, wpath)) {
      set_status(SnapshotStatus::IoError);
      return false;
    }
    HANDLE hfile = ::CreateFileW(
        wpath.c_str(),
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        nullptr,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        nullptr);
    if (hfile == INVALID_HANDLE_VALUE) {
      set_status(SnapshotStatus::IoError);
      return false;
    }
    if (::GetFileType(hfile) != FILE_TYPE_DISK) {
      ::CloseHandle(hfile);
      set_status(SnapshotStatus::IoError);
      return false;
    }
    LARGE_INTEGER file_size{};
    if (!::GetFileSizeEx(hfile, &file_size) || file_size.QuadPart < 0) {
      ::CloseHandle(hfile);
      set_status(SnapshotStatus::IoError);
      return false;
    }
    const uint64_t u64_len = static_cast<uint64_t>(file_size.QuadPart);
    if (u64_len == 0u) {
      ::CloseHandle(hfile);
      set_status(SnapshotStatus::Truncated);
      return false;
    }
    if (u64_len > static_cast<uint64_t>(std::numeric_limits<size_t>::max())) {
      ::CloseHandle(hfile);
      set_status(SnapshotStatus::IoError);
      return false;
    }
    const size_t len = static_cast<size_t>(u64_len);

    HANDLE mapping = ::CreateFileMappingW(hfile, nullptr, PAGE_READONLY, 0, 0, nullptr);
    ::CloseHandle(hfile);
    if (mapping == nullptr) {
      set_status(SnapshotStatus::IoError);
      return false;
    }

    void* view = ::MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, len);
    if (view == nullptr) {
      ::CloseHandle(mapping);
      set_status(SnapshotStatus::IoError);
      return false;
    }

    candidate.data_ = static_cast<const uint8_t*>(view);
    candidate.size_ = len;
    candidate.is_mmap_ = true;
    candidate.mapping_handle_ = mapping;
#else
    const std::string path_cstr(path);
    int flags = O_RDONLY;
#if defined(O_CLOEXEC)
    flags |= O_CLOEXEC;
#endif
    const int fd = ::open(path_cstr.c_str(), flags);
    if (fd < 0) {
      set_status(SnapshotStatus::IoError);
      return false;
    }

    struct stat st {};
    if (::fstat(fd, &st) != 0) {
      (void)::close(fd);
      set_status(SnapshotStatus::IoError);
      return false;
    }
    if (!S_ISREG(st.st_mode) || st.st_size < 0) {
      (void)::close(fd);
      set_status(SnapshotStatus::IoError);
      return false;
    }

    const uint64_t u64_len = static_cast<uint64_t>(st.st_size);
    if (u64_len == 0u) {
      (void)::close(fd);
      set_status(SnapshotStatus::Truncated);
      return false;
    }
    if (u64_len > static_cast<uint64_t>(std::numeric_limits<size_t>::max())) {
      (void)::close(fd);
      set_status(SnapshotStatus::IoError);
      return false;
    }
    const size_t len = static_cast<size_t>(u64_len);

    void* mapped = ::mmap(nullptr, len, PROT_READ, MAP_PRIVATE, fd, 0);
    (void)::close(fd);
    if (mapped == MAP_FAILED || mapped == nullptr) {
      set_status(SnapshotStatus::IoError);
      return false;
    }

    candidate.data_ = static_cast<const uint8_t*>(mapped);
    candidate.size_ = len;
    candidate.is_mmap_ = true;
#endif

    const SnapshotStatus st_code = detail::decode_and_validate_reloc_bundle(
        candidate.data_, candidate.size_, verify_checksum, candidate.header_, candidate.bundle_);
    if (st_code != SnapshotStatus::Ok) {
      set_status(st_code);
      return false;
    }

    out = std::move(candidate);
    set_status(SnapshotStatus::Ok);
    return true;
  }

  /// Opens an `NVERELOC` file, validates integrity, and enforces
  /// `app_schema_version == expected_schema_version`.
  static bool open_with_schema(
      std::string_view path,
      uint32_t expected_schema_version,
      MmapSnapshot& out,
      bool verify_checksum = true,
      SnapshotStatus* out_status = nullptr) {
    MmapSnapshot candidate;
    if (!open(path, candidate, verify_checksum, out_status)) {
      out.close();
      return false;
    }
    if (candidate.header_.app_schema_version != expected_schema_version) {
      if (out_status != nullptr) {
        *out_status = SnapshotStatus::SchemaVersionMismatch;
      }
      out.close();
      return false;
    }
    out = std::move(candidate);
    if (out_status != nullptr) {
      *out_status = SnapshotStatus::Ok;
    }
    return true;
  }

  /// Wraps an in-memory `NVERELOC` buffer with identical header, alignment, bounds, overlap,
  /// and optional XXH64 checksum validation.
  static bool from_bytes(
      std::vector<uint8_t> bytes,
      MmapSnapshot& out,
      bool verify_checksum = true,
      SnapshotStatus* out_status = nullptr) {
    out.close();
    auto set_status = [out_status](SnapshotStatus st) {
      if (out_status != nullptr) {
        *out_status = st;
      }
    };

    MmapSnapshot candidate;
    candidate.owned_ = std::move(bytes);
    candidate.data_ = candidate.owned_.empty() ? nullptr : candidate.owned_.data();
    candidate.size_ = candidate.owned_.size();
    candidate.is_mmap_ = false;

    const SnapshotStatus st_code = detail::decode_and_validate_reloc_bundle(
        candidate.data_, candidate.size_, verify_checksum, candidate.header_, candidate.bundle_);
    if (st_code != SnapshotStatus::Ok) {
      set_status(st_code);
      return false;
    }

    out = std::move(candidate);
    set_status(SnapshotStatus::Ok);
    return true;
  }

  bool is_open() const noexcept { return data_ != nullptr && size_ >= kRelocBundleMinSize; }
  explicit operator bool() const noexcept { return is_open(); }

  /// Verifies the stored XXH64 checksum against the mapped bytes.
  bool verify_checksum() const noexcept {
    if (!is_open()) {
      return false;
    }
    return detail::compute_reloc_checksum(data_, size_) == header_.checksum_xxh64;
  }

  const RelocHeader& header() const noexcept { return header_; }
  const RelocBundleRoot& bundle_root() const noexcept { return bundle_; }
  uint32_t format_version() const noexcept { return header_.format_version; }
  uint32_t app_schema_version() const noexcept { return header_.app_schema_version; }
  uint32_t flags() const noexcept { return header_.flags; }
  uint64_t checksum_xxh64() const noexcept { return header_.checksum_xxh64; }

  const uint8_t* data() const noexcept { return data_; }
  size_t size() const noexcept { return size_; }

  NveBufferSlice raw_slice() const noexcept {
    return is_open() ? NveBufferSlice{data_, size_} : NveBufferSlice{nullptr, 0u};
  }

  /// Zero-copy borrowed slice of the freeze-dried Frame-0 `CommandBuffer`.
  NveBufferSlice frame0_slice() const noexcept {
    if (!is_open() || bundle_.frame0.len == 0u) {
      return NveBufferSlice{nullptr, 0u};
    }
    return NveBufferSlice{
        data_ + static_cast<size_t>(bundle_.frame0.offset),
        static_cast<size_t>(bundle_.frame0.len),
    };
  }

  /// Zero-copy borrowed slice of the application state section.
  NveBufferSlice state_slice() const noexcept {
    if (!is_open() || bundle_.state.len == 0u) {
      return NveBufferSlice{nullptr, 0u};
    }
    return NveBufferSlice{
        data_ + static_cast<size_t>(bundle_.state.offset),
        static_cast<size_t>(bundle_.state.len),
    };
  }

  /// Zero-copy borrowed slice of the optional metadata/viewport section.
  NveBufferSlice meta_slice() const noexcept {
    if (!is_open() || bundle_.meta.len == 0u) {
      return NveBufferSlice{nullptr, 0u};
    }
    return NveBufferSlice{
        data_ + static_cast<size_t>(bundle_.meta.offset),
        static_cast<size_t>(bundle_.meta.len),
    };
  }

 private:
  const uint8_t* data_{nullptr};
  size_t size_{0u};
  bool is_mmap_{false};
  std::vector<uint8_t> owned_;
  RelocHeader header_{};
  RelocBundleRoot bundle_{};
#if defined(_WIN32)
  HANDLE mapping_handle_{nullptr};
#endif
};

using MmapReloc = MmapSnapshot;

} // namespace nve
