#pragma once

#import <Foundation/Foundation.h>

#include <nve/generated/nve_abi.generated.h>
#include <nve/generated/nve_runtime_abi.generated.h>

#include <cstdint>
#include <deque>
#include <string>
#include <string_view>
#include <vector>

namespace nve::apple {

/// Owns the backing storage of `NveValue` graphs built from Foundation objects (strings, bytes,
/// arrays, map entries) until `reset()`; pointers stay stable while it lives.
class ValueArena {
 public:
  void reset() noexcept {
    strings_.clear();
    values_.clear();
    entries_.clear();
  }
  NveStrSlice str(std::string_view s);
  NveBufferSlice bytes(const void *_Nullable data, size_t len);
  NveValue *_Nonnull values(size_t n) { return values_.emplace_back(n).data(); }
  NveMapEntry *_Nonnull entries(size_t n) { return entries_.emplace_back(n).data(); }

 private:
  std::deque<std::string> strings_;
  std::deque<std::vector<NveValue>> values_;
  std::deque<std::vector<NveMapEntry>> entries_;
};

/// UTF-8 → non-null `NSString` (`@""` when empty or invalid).
NSString *_Nonnull nsstring_from_sv(std::string_view sv);

/// UTF-8 → `NSString`, `nil` when empty.
NSString *_Nullable nsstring_or_nil_from_sv(std::string_view sv);

/// Recursively converts `NSDictionary` / `NSArray` / `NSString` / `NSNumber` / `NSData` / `NSNull`
/// into an arena-backed `NveValue`. `depth` counts container nesting (one per array / dictionary
/// level); a value deeper than the limit, or an unknown object, becomes null.
NveValue nve_value_from_objc_object(id _Nullable obj, ValueArena &arena, uint32_t depth = 0u);

/// Recursively converts a borrowed `NveValue` into Foundation objects (`NSNull` for null).
id _Nonnull objc_object_from_nve_value(const NveValue &value, uint32_t depth = 0u);

/// Builds `NveMapEntry` props from a dictionary at nesting `depth`: only `NSString` keys (others
/// are skipped, never compared), sorted for deterministic order; empty beyond the depth limit.
std::vector<NveMapEntry> map_entries_from_dictionary(NSDictionary<NSString *, id> *_Nonnull props, ValueArena &arena,
                                                     uint32_t depth = 0u);

/// Event name (`"press"`, `"textChange"`, custom name, …) and payload dictionary of an outbound
/// app event.
NSString *_Nonnull event_name(const NveDirectEvent &ev);
NSDictionary<NSString *, id> *_Nonnull payload_dictionary_from_event(const NveDirectEvent &ev);

/// Inbound host events; string payloads borrow `arena`.
NveDirectEvent make_point_event(uint32_t surface, uint32_t node, uint8_t kind, float x, float y);
NveDirectEvent make_text_event(uint32_t surface, uint32_t node, uint8_t kind, std::string_view text, ValueArena &arena);
NveDirectEvent make_scroll_event(uint32_t surface, uint32_t node, float x, float y, float content_w, float content_h);

}  // namespace nve::apple