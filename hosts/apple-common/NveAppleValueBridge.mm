#import "NveAppleValueBridge.h"

#import <CoreFoundation/CoreFoundation.h>

#include <cmath>

namespace nve::apple {

NSString * _Nonnull nsstring_from_sv(std::string_view sv) {
  if (sv.empty()) {
    return @"";
  }
  NSString *str = [[NSString alloc] initWithBytes:sv.data()
                                           length:sv.size()
                                         encoding:NSUTF8StringEncoding];
  return str ?: @"";
}

NSString * _Nullable nsstring_or_nil_from_sv(std::string_view sv) {
  if (sv.empty()) {
    return nil;
  }
  return [[NSString alloc] initWithBytes:sv.data()
                                  length:sv.size()
                                encoding:NSUTF8StringEncoding];
}

NveValue nve_value_from_objc_object(id _Nullable obj, nve::BumpArena &arena, uint32_t depth) {
  if (obj == nil || obj == [NSNull null] || depth > NVE_BIN_VALUE_MAX_DEPTH) {
    return nve::make_null_value();
  }
  if ([obj isKindOfClass:[NSNumber class]]) {
    NSNumber *num = (NSNumber *)obj;
    if (CFGetTypeID((__bridge CFTypeRef)num) == CFBooleanGetTypeID()) {
      return nve::make_bool_value(num.boolValue ? true : false);
    }
    if (CFNumberIsFloatType((__bridge CFNumberRef)num)) {
      const double d = num.doubleValue;
      return std::isfinite(d) ? nve::make_f64_value(d) : nve::make_null_value();
    }
    return nve::make_i64_value(static_cast<int64_t>(num.longLongValue));
  }
  if ([obj isKindOfClass:[NSString class]]) {
    NSString *str = (NSString *)obj;
    const char *utf8 = str.UTF8String;
    if (!utf8) {
      return nve::make_str_value(NveStrSlice{nullptr, 0});
    }
    return nve::make_str_value(arena.copy_str(std::string_view(utf8)));
  }
  if ([obj isKindOfClass:[NSData class]]) {
    NSData *data = (NSData *)obj;
    return nve::make_bytes_value(
        arena.copy_bytes(static_cast<const uint8_t *>(data.bytes), data.length));
  }
  if ([obj isKindOfClass:[NSArray class]]) {
    NSArray *arr = (NSArray *)obj;
    const size_t n = arr.count;
    NveValue *elems = n > 0 ? arena.alloc_array<NveValue>(n) : nullptr;
    for (size_t i = 0; i < n; ++i) {
      elems[i] = nve_value_from_objc_object(arr[i], arena, depth + 1u);
    }
    return nve::make_array_value(elems, n);
  }
  if ([obj isKindOfClass:[NSDictionary class]]) {
    NSDictionary *dict = (NSDictionary *)obj;
    const size_t n = dict.count;
    NveMapEntry *entries = n > 0 ? arena.alloc_array<NveMapEntry>(n) : nullptr;
    __block size_t idx = 0;
    [dict enumerateKeysAndObjectsUsingBlock:^(id key, id val, BOOL *stop) {
      (void)stop;
      if (idx >= n) return;
      NSString *keyStr = [key isKindOfClass:[NSString class]] ? (NSString *)key : [key description];
      const char *kutf8 = keyStr.UTF8String;
      entries[idx++] = NveMapEntry{
          kutf8 ? arena.copy_str(std::string_view(kutf8)) : NveStrSlice{nullptr, 0},
          nve_value_from_objc_object(val, arena, depth + 1u),
      };
    }];
    return nve::make_map_value(entries, idx);
  }
  return nve::make_null_value();
}

id _Nonnull objc_object_from_bin_value(const nve::BinValueView &view, uint32_t depth) {
  if (depth > NVE_BIN_VALUE_MAX_DEPTH) {
    return [NSNull null];
  }
  switch (view.tag()) {
    case NVE_VAL_NULL:
      return [NSNull null];
    case NVE_VAL_BOOL:
      return [NSNumber numberWithBool:view.as_bool().value_or(false) ? YES : NO];
    case NVE_VAL_I64:
      return @(view.as_i64().value_or(0));
    case NVE_VAL_F64:
      return @(view.as_f64().value_or(0.0));
    case NVE_VAL_STR:
      return nsstring_from_sv(view.as_str().value_or(std::string_view{}));
    case NVE_VAL_BYTES: {
      const NveBufferSlice b = view.as_bytes().value_or(NveBufferSlice{nullptr, 0});
      return (b.ptr != nullptr && b.len > 0) ? [NSData dataWithBytes:b.ptr length:b.len]
                                             : [NSData data];
    }
    case NVE_VAL_ARRAY: {
      const size_t n = view.array_len().value_or(0);
      NSMutableArray *arr = [NSMutableArray arrayWithCapacity:n];
      (void)view.for_each_array([&](const nve::BinValueView &elem) {
        [arr addObject:objc_object_from_bin_value(elem, depth + 1u)];
      });
      return [arr copy];
    }
    case NVE_VAL_MAP: {
      const size_t n = view.map_len().value_or(0);
      NSMutableDictionary<NSString *, id> *map = [NSMutableDictionary dictionaryWithCapacity:n];
      (void)view.for_each_map([&](std::string_view k, const nve::BinValueView &val) {
        NSString *key = nsstring_or_nil_from_sv(k);
        if (key != nil) {
          map[key] = objc_object_from_bin_value(val, depth + 1u);
        }
      });
      return [map copy];
    }
  }
  return [NSNull null];
}

NSDictionary<NSString *, id> *payload_dictionary_from_event(const nve::DecodedEvent &ev) {
  NSMutableDictionary<NSString *, id> *dict = [NSMutableDictionary dictionary];
  dict[@"surfaceId"] = @(ev.surface_id);
  dict[@"nodeId"] = @(ev.node_id);
  dict[@"timestampUs"] = @(ev.timestamp_us);

  switch (ev.kind) {
    case nve::EventKind::Press:
    case nve::EventKind::LongPress:
      dict[@"x"] = @(ev.x);
      dict[@"y"] = @(ev.y);
      break;
    case nve::EventKind::Scroll:
      dict[@"x"] = @(ev.x);
      dict[@"y"] = @(ev.y);
      dict[@"contentWidth"] = @(ev.width);
      dict[@"contentHeight"] = @(ev.height);
      break;
    case nve::EventKind::TextChange:
    case nve::EventKind::CompositionStart:
    case nve::EventKind::CompositionUpdate:
    case nve::EventKind::CompositionEnd:
      dict[@"text"] = nsstring_from_sv(ev.text);
      break;
    case nve::EventKind::Layout:
      dict[@"x"] = @(ev.x);
      dict[@"y"] = @(ev.y);
      dict[@"width"] = @(ev.width);
      dict[@"height"] = @(ev.height);
      break;
    case nve::EventKind::Custom: {
      if (!ev.payload_bytes.empty()) {
        const auto parsed =
            nve::BinValueView::parse(ev.payload_bytes.data(), ev.payload_bytes.size());
        if (parsed.has_value()) {
          if (parsed->tag() == NVE_VAL_MAP) {
            (void)parsed->for_each_map([&](std::string_view k, const nve::BinValueView &val) {
              NSString *key = nsstring_or_nil_from_sv(k);
              if (key != nil) {
                dict[key] = objc_object_from_bin_value(val, 1u);
              }
            });
          } else {
            dict[@"value"] = objc_object_from_bin_value(*parsed, 0u);
          }
        } else {
          NSString *raw = nsstring_or_nil_from_sv(std::string_view(
              reinterpret_cast<const char *>(ev.payload_bytes.data()), ev.payload_bytes.size()));
          if (raw != nil) {
            dict[@"raw"] = raw;
          }
        }
      }
      break;
    }
    case nve::EventKind::Focus:
    case nve::EventKind::Blur:
      break;
  }
  return [dict copy];
}

} // namespace nve::apple
