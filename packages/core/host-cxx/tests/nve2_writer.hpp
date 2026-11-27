// Minimal binary `NVE2` frame writer shared by the C++20 runtime tests (little-endian words,
// 24-byte header, `[op | table << 8 | count << 16][payload_words]` op heads, 4-byte padding).
#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>
#include <utility>
#include <vector>

#include "nve/host/xxh64.hpp"

namespace nve_test {

inline constexpr uint32_t kNve2Magic = 0x3245564Eu;  // "NVE2" LE
inline constexpr uint32_t kNve2Version = 1u;

enum class Op : uint8_t {
  DeclareTables = 1u,
  CreateNodes = 3u,
  DefineResources = 4u,
  Patch = 6u,
  SetStrings = 8u,
  SetChildren = 9u,
};

inline void push_u32(std::vector<uint8_t>& out, uint32_t v) {
  for (unsigned shift = 0u; shift < 32u; shift += 8u) out.push_back(static_cast<uint8_t>(v >> shift));
}

inline void write_u32_at(std::vector<uint8_t>& out, size_t off, uint32_t v) {
  for (unsigned i = 0u; i < 4u; ++i) out[off + i] = static_cast<uint8_t>(v >> (i * 8u));
}

inline void push_padded(std::vector<uint8_t>& out, std::span<const uint8_t> bytes) {
  out.insert(out.end(), bytes.begin(), bytes.end());
  while ((out.size() & 3u) != 0u) out.push_back(0u);
}

inline void push_padded(std::vector<uint8_t>& out, std::string_view sv) {
  push_padded(out, std::span<const uint8_t>(reinterpret_cast<const uint8_t*>(sv.data()), sv.size()));
}

/// One `DeclareTables` entry (static tables must match the compiled descriptor exactly).
struct TableDecl {
  uint8_t wire_slot;
  nve::host::Key64 key;
  nve::host::Key64 schema_hash;
  uint8_t storage;  // 0 = Dense, 1 = Sparse
  uint8_t flags;    // table_flags::{HOST_SLOT = 1, PUBLISH_TO_SCENE = 2, AFFECTS_LAYOUT = 4}
  std::span<const uint32_t> defaults;
  std::span<const uint8_t> word_effects;
  uint32_t str_words;
  uint32_t resource_words;
  std::string_view name;
};

class Nve2Writer {
 public:
  Nve2Writer() { out_.resize(24u, 0u); }

  /// Appends one op; `body` writes its payload through `u32` / `padded`.
  template <class Body>
  Nve2Writer& op(Op op, uint8_t table, uint16_t count, Body&& body) {
    const size_t start = out_.size();
    push_u32(out_, static_cast<uint32_t>(op) | (uint32_t{table} << 8u) | (uint32_t{count} << 16u));
    push_u32(out_, 0u);
    std::forward<Body>(body)(*this);
    write_u32_at(out_, start + 4u, static_cast<uint32_t>((out_.size() - start) / 4u - 2u));
    ++ops_;
    return *this;
  }

  Nve2Writer& u32(uint32_t v) {
    push_u32(out_, v);
    return *this;
  }

  Nve2Writer& padded(std::string_view sv) {
    push_padded(out_, sv);
    return *this;
  }

  Nve2Writer& padded(std::span<const uint8_t> bytes) {
    push_padded(out_, bytes);
    return *this;
  }

  Nve2Writer& declare(const TableDecl& t) {
    return op(Op::DeclareTables, 0u, 1u, [&](Nve2Writer& w) {
      w.u32(t.key.lo()).u32(t.key.hi()).u32(t.schema_hash.lo()).u32(t.schema_hash.hi());
      w.u32(uint32_t{t.wire_slot} | (uint32_t{t.storage} << 8u) | (uint32_t{t.flags} << 16u) |
            (static_cast<uint32_t>(t.defaults.size()) << 24u));
      w.u32(t.str_words).u32(t.resource_words);
      for (const uint32_t d : t.defaults) w.u32(d);
      w.padded(t.word_effects);
      w.u32(static_cast<uint32_t>(t.name.size())).padded(t.name);
    });
  }

  /// Finishes the header (`magic, version, frame, flags 0, op count, total words`).
  [[nodiscard]] std::vector<uint8_t> finish(uint32_t frame = 1u) && {
    write_u32_at(out_, 0u, kNve2Magic);
    write_u32_at(out_, 4u, kNve2Version);
    write_u32_at(out_, 8u, frame);
    write_u32_at(out_, 12u, 0u);
    write_u32_at(out_, 16u, ops_);
    write_u32_at(out_, 20u, static_cast<uint32_t>(out_.size() / 4u));
    return std::move(out_);
  }

 private:
  std::vector<uint8_t> out_;
  uint32_t ops_ = 0u;
};

}  // namespace nve_test
