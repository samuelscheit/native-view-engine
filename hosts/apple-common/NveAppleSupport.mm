#import "NveAppleSupport.h"

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
        NSData *data = [NSData dataWithBytes:ev.payload_bytes.data()
                                      length:ev.payload_bytes.size()];
        NSError *jsonError = nil;
        id parsed = [NSJSONSerialization JSONObjectWithData:data options:0 error:&jsonError];
        if (!jsonError && [parsed isKindOfClass:[NSDictionary class]]) {
          [dict addEntriesFromDictionary:(NSDictionary *)parsed];
        } else if (!jsonError && parsed != nil) {
          dict[@"value"] = parsed;
        } else {
          NSString *raw = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
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
