//! Static C++20 runtime primitives template emitted by `reloc::emit_cpp`.

use crate::writer::CodeWriter;

use crate::schema::reloc::ir::{
    RELOC_HEADER_ALIGN, RELOC_HEADER_SIZE, RELOC_SLICE_ALIGN, RELOC_SLICE_SIZE,
};

pub(super) fn emit_core_primitives(out: &mut CodeWriter) {
    writeln!(
        out,
        r#"enum class VerifyMode : uint8_t {{
  Full = 0,
  HeaderOnly = 1,
  Unchecked = 2,
}};

enum class RelocError : uint8_t {{
  Ok = 0,
  Truncated = 1,
  Misaligned = 2,
  InvalidMagic = 3,
  SchemaHashMismatch = 4,
  OutOfBounds = 5,
  LengthOverflow = 6,
  InvalidUtf8 = 7,
  InvalidBool = 8,
  InvalidEnumDiscriminant = 9,
  DepthExceeded = 10,
}};

template <typename T>
struct RelocResult {{
  T value{{}};
  RelocError error = RelocError::Ok;

  [[nodiscard]] constexpr bool ok() const noexcept {{ return error == RelocError::Ok; }}
  [[nodiscard]] static constexpr RelocResult success(T v) noexcept {{
    return RelocResult{{v, RelocError::Ok}};
  }}
  [[nodiscard]] static constexpr RelocResult failure(RelocError err) noexcept {{
    return RelocResult{{T{{}}, err}};
  }}
}};

struct RelocHeader {{
  uint8_t magic[8];
  uint64_t schema_hash;
  uint32_t total_len;
  uint32_t flags;
  uint32_t root_offset;
  uint32_t _pad;

  [[nodiscard]] std::span<const uint8_t> as_bytes() const noexcept {{
    return std::span<const uint8_t>(reinterpret_cast<const uint8_t*>(this), sizeof(RelocHeader));
  }}
}};

static_assert(sizeof(RelocHeader) == {RELOC_HEADER_SIZE}u);
static_assert(alignof(RelocHeader) == {RELOC_HEADER_ALIGN}u);
static_assert(offsetof(RelocHeader, magic) == 0u);
static_assert(offsetof(RelocHeader, schema_hash) == 8u);
static_assert(offsetof(RelocHeader, total_len) == 16u);
static_assert(offsetof(RelocHeader, flags) == 20u);
static_assert(offsetof(RelocHeader, root_offset) == 24u);
static_assert(offsetof(RelocHeader, _pad) == 28u);

namespace detail {{

[[nodiscard]] inline uint16_t read_u16_le(const uint8_t* p) noexcept {{
  return static_cast<uint16_t>(static_cast<uint16_t>(p[0]) | (static_cast<uint16_t>(p[1]) << 8u));
}}

[[nodiscard]] inline uint32_t read_u32_le(const uint8_t* p) noexcept {{
  return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8u) |
         (static_cast<uint32_t>(p[2]) << 16u) | (static_cast<uint32_t>(p[3]) << 24u);
}}

[[nodiscard]] inline uint64_t read_u64_le(const uint8_t* p) noexcept {{
  uint64_t v = 0;
  for (size_t i = 0; i < 8u; ++i) {{
    v |= static_cast<uint64_t>(p[i]) << (i * 8u);
  }}
  return v;
}}

[[nodiscard]] inline bool is_valid_utf8(const uint8_t* data, size_t len) noexcept {{
  size_t i = 0;
  while (i < len) {{
    const uint8_t b0 = data[i];
    if (b0 <= 0x7Fu) {{
      ++i;
      continue;
    }}
    if (b0 >= 0xC2u && b0 <= 0xDFu) {{
      if (i + 1u >= len) return false;
      const uint8_t b1 = data[i + 1u];
      if ((b1 & 0xC0u) != 0x80u) return false;
      i += 2u;
      continue;
    }}
    if (b0 >= 0xE0u && b0 <= 0xEFu) {{
      if (i + 2u >= len) return false;
      const uint8_t b1 = data[i + 1u];
      const uint8_t b2 = data[i + 2u];
      if ((b1 & 0xC0u) != 0x80u || (b2 & 0xC0u) != 0x80u) return false;
      if (b0 == 0xE0u && b1 < 0xA0u) return false;
      if (b0 == 0xEDu && b1 > 0x9Fu) return false;
      i += 3u;
      continue;
    }}
    if (b0 >= 0xF0u && b0 <= 0xF4u) {{
      if (i + 3u >= len) return false;
      const uint8_t b1 = data[i + 1u];
      const uint8_t b2 = data[i + 2u];
      const uint8_t b3 = data[i + 3u];
      if ((b1 & 0xC0u) != 0x80u || (b2 & 0xC0u) != 0x80u || (b3 & 0xC0u) != 0x80u) return false;
      if (b0 == 0xF0u && b1 < 0x90u) return false;
      if (b0 == 0xF4u && b1 > 0x8Fu) return false;
      i += 4u;
      continue;
    }}
    return false;
  }}
  return true;
}}

[[nodiscard]] inline RelocError verify_reloc_str(const uint8_t* slab, size_t total_len,
                                                 uint32_t offset, uint32_t len) noexcept {{
  const size_t off = static_cast<size_t>(offset);
  const size_t byte_len = static_cast<size_t>(len);
  if (byte_len == 0u) {{
    return off <= total_len ? RelocError::Ok : RelocError::OutOfBounds;
  }}
  if (off < kHeaderSize) {{
    return RelocError::OutOfBounds;
  }}
  if (len > std::numeric_limits<uint32_t>::max() - offset) {{
    return RelocError::LengthOverflow;
  }}
  if (byte_len > total_len || off > total_len - byte_len) {{
    return RelocError::OutOfBounds;
  }}
  if (!is_valid_utf8(slab + off, byte_len)) {{
    return RelocError::InvalidUtf8;
  }}
  return RelocError::Ok;
}}

template <typename T>
struct RelocTraits {{
  static constexpr bool kNeedsVerify = false;
  static RelocError verify_bytes(const uint8_t*, size_t, size_t, size_t) noexcept {{
    return RelocError::Ok;
  }}
}};

template <>
struct RelocTraits<bool> {{
  static constexpr bool kNeedsVerify = true;
  static RelocError verify_bytes(const uint8_t* slab, size_t, size_t base, size_t) noexcept {{
    const uint8_t b = slab[base];
    return (b == 0u || b == 1u) ? RelocError::Ok : RelocError::InvalidBool;
  }}
}};

template <typename T, size_t N>
struct RelocTraits<std::array<T, N>> {{
  static constexpr bool kNeedsVerify = RelocTraits<T>::kNeedsVerify;
  static RelocError verify_bytes(const uint8_t* slab, size_t total_len, size_t base,
                                 size_t depth) noexcept {{
    if constexpr (RelocTraits<T>::kNeedsVerify) {{
      for (size_t i = 0; i < N; ++i) {{
        const RelocError err =
            RelocTraits<T>::verify_bytes(slab, total_len, base + i * sizeof(T), depth);
        if (err != RelocError::Ok) return err;
      }}
    }}
    return RelocError::Ok;
  }}
}};

template <typename T>
[[nodiscard]] inline RelocError verify_reloc_slice(const uint8_t* slab, size_t total_len,
                                                   uint32_t offset, uint32_t len,
                                                   size_t depth) noexcept {{
  if (depth >= kMaxVerifyDepth) {{
    return RelocError::DepthExceeded;
  }}
  const size_t off = static_cast<size_t>(offset);
  const size_t count = static_cast<size_t>(len);
  if (count == 0u) {{
    return off <= total_len ? RelocError::Ok : RelocError::OutOfBounds;
  }}
  if (off < kHeaderSize) {{
    return RelocError::OutOfBounds;
  }}
  constexpr size_t elem_size = sizeof(T);
  constexpr size_t elem_align = alignof(T);
  if (count > (static_cast<size_t>(std::numeric_limits<uint32_t>::max()) / elem_size)) {{
    return RelocError::LengthOverflow;
  }}
  const size_t byte_len = count * elem_size;
  if (byte_len > static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - off) {{
    return RelocError::LengthOverflow;
  }}
  if (byte_len > total_len || off > total_len - byte_len) {{
    return RelocError::OutOfBounds;
  }}
  const uintptr_t abs_addr = reinterpret_cast<uintptr_t>(slab) + off;
  if ((off % elem_align) != 0u || (abs_addr % elem_align) != 0u) {{
    return RelocError::Misaligned;
  }}
  if constexpr (RelocTraits<T>::kNeedsVerify) {{
    for (size_t i = 0; i < count; ++i) {{
      const RelocError err =
          RelocTraits<T>::verify_bytes(slab, total_len, off + i * elem_size, depth + 1u);
      if (err != RelocError::Ok) return err;
    }}
  }}
  return RelocError::Ok;
}}

}} // namespace detail

template <typename T>
struct RelocSlice {{
  uint32_t offset;
  uint32_t len;

  [[nodiscard]] constexpr size_t size() const noexcept {{ return static_cast<size_t>(len); }}
  [[nodiscard]] constexpr bool empty() const noexcept {{ return len == 0u; }}

  [[nodiscard]] std::span<const T> as_span_unchecked(const uint8_t* slab) const noexcept {{
    if (len == 0u) return {{}};
    return std::span<const T>(reinterpret_cast<const T*>(slab + offset), len);
  }}

  [[nodiscard]] RelocResult<std::span<const T>> resolve(std::span<const uint8_t> slab) const noexcept {{
    const RelocError err =
        detail::verify_reloc_slice<T>(slab.data(), slab.size(), offset, len, 0u);
    if (err != RelocError::Ok) return RelocResult<std::span<const T>>::failure(err);
    return RelocResult<std::span<const T>>::success(as_span_unchecked(slab.data()));
  }}
}};

struct RelocStr {{
  uint32_t offset;
  uint32_t len;

  [[nodiscard]] constexpr size_t size() const noexcept {{ return static_cast<size_t>(len); }}
  [[nodiscard]] constexpr bool empty() const noexcept {{ return len == 0u; }}

  [[nodiscard]] std::span<const uint8_t> as_bytes() const noexcept {{
    return std::span<const uint8_t>(reinterpret_cast<const uint8_t*>(this), sizeof(RelocStr));
  }}

  [[nodiscard]] std::string_view as_str_unchecked(const uint8_t* slab) const noexcept {{
    if (len == 0u) return {{}};
    return std::string_view(reinterpret_cast<const char*>(slab + offset), len);
  }}

  [[nodiscard]] RelocResult<std::string_view> as_str(std::span<const uint8_t> slab) const noexcept {{
    const RelocError err = detail::verify_reloc_str(slab.data(), slab.size(), offset, len);
    if (err != RelocError::Ok) return RelocResult<std::string_view>::failure(err);
    return RelocResult<std::string_view>::success(as_str_unchecked(slab.data()));
  }}
}};

static_assert(sizeof(RelocSlice<uint32_t>) == {RELOC_SLICE_SIZE}u);
static_assert(alignof(RelocSlice<uint32_t>) == {RELOC_SLICE_ALIGN}u);
static_assert(offsetof(RelocSlice<uint32_t>, offset) == 0u);
static_assert(offsetof(RelocSlice<uint32_t>, len) == 4u);
static_assert(sizeof(RelocStr) == {RELOC_SLICE_SIZE}u);
static_assert(alignof(RelocStr) == {RELOC_SLICE_ALIGN}u);
static_assert(offsetof(RelocStr, offset) == 0u);
static_assert(offsetof(RelocStr, len) == 4u);

namespace detail {{

template <typename T>
struct RelocTraits<RelocSlice<T>> {{
  static constexpr bool kNeedsVerify = true;
  static RelocError verify_bytes(const uint8_t* slab, size_t total_len, size_t base,
                                 size_t depth) noexcept {{
    const uint32_t off = read_u32_le(slab + base);
    const uint32_t len = read_u32_le(slab + base + 4u);
    return verify_reloc_slice<T>(slab, total_len, off, len, depth);
  }}
}};

template <>
struct RelocTraits<RelocStr> {{
  static constexpr bool kNeedsVerify = true;
  static RelocError verify_bytes(const uint8_t* slab, size_t total_len, size_t base,
                                 size_t) noexcept {{
    const uint32_t off = read_u32_le(slab + base);
    const uint32_t len = read_u32_le(slab + base + 4u);
    return verify_reloc_str(slab, total_len, off, len);
  }}
}};

}} // namespace detail
"#
    );
}
