// NVE host runtime: `NVE2` frame writer for C++ apps, demos, and fixtures.
//
// Mirrors `nve_store::wire::WireEncoder` (packages/core/store/src/wire/encode.rs): one session per
// receiving store, tables declared once per session from generated `NveTableDesc`s, word-level
// row shadows so bit fields sharing a word patch correctly, and ops emitted in the canonical
// order `DeclareTables, CreateNodes, DefineResources, per table {Patch, SetStrings, Detach},
// SetChildren, RemoveNodes, ReleaseResources`. Every frame round-trips through the real runtime
// decoder in `tests/test_runtime_session.cpp`, which catches any opcode / layout drift.
#pragma once

#include <algorithm>
#include <array>
#include <bit>
#include <cstdint>
#include <map>
#include <set>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "../generated/nve_abi.generated.h"
#include "table_word.h"
#include "utf8.h"
#include "xxh64.hpp"

namespace nve::host {

/// `nve_store::Length` word encodings (`f32` bits with a 2-bit unit tag in the low mantissa bits).
[[nodiscard]] inline uint32_t length_points(float px) noexcept {
  return (std::bit_cast<uint32_t>(px) & ~uint32_t{NVE_LENGTH_TAG_MASK}) | NVE_LENGTH_POINTS;
}
[[nodiscard]] inline uint32_t length_percent(float pct) noexcept {
  return (std::bit_cast<uint32_t>(pct) & ~uint32_t{NVE_LENGTH_TAG_MASK}) | NVE_LENGTH_PERCENT;
}
inline constexpr uint32_t kLengthAuto = NVE_LENGTH_AUTO;

/// Packed RGBA8888 (`r | g<<8 | b<<16 | a<<24`), the `Color` field encoding.
[[nodiscard]] constexpr uint32_t rgba(uint8_t r, uint8_t g, uint8_t b, uint8_t a = 255u) noexcept {
  return uint32_t{r} | (uint32_t{g} << 8u) | (uint32_t{b} << 16u) | (uint32_t{a} << 24u);
}

class Nve2Writer {
 public:
  /// Forgets declarations and row shadows: the next frame starts a new receiving session.
  void reset_session() {
    declared_.clear();
    rows_.clear();
    clear_frame();
  }

  void create(uint32_t node) { creates_.push_back(node); }

  /// Removes `node` (descendants go with it on the receiver) and forgets its rows.
  void remove(uint32_t node) {
    removes_.push_back(node);
    for (auto it = rows_.begin(); it != rows_.end();) {
      it = static_cast<uint32_t>(it->first) == node ? rows_.erase(it) : std::next(it);
    }
    for (auto& [slot, nodes] : detaches_) nodes.erase(node);
  }

  void set_children(uint32_t parent, std::span<const uint32_t> kids) {
    children_[parent].assign(kids.begin(), kids.end());
  }

  /// Ensures `node` has a `t` row (attached with defaults when new).
  void attach(uint32_t node, const NveTableDesc& t) { (void)row(node, t); }

  /// Detaches `node`'s `t` row.
  void detach(uint32_t node, const NveTableDesc& t) {
    const uint8_t slot = declare(t);
    const auto it = rows_.find(row_key(slot, node));
    const bool fresh = it != rows_.end() && it->second.attached_now;
    if (it != rows_.end()) rows_.erase(it);
    if (!fresh) detaches_[slot].insert(node);
  }

  /// Writes `value` into field `f` (any non-`Str` kind; resources carry their `ResourceId`).
  /// Returns `false` for a spec that is invalid, a `Str`, or outside `t`'s row.
  bool set(uint32_t node, const NveTableDesc& t, NveFieldSpec f, uint32_t value) {
    if (f.kind == static_cast<uint8_t>(NVE_FIELD_STR) || nve_field_fits(f, t.words) == 0) return false;
    Row& r = row(node, t);
    const uint32_t mask = f.bits >= 32u ? ~uint32_t{0} : ((uint32_t{1} << f.bits) - 1u) << f.shift;
    r.words[f.word] = (r.words[f.word] & ~mask) | ((value << f.shift) & mask);
    r.mask |= uint32_t{1} << f.word;
    return true;
  }
  bool set_f32(uint32_t node, const NveTableDesc& t, NveFieldSpec f, float v) {
    return set(node, t, f, std::bit_cast<uint32_t>(v));
  }
  bool set_bool(uint32_t node, const NveTableDesc& t, NveFieldSpec f, bool v) {
    return set(node, t, f, v ? 1u : 0u);
  }

  /// Sets a `Str` field. Rejects non-`Str` specs and invalid UTF-8.
  bool set_str(uint32_t node, const NveTableDesc& t, NveFieldSpec f, std::string_view text) {
    if (f.kind != static_cast<uint8_t>(NVE_FIELD_STR) || nve_field_fits(f, t.words) == 0) return false;
    if (text.size() > 0x00FFFFFFu) return false;
    if (!text.empty() && nve_utf8_valid(reinterpret_cast<const uint8_t*>(text.data()), text.size()) == 0) {
      return false;
    }
    row(node, t).strs[f.word] = std::string(text);
    return true;
  }

  /// Defines resource `id` (non-zero) of `kind` with its codec bytes for this frame.
  void define_resource(uint32_t id, Key64 kind, std::span<const uint8_t> bytes, bool ephemeral = false) {
    resources_.push_back(Resource{id, kind, ephemeral, std::vector<uint8_t>(bytes.begin(), bytes.end())});
  }
  void release_resource(uint32_t id) { releases_.push_back(id); }

  [[nodiscard]] bool pending() const noexcept {
    if (!creates_.empty() || !resources_.empty() || !children_.empty() || !removes_.empty() ||
        !releases_.empty() || !new_decls_.empty()) {
      return true;
    }
    for (const auto& [slot, nodes] : detaches_) {
      if (!nodes.empty()) return true;
    }
    for (const auto& [key, r] : rows_) {
      if (r.attached_now || r.mask != 0u || !r.strs.empty()) return true;
    }
    return false;
  }

  /// Encodes every pending change as one `NVE2` frame (valid until the next call).
  std::span<const uint8_t> finish(uint64_t frame) {
    out_.clear();
    ops_ = 0u;
    for (int i = 0; i < 6; ++i) u32(0u);
    emit_declares();
    list(kCreateNodes, 0u, creates_);
    emit_resources();
    emit_rows();
    emit_children();
    list(kRemoveNodes, 0u, removes_);
    list(kReleaseResources, 0u, releases_);
    const uint32_t header[6] = {NVE_ABI_MAGIC, NVE_ABI_VERSION, static_cast<uint32_t>(frame),
                                static_cast<uint32_t>(frame >> 32u), ops_,
                                static_cast<uint32_t>(out_.size() / 4u)};
    for (size_t i = 0; i < 6u; ++i) put_at(i * 4u, header[i]);
    clear_frame();
    return out_;
  }

 private:
  // `nve_store::wire::Op` (packages/core/store/src/wire/mod.rs).
  static constexpr uint8_t kDeclareTables = 1u;
  static constexpr uint8_t kRemoveNodes = 2u;
  static constexpr uint8_t kCreateNodes = 3u;
  static constexpr uint8_t kDefineResources = 4u;
  static constexpr uint8_t kReleaseResources = 5u;
  static constexpr uint8_t kPatch = 6u;
  static constexpr uint8_t kDetach = 7u;
  static constexpr uint8_t kSetStrings = 8u;
  static constexpr uint8_t kSetChildren = 9u;

  struct Row {
    std::array<uint32_t, NVE_MAX_TABLE_WORDS> words{};
    uint32_t mask = 0u;
    uint32_t str_words = 0u;
    bool attached_now = false;
    std::map<uint8_t, std::string> strs;
  };
  struct Resource {
    uint32_t id;
    Key64 kind;
    bool ephemeral;
    std::vector<uint8_t> bytes;
  };

  static uint64_t row_key(uint8_t slot, uint32_t node) noexcept { return (uint64_t{slot} << 32u) | node; }

  uint8_t declare(const NveTableDesc& t) {
    for (size_t i = 0; i < declared_.size(); ++i) {
      if (declared_[i]->key == t.key) return static_cast<uint8_t>(i);
    }
    declared_.push_back(&t);
    new_decls_.push_back(static_cast<uint8_t>(declared_.size() - 1u));
    return static_cast<uint8_t>(declared_.size() - 1u);
  }

  Row& row(uint32_t node, const NveTableDesc& t) {
    const uint8_t slot = declare(t);
    auto [it, inserted] = rows_.try_emplace(row_key(slot, node));
    if (inserted) {
      for (uint32_t w = 0; w < t.words; ++w) it->second.words[w] = t.defaults[w];
      it->second.str_words = t.str_words;
      it->second.attached_now = true;
      // A row detached earlier this frame is re-created: resend every word.
      if (const auto d = detaches_.find(slot); d != detaches_.end() && d->second.erase(node) != 0u) {
        it->second.mask = t.words >= 32u ? ~uint32_t{0} : (uint32_t{1} << t.words) - 1u;
      }
    }
    return it->second;
  }

  void clear_frame() {
    new_decls_.clear();
    creates_.clear();
    resources_.clear();
    children_.clear();
    removes_.clear();
    releases_.clear();
    detaches_.clear();
    for (auto& [key, r] : rows_) {
      r.mask = 0u;
      r.attached_now = false;
      r.strs.clear();
    }
  }

  void u32(uint32_t v) {
    for (uint32_t s = 0; s < 32u; s += 8u) out_.push_back(static_cast<uint8_t>(v >> s));
  }
  void put_at(size_t off, uint32_t v) {
    for (uint32_t s = 0; s < 4u; ++s) out_[off + s] = static_cast<uint8_t>(v >> (s * 8u));
  }
  void bytes(std::span<const uint8_t> b) {
    out_.insert(out_.end(), b.begin(), b.end());
    while ((out_.size() & 3u) != 0u) out_.push_back(0u);
  }
  void begin(uint8_t op, uint8_t table, uint32_t count) {
    op_start_ = out_.size();
    u32(uint32_t{op} | (uint32_t{table} << 8u) | (std::min<uint32_t>(count, 0xFFFFu) << 16u));
    u32(0u);
    if (count >= 0xFFFFu) u32(count);
  }
  void end() {
    put_at(op_start_ + 4u, static_cast<uint32_t>((out_.size() - op_start_) / 4u - 2u));
    ++ops_;
  }
  void list(uint8_t op, uint8_t table, std::span<const uint32_t> items) {
    if (items.empty()) return;
    begin(op, table, static_cast<uint32_t>(items.size()));
    for (const uint32_t v : items) u32(v);
    end();
  }

  void emit_declares() {
    if (new_decls_.empty()) return;
    begin(kDeclareTables, 0u, static_cast<uint32_t>(new_decls_.size()));
    for (const uint8_t slot : new_decls_) {
      const NveTableDesc& t = *declared_[slot];
      u32(static_cast<uint32_t>(t.key));
      u32(static_cast<uint32_t>(t.key >> 32u));
      u32(static_cast<uint32_t>(t.schema_hash));
      u32(static_cast<uint32_t>(t.schema_hash >> 32u));
      u32(uint32_t{slot} | (uint32_t{t.storage} << 8u) | (uint32_t{t.flags} << 16u) | (uint32_t{t.words} << 24u));
      u32(t.str_words);
      u32(t.resource_words);
      for (uint32_t w = 0; w < t.words; ++w) u32(t.defaults[w]);
      bytes(std::span<const uint8_t>(t.word_effects, t.words));
      u32(t.name_len);
      bytes(std::span<const uint8_t>(reinterpret_cast<const uint8_t*>(t.name), t.name_len));
    }
    end();
  }

  void emit_resources() {
    if (resources_.empty()) return;
    begin(kDefineResources, 0u, static_cast<uint32_t>(resources_.size()));
    for (const Resource& r : resources_) {
      u32(r.id);
      u32(r.kind.lo());
      u32(r.kind.hi());
      u32(r.ephemeral ? 1u : 0u);
      u32(static_cast<uint32_t>(r.bytes.size()));
      bytes(r.bytes);
    }
    end();
  }

  void emit_rows() {
    for (uint8_t slot = 0; slot < declared_.size(); ++slot) {
      const auto lo = rows_.lower_bound(row_key(slot, 0u));
      const auto hi = rows_.lower_bound(row_key(static_cast<uint8_t>(slot + 1u), 0u));
      uint32_t patches = 0u;
      uint32_t strings = 0u;
      for (auto it = lo; it != hi; ++it) {
        const Row& r = it->second;
        const uint32_t m = r.mask & ~(r.str_words | (r.str_words << 1u));
        patches += (m != 0u || (r.attached_now && r.strs.empty())) ? 1u : 0u;
        strings += static_cast<uint32_t>(r.strs.size());
      }
      if (patches != 0u) {
        begin(kPatch, slot, patches);
        for (auto it = lo; it != hi; ++it) {
          const Row& r = it->second;
          const uint32_t m = r.mask & ~(r.str_words | (r.str_words << 1u));
          if (m == 0u && !(r.attached_now && r.strs.empty())) continue;
          u32(static_cast<uint32_t>(it->first));
          u32(m);
          for (uint32_t bits = m; bits != 0u; bits &= bits - 1u) u32(r.words[std::countr_zero(bits)]);
        }
        end();
      }
      if (strings != 0u) {
        begin(kSetStrings, slot, strings);
        for (auto it = lo; it != hi; ++it) {
          for (const auto& [word, text] : it->second.strs) {
            u32(static_cast<uint32_t>(it->first));
            u32(uint32_t{word} | (static_cast<uint32_t>(text.size()) << 8u));
            bytes(std::span<const uint8_t>(reinterpret_cast<const uint8_t*>(text.data()), text.size()));
          }
        }
        end();
      }
      if (const auto d = detaches_.find(slot); d != detaches_.end() && !d->second.empty()) {
        const std::vector<uint32_t> nodes(d->second.begin(), d->second.end());
        list(kDetach, slot, nodes);
      }
    }
  }

  void emit_children() {
    if (children_.empty()) return;
    begin(kSetChildren, 0u, static_cast<uint32_t>(children_.size()));
    for (const auto& [parent, kids] : children_) {
      u32(parent);
      u32(static_cast<uint32_t>(kids.size()));
      for (const uint32_t k : kids) u32(k);
    }
    end();
  }

  // Session state.
  std::vector<const NveTableDesc*> declared_;
  std::map<uint64_t, Row> rows_;
  // Frame state.
  std::vector<uint8_t> new_decls_;
  std::vector<uint32_t> creates_;
  std::vector<Resource> resources_;
  std::map<uint32_t, std::vector<uint32_t>> children_;
  std::vector<uint32_t> removes_;
  std::vector<uint32_t> releases_;
  std::map<uint8_t, std::set<uint32_t>> detaches_;
  std::vector<uint8_t> out_;
  size_t op_start_ = 0u;
  uint32_t ops_ = 0u;
};

}  // namespace nve::host
