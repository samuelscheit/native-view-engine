#import "NveAppleSupport.h"

#include "nve_value.hpp"

#include <chrono>
#include <cmath>
#include <dlfcn.h>

namespace nve::apple {

namespace {

NSError *make_dylib_error(NSString *domain, DylibErrorCode code, NSString *description) {
  return [NSError errorWithDomain:domain
                             code:static_cast<NSInteger>(code)
                         userInfo:@{NSLocalizedDescriptionKey : description}];
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
    case NVE_VAL_STR: {
      const std::string_view sv = view.as_str().value_or(std::string_view{});
      NSString *s = [[NSString alloc] initWithBytes:sv.data()
                                             length:sv.size()
                                           encoding:NSUTF8StringEncoding];
      return s ?: @"";
    }
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
        NSString *key = [[NSString alloc] initWithBytes:k.data()
                                                 length:k.size()
                                               encoding:NSUTF8StringEncoding];
        if (key != nil) {
          map[key] = objc_object_from_bin_value(val, depth + 1u);
        }
      });
      return [map copy];
    }
  }
  return [NSNull null];
}

} // namespace

uint64_t current_timestamp_us() noexcept {
  using Clock = std::chrono::steady_clock;
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch())
          .count());
}

NvePlatformColor *color_from_rgba(uint32_t rgba) {
  const CGFloat r = static_cast<CGFloat>(rgba & 0xFFu) / 255.0;
  const CGFloat g = static_cast<CGFloat>((rgba >> 8) & 0xFFu) / 255.0;
  const CGFloat b = static_cast<CGFloat>((rgba >> 16) & 0xFFu) / 255.0;
  const CGFloat a = static_cast<CGFloat>((rgba >> 24) & 0xFFu) / 255.0;
#if TARGET_OS_OSX
  return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
#else
  return [UIColor colorWithRed:r green:g blue:b alpha:a];
#endif
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
      dict[@"text"] = [NSString stringWithUTF8String:ev.text.c_str()] ?: @"";
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
              NSString *key = [[NSString alloc] initWithBytes:k.data()
                                                       length:k.size()
                                                     encoding:NSUTF8StringEncoding];
              if (key != nil) {
                dict[key] = objc_object_from_bin_value(val, 1u);
              }
            });
          } else {
            dict[@"value"] = objc_object_from_bin_value(*parsed, 0u);
          }
        } else {
          NSString *raw = [[NSString alloc] initWithBytes:ev.payload_bytes.data()
                                                   length:ev.payload_bytes.size()
                                                 encoding:NSUTF8StringEncoding];
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

bool open_nve_dylib(
    NSString * _Nullable dylib_path,
    NSString * _Nonnull error_domain,
    LoadedDylib * _Nonnull out_dylib,
    NSError * _Nullable * _Nullable out_error) {
  *out_dylib = LoadedDylib{};

  if (dylib_path == nil || dylib_path.length == 0) {
    if (out_error) {
      *out_error = make_dylib_error(error_domain, DylibErrorCode::OpenFailed, @"Empty dylib path");
    }
    return false;
  }

  void *handle = dlopen(dylib_path.UTF8String, RTLD_NOW | RTLD_LOCAL);
  if (!handle) {
    if (out_error) {
      const char *err = dlerror();
      *out_error = make_dylib_error(
          error_domain,
          DylibErrorCode::OpenFailed,
          err ? @(err) : @"dlopen failed");
    }
    return false;
  }

  auto get_iface = reinterpret_cast<NveGetInterfaceFn>(dlsym(handle, "nve_get_interface"));
  if (!get_iface) {
    dlclose(handle);
    if (out_error) {
      *out_error = make_dylib_error(
          error_domain,
          DylibErrorCode::MissingEntryPoint,
          @"Missing nve_get_interface symbol");
    }
    return false;
  }

  const NveModule mod = nve_resolve_module(get_iface);
  if (!mod.surface || !mod.surface->create) {
    dlclose(handle);
    if (out_error) {
      *out_error = make_dylib_error(
          error_domain,
          DylibErrorCode::MissingSurfaceInterface,
          @"Missing NVE_IFACE_SURFACE interface");
    }
    return false;
  }

  out_dylib->handle = handle;
  out_dylib->get_interface = get_iface;
  out_dylib->module = mod;
  return true;
}

void close_dylib(void * _Nullable &handle) noexcept {
  if (handle != nullptr) {
    dlclose(handle);
    handle = nullptr;
  }
}

nve::TextMeasurerCallback make_native_text_measurer() {
  return [](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
    if (text.empty()) {
      return NveIntrinsicSize{0.0f, 0.0f};
    }
    @autoreleasepool {
      NSString *str = [[NSString alloc] initWithBytes:text.data()
                                               length:text.size()
                                             encoding:NSUTF8StringEncoding];
      if (!str) {
        return NveIntrinsicSize{0.0f, 0.0f};
      }
      const CGFloat ptSize = static_cast<CGFloat>(font_size > 0.0f ? font_size : 16.0f);
      const CGFloat constraintW =
          (max_width > 0.0f && std::isfinite(max_width)) ? static_cast<CGFloat>(max_width)
                                                         : CGFLOAT_MAX;
#if TARGET_OS_OSX
      NSFont *font = [NSFont systemFontOfSize:ptSize];
      const NSRect bounds =
          [str boundingRectWithSize:NSMakeSize(constraintW, CGFLOAT_MAX)
                            options:NSStringDrawingUsesLineFragmentOrigin |
                                    NSStringDrawingUsesFontLeading
                         attributes:@{NSFontAttributeName : font}];
      // Add 4pt horizontal slack for NSTextFieldCell's 2pt leading/trailing text inset.
      constexpr double kHorizontalSlack = 4.0;
#else
      UIFont *font = [UIFont systemFontOfSize:ptSize];
      const CGRect bounds =
          [str boundingRectWithSize:CGSizeMake(constraintW, CGFLOAT_MAX)
                            options:NSStringDrawingUsesLineFragmentOrigin |
                                    NSStringDrawingUsesFontLeading
                         attributes:@{NSFontAttributeName : font}
                            context:nil];
      constexpr double kHorizontalSlack = 0.0;
#endif
      return NveIntrinsicSize{
          static_cast<float>(std::ceil(bounds.size.width) + kHorizontalSlack),
          static_cast<float>(std::ceil(bounds.size.height)),
      };
    }
  };
}

void install_native_text_measurer(NveFabricHostHandle _Nullable host) {
  if (host != nullptr) {
    nve::fabric_host_set_text_measurer(host, make_native_text_measurer());
  }
}

void apply_common_layer_style(CALayer * _Nullable layer, const nve::RenderedNodeSnapshot &snap) {
  if (!layer) return;
  layer.cornerRadius = static_cast<CGFloat>(snap.border_radius);
  layer.borderWidth = static_cast<CGFloat>(snap.border_width);
  layer.borderColor = color_from_rgba(snap.border_color_rgba).CGColor;
  layer.zPosition = static_cast<CGFloat>(snap.z_index);
}

} // namespace nve::apple
