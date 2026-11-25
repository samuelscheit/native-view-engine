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

/// 8-byte file magic identifying an NVE instant-boot snapshot container (`*b"NVE_SNAP"`).
inline constexpr uint8_t kSnapshotMagic[8] = {'N', 'V', 'E', '_', 'S', 'N', 'A', 'P'};

/// Current snapshot container format version (matches `crates/nve-engine/src/snapshot.rs`).
inline constexpr uint32_t kSnapshotFormatVersion = 1u;

/// Fixed byte length of the aligned `NVE_SNAP` header (32-byte prefix + 48-byte section table).
inline constexpr size_t kSnapshotHeaderSize = 80u;

/// Byte alignment enforced for every payload section offset.
inline constexpr size_t kSnapshotSectionAlign = 8u;

/// Header offset of the 8-byte `checksum_fnv64` field.
inline constexpr size_t kSnapshotChecksumOffset = 24u;
inline constexpr size_t kSnapshotChecksumEnd = 32u;

/// FNV-1a 64-bit parameters matching `crates/nve-engine/src/snapshot.rs`.
inline constexpr uint64_t kFnv1a64OffsetBasis = 0xcbf29ce484222325ULL;
inline constexpr uint64_t kFnv1a64Prime = 0x00000100000001b3ULL;

/// Validation & I/O status codes for `MmapSnapshot` and `write_snapshot_atomic`.
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
};

/// Decoded fixed 80-byte header of an `NVE_SNAP` binary container.
struct SnapshotHeader {
  uint8_t magic[8]{0, 0, 0, 0, 0, 0, 0, 0};
  uint32_t format_version{0};
  uint32_t app_schema_version{0};
  uint32_t flags{0};
  uint64_t checksum_fnv64{0};
  uint64_t frame0_offset{0};
  uint64_t frame0_len{0};
  uint64_t state_offset{0};
  uint64_t state_len{0};
  uint64_t meta_offset{0};
  uint64_t meta_len{0};
};

/// Continues an FNV-1a 64-bit hash with additional bytes.
inline uint64_t fnv1a_64_update(uint64_t hash, const uint8_t* data, size_t len) noexcept {
  if (data == nullptr || len == 0) {
    return hash;
  }
  for (size_t i = 0; i < len; ++i) {
    hash ^= static_cast<uint64_t>(data[i]);
    hash *= kFnv1a64Prime;
  }
  return hash;
}

/// Computes the FNV-1a 64-bit hash over `data[0..len]`.
inline uint64_t fnv1a_64(const uint8_t* data, size_t len) noexcept {
  return fnv1a_64_update(kFnv1a64OffsetBasis, data, len);
}

namespace detail {

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

inline uint64_t compute_snapshot_checksum(const uint8_t* data, size_t len) noexcept {
  const uint64_t h = fnv1a_64_update(kFnv1a64OffsetBasis, data, kSnapshotChecksumOffset);
  return fnv1a_64_update(h, data + kSnapshotChecksumEnd, len - kSnapshotChecksumEnd);
}

inline SnapshotStatus decode_and_validate_header(
    const uint8_t* data,
    size_t len,
    SnapshotHeader& out_header) noexcept {
  if (data == nullptr || len < kSnapshotHeaderSize) {
    return SnapshotStatus::Truncated;
  }
  if (std::memcmp(data, kSnapshotMagic, sizeof(kSnapshotMagic)) != 0) {
    return SnapshotStatus::InvalidMagic;
  }

  const uint32_t format_version = read_le32(data + 8u);
  if (format_version != kSnapshotFormatVersion) {
    return SnapshotStatus::UnsupportedFormatVersion;
  }

  const uint32_t app_schema_version = read_le32(data + 12u);
  const uint32_t flags = read_le32(data + 16u);
  const uint32_t reserved = read_le32(data + 20u);
  if (reserved != 0u) {
    return SnapshotStatus::InvalidReservedHeader;
  }

  SnapshotHeader h{};
  std::memcpy(h.magic, data, sizeof(h.magic));
  h.format_version = format_version;
  h.app_schema_version = app_schema_version;
  h.flags = flags;
  h.checksum_fnv64 = read_le64(data + 24u);
  h.frame0_offset = read_le64(data + 32u);
  h.frame0_len = read_le64(data + 40u);
  h.state_offset = read_le64(data + 48u);
  h.state_len = read_le64(data + 56u);
  h.meta_offset = read_le64(data + 64u);
  h.meta_len = read_le64(data + 72u);

  const uint64_t file_len = static_cast<uint64_t>(len);
  const uint64_t max_size_t = static_cast<uint64_t>(std::numeric_limits<size_t>::max());

  struct Span {
    uint64_t offset;
    uint64_t len;
    uint64_t end;
  };
  Span spans[3] = {
      {h.frame0_offset, h.frame0_len, 0u},
      {h.state_offset, h.state_len, 0u},
      {h.meta_offset, h.meta_len, 0u},
  };

  for (size_t i = 0; i < 3u; ++i) {
    const uint64_t offset = spans[i].offset;
    const uint64_t slen = spans[i].len;
    if ((offset % static_cast<uint64_t>(kSnapshotSectionAlign)) != 0u) {
      return SnapshotStatus::UnalignedSection;
    }
    if (offset < static_cast<uint64_t>(kSnapshotHeaderSize)) {
      return SnapshotStatus::SectionOutOfBounds;
    }
    if (slen > (std::numeric_limits<uint64_t>::max() - offset)) {
      return SnapshotStatus::SectionOutOfBounds;
    }
    const uint64_t end = offset + slen;
    if (end > file_len || end > max_size_t) {
      return SnapshotStatus::SectionOutOfBounds;
    }
    spans[i].end = end;
  }

  for (size_t i = 0; i < 3u; ++i) {
    if (spans[i].offset == spans[i].end) {
      continue;
    }
    for (size_t j = i + 1u; j < 3u; ++j) {
      if (spans[j].offset == spans[j].end) {
        continue;
      }
      if (spans[i].offset < spans[j].end && spans[j].offset < spans[i].end) {
        return SnapshotStatus::OverlappingSections;
      }
    }
  }

  out_header = h;
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

/// Encodes a complete, 8-byte aligned `NVE_SNAP` binary container in memory.
/// Matches `nve_engine::snapshot::encode_snapshot` byte-for-byte.
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
      kSnapshotHeaderSize + detail::align_up_8(f0_len) + detail::align_up_8(st_len) +
      detail::align_up_8(mt_len));
  buf.resize(kSnapshotHeaderSize, 0u);

  const uint64_t frame0_offset = static_cast<uint64_t>(buf.size());
  if (f0_len > 0u) {
    buf.insert(buf.end(), frame0.ptr, frame0.ptr + f0_len);
  }
  detail::pad_to_8(buf);

  const uint64_t state_offset = static_cast<uint64_t>(buf.size());
  if (st_len > 0u) {
    buf.insert(buf.end(), state.ptr, state.ptr + st_len);
  }
  detail::pad_to_8(buf);

  const uint64_t meta_offset = static_cast<uint64_t>(buf.size());
  if (mt_len > 0u) {
    buf.insert(buf.end(), meta.ptr, meta.ptr + mt_len);
  }
  detail::pad_to_8(buf);

  std::memcpy(buf.data(), kSnapshotMagic, sizeof(kSnapshotMagic));
  detail::write_le32(buf.data() + 8u, kSnapshotFormatVersion);
  detail::write_le32(buf.data() + 12u, app_schema_version);
  detail::write_le32(buf.data() + 16u, flags);
  detail::write_le32(buf.data() + 20u, 0u);
  detail::write_le64(buf.data() + 32u, frame0_offset);
  detail::write_le64(buf.data() + 40u, static_cast<uint64_t>(f0_len));
  detail::write_le64(buf.data() + 48u, state_offset);
  detail::write_le64(buf.data() + 56u, static_cast<uint64_t>(st_len));
  detail::write_le64(buf.data() + 64u, meta_offset);
  detail::write_le64(buf.data() + 72u, static_cast<uint64_t>(mt_len));

  const uint64_t checksum = detail::compute_snapshot_checksum(buf.data(), buf.size());
  detail::write_le64(buf.data() + kSnapshotChecksumOffset, checksum);
  return buf;
}

/// Crash-safely writes an `NVE_SNAP` snapshot via sibling temp file + fsync/FlushFileBuffers +
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

  // Best-effort parent directory fsync for durability across power loss.
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

/// Move-only RAII zero-copy memory-mapped `NVE_SNAP` snapshot handle.
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
        header_(other.header_)
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
    other.header_ = SnapshotHeader{};
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
      other.header_ = SnapshotHeader{};
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
    header_ = SnapshotHeader{};
  }

  /// Opens and memory-maps a snapshot file, validating header, alignment, bounds, overlap,
  /// and (when `verify_checksum` is true) the FNV-1a 64-bit checksum.
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
      // Never pass len == 0 to mmap (POSIX returns EINVAL).
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

    const SnapshotStatus st_code =
        detail::decode_and_validate_header(candidate.data_, candidate.size_, candidate.header_);
    if (st_code != SnapshotStatus::Ok) {
      set_status(st_code);
      return false;
    }
    if (verify_checksum && !candidate.verify_checksum()) {
      set_status(SnapshotStatus::ChecksumMismatch);
      return false;
    }

    out = std::move(candidate);
    set_status(SnapshotStatus::Ok);
    return true;
  }

  /// Opens a snapshot file, validates integrity, and enforces
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

  /// Wraps an in-memory snapshot buffer with identical header, alignment, bounds, overlap,
  /// and optional checksum validation.
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

    const SnapshotStatus st_code =
        detail::decode_and_validate_header(candidate.data_, candidate.size_, candidate.header_);
    if (st_code != SnapshotStatus::Ok) {
      set_status(st_code);
      return false;
    }
    if (verify_checksum && !candidate.verify_checksum()) {
      set_status(SnapshotStatus::ChecksumMismatch);
      return false;
    }

    out = std::move(candidate);
    set_status(SnapshotStatus::Ok);
    return true;
  }

  bool is_open() const noexcept { return data_ != nullptr && size_ >= kSnapshotHeaderSize; }
  explicit operator bool() const noexcept { return is_open(); }

  /// Verifies the stored FNV-1a 64-bit checksum against the mapped bytes.
  bool verify_checksum() const noexcept {
    if (!is_open()) {
      return false;
    }
    return detail::compute_snapshot_checksum(data_, size_) == header_.checksum_fnv64;
  }

  const SnapshotHeader& header() const noexcept { return header_; }
  uint32_t format_version() const noexcept { return header_.format_version; }
  uint32_t app_schema_version() const noexcept { return header_.app_schema_version; }
  uint32_t flags() const noexcept { return header_.flags; }
  uint64_t checksum_fnv64() const noexcept { return header_.checksum_fnv64; }

  const uint8_t* data() const noexcept { return data_; }
  size_t size() const noexcept { return size_; }

  NveBufferSlice raw_slice() const noexcept {
    return is_open() ? NveBufferSlice{data_, size_} : NveBufferSlice{nullptr, 0u};
  }

  /// Zero-copy borrowed slice of the freeze-dried Frame-0 `CommandBuffer`.
  NveBufferSlice frame0_slice() const noexcept {
    if (!is_open()) {
      return NveBufferSlice{nullptr, 0u};
    }
    return NveBufferSlice{
        data_ + static_cast<size_t>(header_.frame0_offset),
        static_cast<size_t>(header_.frame0_len),
    };
  }

  /// Zero-copy borrowed slice of the application state section.
  NveBufferSlice state_slice() const noexcept {
    if (!is_open()) {
      return NveBufferSlice{nullptr, 0u};
    }
    return NveBufferSlice{
        data_ + static_cast<size_t>(header_.state_offset),
        static_cast<size_t>(header_.state_len),
    };
  }

  /// Zero-copy borrowed slice of the optional metadata/viewport section.
  NveBufferSlice meta_slice() const noexcept {
    if (!is_open()) {
      return NveBufferSlice{nullptr, 0u};
    }
    return NveBufferSlice{
        data_ + static_cast<size_t>(header_.meta_offset),
        static_cast<size_t>(header_.meta_len),
    };
  }

 private:
  const uint8_t* data_{nullptr};
  size_t size_{0u};
  bool is_mmap_{false};
  std::vector<uint8_t> owned_;
  SnapshotHeader header_{};
#if defined(_WIN32)
  HANDLE mapping_handle_{nullptr};
#endif
};

} // namespace nve
