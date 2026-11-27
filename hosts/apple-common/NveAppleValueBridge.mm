#import "NveAppleValueBridge.h"

#import <CoreFoundation/CoreFoundation.h>

#include <algorithm>
#include <cmath>
#include <cstring>

namespace nve::apple {

namespace {
constexpr uint32_t kMaxDepth = 32u;

uint64_t now_us() noexcept {
  return static_cast<uint64_t>([[NSDate date] timeIntervalSince1970] * 1'000'000.0);
}

NveValue null_value() noexcept {
  NveValue v{};
  v.tag = NVE_VAL_NULL;
  return v;
}

std::string_view sv(NveStrSlice s) noexcept {
  return s.ptr != nullptr ? std::string_view(reinterpret_cast<const char *>(s.ptr), s.len) : std::string_view();
}
}  // namespace

NveStrSlice ValueArena::str(std::string_view s) {
  const std::string &owned = strings_.emplace_back(s);
  return NveStrSlice{reinterpret_cast<const uint8_t *>(owned.data()), owned.size()};
}

NveBufferSlice ValueArena::bytes(const void *data, size_t len) {
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
    const std::vector<NveMapEntry> entries = map_entries_from_dictionary(obj, arena);
    NveMapEntry *stored = arena.entries(entries.size());
    std::copy(entries.begin(), entries.end(), stored);
    v.tag = NVE_VAL_MAP;
    v.data.map_val = NveValueMap{stored, entries.size()};
  }
  return v;
}

std::vector<NveMapEntry> map_entries_from_dictionary(NSDictionary<NSString *, id> *props, ValueArena &arena) {
  NSArray<NSString *> *keys = [props.allKeys sortedArrayUsingSelector:@selector(compare:)];
  std::vector<NveMapEntry> out;
  out.reserve(keys.count);
  for (NSString *key in keys) {
    if (![key isKindOfClass:[NSString class]]) continue;
    NveMapEntry e{};
    e.key = arena.str(key.UTF8String ?: "");
    e.value = nve_value_from_objc_object(props[key], arena, 1u);
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

NSString *event_name(const NveDirectEvent &ev) {
  if (ev.name.ptr != nullptr && ev.name.len > 0u) return nsstring_from_sv(sv(ev.name));
  switch (ev.kind) {
    case NVE_EVENT_PRESS: return @"press";
    case NVE_EVENT_LONG_PRESS: return @"longPress";
    case NVE_EVENT_SCROLL: return @"scroll";
    case NVE_EVENT_TEXT_CHANGE: return @"textChange";
    case NVE_EVENT_FOCUS: return @"focus";
    case NVE_EVENT_BLUR: return @"blur";
    case NVE_EVENT_LAYOUT: return @"layout";
    case NVE_EVENT_SUBMIT: return @"submit";
    case NVE_EVENT_KEY_DOWN: return @"keyDown";
    case NVE_EVENT_KEY_UP: return @"keyUp";
    default: return @"custom";
  }
}

NSDictionary<NSString *, id> *payload_dictionary_from_event(const NveDirectEvent &ev) {
  NSMutableDictionary *p = [NSMutableDictionary dictionary];
  p[@"nodeId"] = @(ev.node_id);
  p[@"surfaceId"] = @(ev.surface_id);
  p[@"timestampUs"] = @(ev.timestamp_us);
  switch (ev.payload_tag) {
    case NVE_EVENT_PAYLOAD_POINT:
      p[@"x"] = @(ev.x);
      p[@"y"] = @(ev.y);
      break;
    case NVE_EVENT_PAYLOAD_SCROLL:
    case NVE_EVENT_PAYLOAD_LAYOUT:
      p[@"x"] = @(ev.x);
      p[@"y"] = @(ev.y);
      p[@"width"] = @(ev.width);
      p[@"height"] = @(ev.height);
      break;
    case NVE_EVENT_PAYLOAD_TEXT:
      p[@"text"] = nsstring_from_sv(sv(ev.text));
      break;
    default:
      break;
  }
  if (ev.value.tag != NVE_VAL_NULL) p[@"value"] = objc_object_from_nve_value(ev.value);
  return p;
}

NveDirectEvent make_point_event(uint32_t surface, uint32_t node, uint8_t kind, float x, float y) {
  NveDirectEvent ev{};
  ev.surface_id = surface;
  ev.node_id = node;
  ev.timestamp_us = now_us();
  ev.kind = kind;
  ev.payload_tag = NVE_EVENT_PAYLOAD_POINT;
  ev.x = x;
  ev.y = y;
  ev.value = null_value();
  return ev;
}

NveDirectEvent make_text_event(uint32_t surface, uint32_t node, uint8_t kind, std::string_view text, ValueArena &arena) {
  NveDirectEvent ev = make_point_event(surface, node, kind, 0.0f, 0.0f);
  ev.payload_tag = NVE_EVENT_PAYLOAD_TEXT;
  ev.text = arena.str(text);
  return ev;
}

NveDirectEvent make_scroll_event(uint32_t surface, uint32_t node, float x, float y, float content_w, float content_h) {
  NveDirectEvent ev = make_point_event(surface, node, NVE_EVENT_SCROLL, x, y);
  ev.payload_tag = NVE_EVENT_PAYLOAD_SCROLL;
  ev.width = content_w;
  ev.height = content_h;
  return ev;
}

}  // namespace nve::apple