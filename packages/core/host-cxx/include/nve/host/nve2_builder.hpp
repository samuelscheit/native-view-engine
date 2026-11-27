// NVE host runtime: header-only C++20 builder of binary `NVE2` frames (`nve_store::wire`).
//
// Lets C++ / Objective-C++ hosts and their tests produce real scenes for `submit_nve2` without a
// Rust app: declare the generated core tables (`declare_core_tables`), create nodes, patch word
// rows, set strings, define resources, and parent children. Every op is length-prefixed exactly
// like `WireEncoder`, so the runtime decodes and validates it the same way.
#pragma once

#include <cstddef>
#include <cstdint>
#include <initializer_list>
#include <span>
#include <string_view>
#include <vector>

#include "../generated/nve_tables.generated.h"
#include "xxh64.hpp"

namespace nve::host {

/// `DeclareTables` entry: one table's identity and row layout.
struct Nve2TableDecl {
  Key64 key;
  Key64 schema_hash;
  uint8_t wire_slot = 0u;
  bool sparse = false;
  uint8_t flags = 0u;
  std::span<const uint32_t> defaults;
  std::span<const uint8_t> word_effects;
  uint32_t str_words = 0u;
  uint32_t resource_words = 0u;
  std::string_view name;
};

class Nve2FrameBuilder {
 public:
  static constexpr uint32_t kMagic = 0x3245564Eu;  // "NVE2"
  static constexpr uint32_t kVersion = 1u;

  /// `frame == 1` marks a keyframe (full reset); later frames are deltas.
  explicit Nve2FrameBuilder(uint64_t frame = 1u) : frame_(frame) { out_.resize(6u * 4u); }

  /// Declares every generated core table at wire slot == its index in `NVE_TABLE_LIST`.
  Nve2FrameBuilder& declare_core_tables() {
    uint8_t slot = 0u;
#define NVE_HOST_DECLARE_CORE_TABLE(IDENT, NAME)                                                  \
  declare({Key64{NVE_TABLE_##IDENT##_KEY}, Key64{NVE_TABLE_##IDENT##_SCHEMA_HASH}, slot++,       \
           NVE_TABLE_##IDENT##_DENSE == 0, static_cast<uint8_t>(NVE_TABLE_##IDENT##_FLAGS),       \
           NVE_TABLE_##IDENT##_DEFAULTS, NVE_TABLE_##IDENT##_WORD_EFFECTS,                        \
           NVE_TABLE_##IDENT##_STR_WORDS, NVE_TABLE_##IDENT##_RESOURCE_WORDS, NAME});
    NVE_TABLE_LIST(NVE_HOST_DECLARE_CORE_TABLE)
#undef NVE_HOST_DECLARE_CORE_TABLE
    return *this;
  }

  /// Wire slot `declare_core_tables` assigns to the core table `key` (`0xFF` when unknown).
  [[nodiscard]] static constexpr uint8_t core_slot(Key64 key) noexcept {
    uint8_t slot = 0u;
    uint8_t found = 0xFFu;
#define NVE_HOST_FIND_CORE_TABLE(IDENT, NAME) \
  if (Key64{NVE_TABLE_##IDENT##_KEY} == key) found = slot; \
  ++slot;
    NVE_TABLE_LIST(NVE_HOST_FIND_CORE_TABLE)
#undef NVE_HOST_FIND_CORE_TABLE
    return found;
  }

  Nve2FrameBuilder& declare(const Nve2TableDecl& t) {
    const size_t op = begin(1u, 0u, 1u);
    push(t.key.lo());
    push(t.key.hi());
    push(t.schema_hash.lo());
    push(t.schema_hash.hi());
    const auto words = static_cast<uint32_t>(t.defaults.size());
    push(uint32_t{t.wire_slot} | (uint32_t{t.sparse ? 1u : 0u} << 8u) | (uint32_t{t.flags} << 16u) | (words << 24u));
    push(t.str_words);
    push(t.resource_words);
    for (const uint32_t w : t.defaults) push(w);
    push_bytes(t.word_effects);
    push(static_cast<uint32_t>(t.name.size()));
    push_bytes(std::span(reinterpret_cast<const uint8_t*>(t.name.data()), t.name.size()));
    end(op);
    return *this;
  }

  /// Creates nodes at raw generational ids (`index | generation << 24`).
  Nve2FrameBuilder& create_nodes(std::initializer_list<uint32_t> ids) { return node_list(3u, 0u, ids); }

  /// Removes nodes (and their subtrees).
  Nve2FrameBuilder& remove_nodes(std::initializer_list<uint32_t> ids) { return node_list(2u, 0u, ids); }

  /// Detaches the table at `wire_slot` from `nodes`.
  Nve2FrameBuilder& detach(uint8_t wire_slot, std::initializer_list<uint32_t> nodes) {
    return node_list(7u, wire_slot, nodes);
  }

  /// Writes the words of `row` selected by `mask` (bit `w` = word `w`) into `node`'s row of the
  /// table at `wire_slot`, attaching the table when absent. String words go through `set_string`.
  Nve2FrameBuilder& patch(uint8_t wire_slot, uint32_t node, uint32_t mask, std::span<const uint32_t> row) {
    const size_t op = begin(6u, wire_slot, 1u);
    push(node);
    push(mask);
    for (uint32_t w = 0u; w < row.size() && w < 32u; ++w) {
      if ((mask >> w) & 1u) push(row[w]);
    }
    end(op);
    return *this;
  }

  Nve2FrameBuilder& set_string(uint8_t wire_slot, uint32_t node, uint8_t word, std::string_view text) {
    const size_t op = begin(8u, wire_slot, 1u);
    push(node);
    push(uint32_t{word} | (static_cast<uint32_t>(text.size()) << 8u));
    push_bytes(std::span(reinterpret_cast<const uint8_t*>(text.data()), text.size()));
    end(op);
    return *this;
  }

  /// Defines resource `id` of `kind` with its codec encoding (e.g. `nve.res.image_src`).
  Nve2FrameBuilder& define_resource(uint32_t id, Key64 kind, std::span<const uint8_t> encoded, bool ephemeral = false) {
    const size_t op = begin(4u, 0u, 1u);
    push(id);
    push(kind.lo());
    push(kind.hi());
    push(ephemeral ? 1u : 0u);
    push(static_cast<uint32_t>(encoded.size()));
    push_bytes(encoded);
    end(op);
    return *this;
  }

  Nve2FrameBuilder& set_children(uint32_t parent, std::initializer_list<uint32_t> kids) {
    const size_t op = begin(9u, 0u, 1u);
    push(parent);
    push(static_cast<uint32_t>(kids.size()));
    for (const uint32_t k : kids) push(k);
    end(op);
    return *this;
  }

  [[nodiscard]] std::vector<uint8_t> finish() {
    write_at(0u, kMagic);
    write_at(4u, kVersion);
    write_at(8u, static_cast<uint32_t>(frame_));
    write_at(12u, static_cast<uint32_t>(frame_ >> 32u));
    write_at(16u, ops_);
    write_at(20u, static_cast<uint32_t>(out_.size() / 4u));
    return out_;
  }

 private:
  Nve2FrameBuilder& node_list(uint8_t op_code, uint8_t table, std::initializer_list<uint32_t> ids) {
    const size_t op = begin(op_code, table, static_cast<uint16_t>(ids.size()));
    for (const uint32_t id : ids) push(id);
    end(op);
    return *this;
  }

  size_t begin(uint8_t op, uint8_t table, uint16_t count) {
    const size_t start = out_.size();
    push(uint32_t{op} | (uint32_t{table} << 8u) | (uint32_t{count} << 16u));
    push(0u);
    return start;
  }

  void end(size_t start) {
    write_at(start + 4u, static_cast<uint32_t>((out_.size() - start) / 4u - 2u));
    ++ops_;
  }

  void push(uint32_t v) {
    for (uint32_t s = 0u; s < 32u; s += 8u) out_.push_back(static_cast<uint8_t>((v >> s) & 0xFFu));
  }

  void push_bytes(std::span<const uint8_t> bytes) {
    out_.insert(out_.end(), bytes.begin(), bytes.end());
    while ((out_.size() & 3u) != 0u) out_.push_back(0u);
  }

  void write_at(size_t off, uint32_t v) {
    for (uint32_t i = 0u; i < 4u; ++i) out_[off + i] = static_cast<uint8_t>((v >> (8u * i)) & 0xFFu);
  }

  uint64_t frame_;
  uint32_t ops_ = 0u;
  std::vector<uint8_t> out_;
};

/// Sets field `f` of `row` (non-string) and returns its word bit for `Nve2FrameBuilder::patch`.
inline uint32_t set_field(std::span<uint32_t> row, NveFieldSpec f, uint32_t value) noexcept {
  const uint32_t mask = f.bits >= 32u ? 0xFFFFFFFFu : ((uint32_t{1} << f.bits) - 1u) << f.shift;
  row[f.word] = (row[f.word] & ~mask) | ((value << f.shift) & mask);
  return uint32_t{1} << f.word;
}

}  // namespace nve::host
