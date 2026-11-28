#import "NveAppleValueBridge.h"

#import <CoreFoundation/CoreFoundation.h>

#include <nve/host/bin_codec.hpp>
#include <nve/host/owned_events.hpp>

#include <algorithm>
#include <cmath>
#include <cstring>

namespace nve::apple {

namespace {
constexpr uint32_t kMaxDepth = 32u;

NveValue null_value() noexcept {
  NveValue v{};
  v.tag = NVE_VAL_NULL;
  return v;
}

std::string_view sv(NveStrSlice s) noexcept {
  return s.ptr != nullptr ? std::string_view(reinterpret_cast<const char *>(s.ptr), s.len) : std::string_view();
}

id objc_object_from_bin_reader(nve::host::BinReader &r, uint32_t depth = 0u) {
  using nve::host::BinTag;
  if (depth > kMaxDepth) {
    r.skip_value();
    return [NSNull null];
  }
  const BinTag tag = r.peek_tag();
  if (!r.ok()) return [NSNull null];
  switch (tag) {
    case BinTag::Null:
      (void)r.read_null();
      return [NSNull null];
    case BinTag::Bool:
      return @(r.read_bool());
    case BinTag::I64:
      return @(r.read_i64());
    case BinTag::F64: {
      const double d = r.read_f64();
      return std::isfinite(d) ? (id)@(d) : (id)[NSNull null];
    }
    case BinTag::Str:
      return nsstring_from_sv(r.read_str());
    case BinTag::Bytes: {
      const std::span<const uint8_t> b = r.read_bytes();
      return !b.empty() ? [NSData dataWithBytes:b.data() length:b.size()] : [NSData data];
    }
    case BinTag::Array: {
      const auto mark = r.begin_array();
      NSMutableArray *a = [NSMutableArray arrayWithCapacity:mark.count];
      for (uint32_t i = 0u; i < mark.count && r.ok(); ++i) {
        [a addObject:objc_object_from_bin_reader(r, depth + 1u)];
      }
      r.end(mark);
      return a;
    }
    case BinTag::Map: {
      const auto mark = r.begin_map();
      NSMutableDictionary *m = [NSMutableDictionary dictionaryWithCapacity:mark.count];
      for (uint32_t i = 0u; i < mark.count && r.ok(); ++i) {
        const std::string_view key = r.read_map_key();
        if (!r.ok()) break;
        m[nsstring_from_sv(key)] = objc_object_from_bin_reader(r, depth + 1u);
      }
      r.end(mark);
      return m;
    }
  }
  return [NSNull null];
}
}  // namespace

NveStrSlice ValueArena::str(std::string_view s) {
  const std::string &owned = strings_.emplace_back(s);
  return NveStrSlice{reinterpret_cast<const uint8_t *>(owned.data()), owned.size()};
}

NveBufferSlice ValueArena::bytes(const void *data, size_t len) {
  if (data == nullptr || len == 0u) return NveBufferSlice{nullptr, 0u};
  const std::string &owned = strings_.emplace_back(static_cast<const char *>(data), len);
  return NveBufferSlice{reinterpret_cast<const uint8_t *>(owned.data()), owned.size()};
}

NSString *nsstring_from_sv(std::string_view s) {
  if (s.empty()) return @"";
  NSString *str = [[NSString alloc] initWithBytes:s.data() length:s.size() encoding:NSUTF8StringEncoding];
  return str ?: @"";
}

NSString *nsstring_or_nil_from_sv(std::string_view s) {
  return s.empty() ? nil : nsstring_from_sv(s);
}

NveValue nve_value_from_objc_object(id obj, ValueArena &arena, uint32_t depth) {
  NveValue v = null_value();
  if (obj == nil || obj == [NSNull null] || depth > kMaxDepth) return v;
  if ([obj isKindOfClass:[NSString class]]) {
    v.tag = NVE_VAL_STR;
    v.data.str_val = arena.str(((NSString *)obj).UTF8String ?: "");
  } else if ([obj isKindOfClass:[NSNumber class]]) {
    NSNumber *n = obj;
    if (CFGetTypeID((__bridge CFTypeRef)n) == CFBooleanGetTypeID()) {
      v.tag = NVE_VAL_BOOL;
      v.data.bool_val = n.boolValue ? 1u : 0u;
    } else if (CFNumberIsFloatType((__bridge CFNumberRef)n)) {
      v.tag = NVE_VAL_F64;
      v.data.f64_val = n.doubleValue;
    } else {
      v.tag = NVE_VAL_I64;
      v.data.i64_val = n.longLongValue;
    }
  } else if ([obj isKindOfClass:[NSData class]]) {
    NSData *d = obj;
    v.tag = NVE_VAL_BYTES;
    v.data.bytes_val = arena.bytes(d.bytes, d.length);
  } else if ([obj isKindOfClass:[NSArray class]]) {
    NSArray *a = obj;
    NveValue *items = arena.values(a.count);
    for (NSUInteger i = 0; i < a.count; ++i) items[i] = nve_value_from_objc_object(a[i], arena, depth + 1u);
    v.tag = NVE_VAL_ARRAY;
    v.data.array_val = NveValueArray{items, a.count};
  } else if ([obj isKindOfClass:[NSDictionary class]]) {
    const std::vector<NveMapEntry> entries = map_entries_from_dictionary(obj, arena, depth);
    NveMapEntry *stored = arena.entries(entries.size());
    std::copy(entries.begin(), entries.end(), stored);
    v.tag = NVE_VAL_MAP;
    v.data.map_val = NveValueMap{stored, entries.size()};
  }
  return v;
}

std::vector<NveMapEntry> map_entries_from_dictionary(NSDictionary<NSString *, id> *props, ValueArena &arena, uint32_t depth) {
  std::vector<NveMapEntry> out;
  if (depth > kMaxDepth) return out;
  // Filter before sorting: `compare:` on a mixed-key dictionary (NSNumber vs NSString) throws.
  NSMutableArray<NSString *> *keys = [NSMutableArray arrayWithCapacity:props.count];
  for (id key in props) {
    if ([key isKindOfClass:[NSString class]]) [keys addObject:key];
  }
  [keys sortUsingSelector:@selector(compare:)];
  out.reserve(keys.count);
  for (NSString *key in keys) {
    NveMapEntry e{};
    e.key = arena.str(key.UTF8String ?: "");
    e.value = nve_value_from_objc_object(props[key], arena, depth + 1u);
    out.push_back(e);
  }
  return out;
}

id objc_object_from_nve_value(const NveValue &v, uint32_t depth) {
  if (depth > kMaxDepth) return [NSNull null];
  switch (v.tag) {
    case NVE_VAL_BOOL: return @(v.data.bool_val != 0u);
    case NVE_VAL_I64: return @(v.data.i64_val);
    case NVE_VAL_F64: return std::isfinite(v.data.f64_val) ? (id)@(v.data.f64_val) : (id)[NSNull null];
    case NVE_VAL_STR: return nsstring_from_sv(sv(v.data.str_val));
    case NVE_VAL_BYTES:
      return v.data.bytes_val.ptr != nullptr ? [NSData dataWithBytes:v.data.bytes_val.ptr length:v.data.bytes_val.len]
                                             : [NSData data];
    case NVE_VAL_ARRAY: {
      NSMutableArray *a = [NSMutableArray arrayWithCapacity:v.data.array_val.len];
      for (size_t i = 0; i < v.data.array_val.len && v.data.array_val.ptr != nullptr; ++i) {
        [a addObject:objc_object_from_nve_value(v.data.array_val.ptr[i], depth + 1u)];
      }
      return a;
    }
    case NVE_VAL_MAP: {
      NSMutableDictionary *m = [NSMutableDictionary dictionaryWithCapacity:v.data.map_val.len];
      for (size_t i = 0; i < v.data.map_val.len && v.data.map_val.ptr != nullptr; ++i) {
        const NveMapEntry &e = v.data.map_val.ptr[i];
        m[nsstring_from_sv(sv(e.key))] = objc_object_from_nve_value(e.value, depth + 1u);
      }
      return m;
    }
    default: return [NSNull null];
  }
}

NSString *event_name(const NveEvent &ev, NveEventBatch batch) {
  if (nve::host::is_custom_event(ev)) {
    const std::string_view custom = nve::host::event_custom_name(ev, batch);
    return !custom.empty() ? nsstring_from_sv(custom) : @"custom";
  }
  if (ev.table == NVE_TABLE_INTERACTION) {
    switch (ev.event) {
      case NVE_INTERACTION_EVENT_PRESS: return @"press";
      case NVE_INTERACTION_EVENT_LONG_PRESS: return @"longPress";
      case NVE_INTERACTION_EVENT_FOCUS: return @"focus";
      case NVE_INTERACTION_EVENT_BLUR: return @"blur";
      case NVE_INTERACTION_EVENT_KEY_DOWN: return @"keyDown";
      case NVE_INTERACTION_EVENT_KEY_UP: return @"keyUp";
      case NVE_INTERACTION_EVENT_POINTER_DOWN: return @"pointerDown";
      case NVE_INTERACTION_EVENT_POINTER_MOVE: return @"pointerMove";
      case NVE_INTERACTION_EVENT_POINTER_UP: return @"pointerUp";
      case NVE_INTERACTION_EVENT_POINTER_ENTER: return @"pointerEnter";
      case NVE_INTERACTION_EVENT_POINTER_LEAVE: return @"pointerLeave";
      case NVE_INTERACTION_EVENT_WHEEL: return @"wheel";
      default: break;
    }
  } else if (ev.table == NVE_TABLE_TEXT_INPUT) {
    switch (ev.event) {
      case NVE_TEXT_INPUT_EVENT_CHANGE: return @"textChange";
      case NVE_TEXT_INPUT_EVENT_SUBMIT: return @"submit";
      case NVE_TEXT_INPUT_EVENT_COMPOSITION_START: return @"compositionStart";
      case NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE: return @"compositionUpdate";
      case NVE_TEXT_INPUT_EVENT_COMPOSITION_END: return @"compositionEnd";
      default: break;
    }
  } else if (ev.table == NVE_TABLE_SCROLL) {
    if (ev.event == NVE_SCROLL_EVENT_SCROLL) return @"scroll";
  } else if (ev.table == NVE_TABLE_LAYOUT) {
    if (ev.event == NVE_LAYOUT_EVENT_LAYOUT) return @"layout";
  }
  return @"custom";
}

NSDictionary<NSString *, id> *payload_dictionary_from_event(const NveEvent &ev, NveEventBatch batch) {
  NSMutableDictionary *p = [NSMutableDictionary dictionary];
  p[@"nodeId"] = @(ev.node);
  p[@"surfaceId"] = @(ev.surface);
  p[@"timestampUs"] = @(ev.timestamp);
  if (nve::host::is_custom_event(ev)) {
    const std::span<const uint8_t> data = nve::host::event_custom_data(ev, batch);
    if (!data.empty()) {
      if (nve::host::BinReader::validate(data) == nve::host::BinError::None) {
        nve::host::BinReader reader(data);
        if (reader.peek_tag() != nve::host::BinTag::Null) {
          p[@"value"] = objc_object_from_bin_reader(reader);
        }
      } else {
        p[@"value"] = [NSData dataWithBytes:data.data() length:data.size()];
      }
    }
    return p;
  }
  if (ev.table == NVE_TABLE_INTERACTION) {
    switch (ev.event) {
      case NVE_INTERACTION_EVENT_PRESS:
      case NVE_INTERACTION_EVENT_LONG_PRESS:
        p[@"x"] = @(nve::host::event_payload_f32(ev, 0u));
        p[@"y"] = @(nve::host::event_payload_f32(ev, 1u));
        break;
      case NVE_INTERACTION_EVENT_POINTER_DOWN:
      case NVE_INTERACTION_EVENT_POINTER_MOVE:
      case NVE_INTERACTION_EVENT_POINTER_UP:
      case NVE_INTERACTION_EVENT_POINTER_ENTER:
      case NVE_INTERACTION_EVENT_POINTER_LEAVE:
        p[@"x"] = @(nve::host::event_payload_f32(ev, 0u));
        p[@"y"] = @(nve::host::event_payload_f32(ev, 1u));
        p[@"pointerId"] = @(ev.payload[2]);
        break;
      case NVE_INTERACTION_EVENT_WHEEL:
        p[@"deltaX"] = @(nve::host::event_payload_f32(ev, 0u));
        p[@"deltaY"] = @(nve::host::event_payload_f32(ev, 1u));
        break;
      case NVE_INTERACTION_EVENT_KEY_DOWN:
      case NVE_INTERACTION_EVENT_KEY_UP: {
        p[@"keyCode"] = @(ev.payload[0]);
        const std::string_view key = nve::host::event_key(ev, batch);
        if (!key.empty()) p[@"key"] = nsstring_from_sv(key);
        const std::string_view code = nve::host::event_code(ev, batch);
        if (!code.empty()) p[@"code"] = nsstring_from_sv(code);
        break;
      }
      default:
        break;
    }
  } else if (ev.table == NVE_TABLE_TEXT_INPUT) {
    p[@"text"] = nsstring_from_sv(nve::host::event_text(ev, batch));
  } else if (ev.table == NVE_TABLE_SCROLL || ev.table == NVE_TABLE_LAYOUT) {
    p[@"x"] = @(nve::host::event_payload_f32(ev, 0u));
    p[@"y"] = @(nve::host::event_payload_f32(ev, 1u));
    p[@"width"] = @(nve::host::event_payload_f32(ev, 2u));
    p[@"height"] = @(nve::host::event_payload_f32(ev, 3u));
  }
  return p;
}

}  // namespace nve::apple