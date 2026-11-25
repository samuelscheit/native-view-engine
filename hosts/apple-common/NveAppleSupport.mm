#import "NveAppleSupport.h"

#include "nve_value.hpp"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cmath>
#include <cstring>
#include <dlfcn.h>
#include <string_view>

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

namespace {

NSString * _Nullable nsstring_from_std_string(const std::string &s) {
  if (s.empty()) {
    return nil;
  }
  return [[NSString alloc] initWithBytes:s.data()
                                  length:s.size()
                                encoding:NSUTF8StringEncoding];
}

std::string ascii_lower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

bool read_raw_string(const nve::RawPropMap &props, const char *key, std::string &out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto *s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  return false;
}

bool read_raw_value_as_string(const nve::RawPropMap &props, const char *key, std::string &out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto *s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  if (const auto *i = std::get_if<int64_t>(&it->second)) {
    out = std::to_string(*i);
    return true;
  }
  if (const auto *d = std::get_if<double>(&it->second)) {
    if (std::isfinite(*d)) {
      out = std::to_string(*d);
      return true;
    }
    return false;
  }
  if (const auto *b = std::get_if<bool>(&it->second)) {
    out = *b ? "true" : "false";
    return true;
  }
  return false;
}

bool read_raw_bool(const nve::RawPropMap &props, const char *key, bool &out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto *b = std::get_if<bool>(&it->second)) {
    out = *b;
    return true;
  }
  if (const auto *i = std::get_if<int64_t>(&it->second)) {
    out = (*i != 0);
    return true;
  }
  if (const auto *s = std::get_if<std::string>(&it->second)) {
    const std::string lower = ascii_lower(*s);
    if (lower == "true" || lower == "1" || lower == "yes") {
      out = true;
      return true;
    }
    if (lower == "false" || lower == "0" || lower == "no") {
      out = false;
      return true;
    }
  }
  return false;
}

bool parse_a11y_role(std::string_view raw, A11yRole &out) {
  const std::string lower = ascii_lower(raw);
  if (lower == "button") {
    out = A11yRole::Button;
    return true;
  }
  if (lower == "text" || lower == "statictext" || lower == "label") {
    out = A11yRole::Text;
    return true;
  }
  if (lower == "header" || lower == "heading") {
    out = A11yRole::Header;
    return true;
  }
  if (lower == "textbox" || lower == "textinput" || lower == "search" || lower == "searchbox") {
    out = A11yRole::TextBox;
    return true;
  }
  if (lower == "scrollarea" || lower == "scrollview") {
    out = A11yRole::ScrollArea;
    return true;
  }
  if (lower == "image" || lower == "img") {
    out = A11yRole::Image;
    return true;
  }
  if (lower == "list") {
    out = A11yRole::List;
    return true;
  }
  if (lower == "listitem") {
    out = A11yRole::ListItem;
    return true;
  }
  if (lower == "checkbox" || lower == "switch") {
    out = A11yRole::CheckBox;
    return true;
  }
  if (lower == "link") {
    out = A11yRole::Link;
    return true;
  }
  if (lower == "group" || lower == "region") {
    out = A11yRole::Group;
    return true;
  }
  if (lower == "none" || lower == "presentation") {
    out = A11yRole::None;
    return true;
  }
  return false;
}

A11yRole infer_default_role(const nve::RenderedNodeSnapshot &snap) noexcept {
  if (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & kEventMaskPress) != 0u) {
    return A11yRole::Button;
  }
  switch (snap.kind) {
    case NVE_COMP_TEXT:
      return A11yRole::Text;
    case NVE_COMP_TEXT_INPUT:
      return A11yRole::TextBox;
    case NVE_COMP_SCROLL_VIEW:
      return A11yRole::ScrollArea;
    case NVE_COMP_IMAGE:
      return A11yRole::Image;
    default:
      return A11yRole::Group;
  }
}

void decode_raw_props_from_command_buffer(
    const uint8_t *data,
    size_t len,
    std::unordered_map<uint32_t, nve::RawPropMap> &out) {
  if (data == nullptr || len < 24u) {
    return;
  }
  size_t pos = 0;
  auto read_pod = [&](auto &v) -> bool {
    if (sizeof(v) > len - pos) return false;
    std::memcpy(&v, data + pos, sizeof(v));
    pos += sizeof(v);
    return true;
  };
  auto skip = [&](size_t n) -> bool {
    if (n > len - pos) return false;
    pos += n;
    return true;
  };

  uint32_t magic = 0;
  uint16_t version = 0;
  uint16_t flags = 0;
  uint32_t surface_id = 0;
  uint64_t frame_id = 0;
  uint32_t op_count = 0;
  if (!read_pod(magic) || !read_pod(version) || !read_pod(flags) || !read_pod(surface_id) ||
      !read_pod(frame_id) || !read_pod(op_count)) {
    return;
  }
  if (magic != NVE_ABI_MAGIC || version != NVE_ABI_VERSION) {
    return;
  }

  for (uint32_t i = 0; i < op_count; ++i) {
    uint8_t op = 0;
    if (!read_pod(op)) return;
    switch (static_cast<NveOpcode>(op)) {
      case NVE_OP_CREATE_NODE: {
        uint16_t name_len = 0;
        if (!skip(6u) || !read_pod(name_len) || !skip(name_len)) return;
        break;
      }
      case NVE_OP_REMOVE_NODE:
        if (!skip(4u)) return;
        break;
      case NVE_OP_SET_CHILDREN: {
        uint32_t count = 0;
        if (!skip(4u) || !read_pod(count)) return;
        if (static_cast<size_t>(count) > (len - pos) / sizeof(uint32_t)) return;
        pos += static_cast<size_t>(count) * sizeof(uint32_t);
        break;
      }
      case NVE_OP_SET_TEXT: {
        uint32_t tlen = 0;
        if (!skip(4u) || !read_pod(tlen) || !skip(tlen)) return;
        break;
      }
      case NVE_OP_SET_LAYOUT:
        if (!skip(81u)) return;
        break;
      case NVE_OP_SET_STYLE:
        if (!skip(36u)) return;
        break;
      case NVE_OP_SET_RAW_PROP: {
        uint32_t node_id = 0;
        uint16_t key_len = 0;
        if (!read_pod(node_id) || !read_pod(key_len) || key_len > len - pos) return;
        std::string key(reinterpret_cast<const char *>(data + pos), key_len);
        pos += key_len;
        uint8_t tag = 0;
        if (!read_pod(tag)) return;
        nve::RawPropValue val{nullptr};
        switch (tag) {
          case 0:
            val = nullptr;
            break;
          case 1: {
            uint8_t b = 0;
            if (!read_pod(b)) return;
            val = (b != 0);
            break;
          }
          case 2: {
            int64_t v = 0;
            if (!read_pod(v)) return;
            val = v;
            break;
          }
          case 3: {
            double v = 0.0;
            if (!read_pod(v)) return;
            val = v;
            break;
          }
          case 4: {
            uint32_t vlen = 0;
            if (!read_pod(vlen) || vlen > len - pos) return;
            val = std::string(reinterpret_cast<const char *>(data + pos), vlen);
            pos += vlen;
            break;
          }
          case 5: {
            uint32_t vlen = 0;
            if (!read_pod(vlen) || vlen > len - pos) return;
            val = std::vector<uint8_t>(data + pos, data + pos + vlen);
            pos += vlen;
            break;
          }
          default:
            return;
        }
        out[node_id][std::move(key)] = std::move(val);
        break;
      }
      case NVE_OP_SET_EVENT_MASK:
        if (!skip(12u)) return;
        break;
      case NVE_OP_DISPATCH_COMMAND: {
        uint16_t nlen = 0;
        uint32_t plen = 0;
        if (!skip(4u) || !read_pod(nlen) || !skip(nlen) || !read_pod(plen) || !skip(plen)) return;
        break;
      }
      case NVE_OP_COMMIT:
        if (!skip(1u)) return;
        break;
      default:
        return;
    }
  }
}

#if TARGET_OS_OSX
NSAccessibilityRole appkit_role_for(A11yRole role, bool multiline) {
  switch (role) {
    case A11yRole::Button:
      return NSAccessibilityButtonRole;
    case A11yRole::Text:
    case A11yRole::Header:
      return NSAccessibilityStaticTextRole;
    case A11yRole::TextBox:
      return multiline ? NSAccessibilityTextAreaRole : NSAccessibilityTextFieldRole;
    case A11yRole::ScrollArea:
      return NSAccessibilityScrollAreaRole;
    case A11yRole::Image:
      return NSAccessibilityImageRole;
    case A11yRole::List:
      return NSAccessibilityListRole;
    case A11yRole::ListItem:
      return NSAccessibilityRowRole;
    case A11yRole::CheckBox:
      return NSAccessibilityCheckBoxRole;
    case A11yRole::Link:
      return NSAccessibilityLinkRole;
    case A11yRole::Group:
      return NSAccessibilityGroupRole;
    case A11yRole::None:
      return NSAccessibilityUnknownRole;
  }
  return NSAccessibilityGroupRole;
}
#endif

template <typename Snap>
std::unordered_map<uint32_t, nve::RawPropMap> extract_raw_props_impl(
    NveFabricHostHandle _Nullable host,
    const std::vector<Snap> &snapshots) {
  std::unordered_map<uint32_t, nve::RawPropMap> result;
  if constexpr (requires(const Snap &s) { s.raw_props; }) {
    for (const auto &snap : snapshots) {
      if (!snap.raw_props.empty()) {
        result.emplace(snap.node_id, snap.raw_props);
      }
    }
  } else {
    (void)snapshots;
    if (host == nullptr) {
      return result;
    }
    const std::vector<uint8_t> frame0 = nve::fabric_host_serialize_frame0(host, 1u);
    decode_raw_props_from_command_buffer(frame0.data(), frame0.size(), result);
  }
  return result;
}

} // namespace

std::unordered_map<uint32_t, nve::RawPropMap> extract_raw_props_by_node(
    NveFabricHostHandle _Nullable host,
    const std::vector<nve::RenderedNodeSnapshot> &snapshots) {
  return extract_raw_props_impl(host, snapshots);
}

NodeSemantics extract_node_semantics(
    const nve::RenderedNodeSnapshot &snap,
    const nve::RawPropMap * _Nullable raw_props) {
  NodeSemantics sem{};
  sem.role = infer_default_role(snap);

  if (raw_props != nullptr) {
    std::string role_str;
    if (read_raw_string(*raw_props, "accessibilityRole", role_str) ||
        read_raw_string(*raw_props, "role", role_str)) {
      A11yRole parsed = sem.role;
      if (parse_a11y_role(role_str, parsed)) {
        sem.role = parsed;
        sem.explicit_role = true;
      }
    }

    if (read_raw_string(*raw_props, "accessibilityLabel", sem.label) ||
        read_raw_string(*raw_props, "aria-label", sem.label)) {
      sem.has_explicit_label = !sem.label.empty();
    }

    (void)read_raw_string(*raw_props, "accessibilityHint", sem.hint);

    if (read_raw_value_as_string(*raw_props, "accessibilityValue", sem.value)) {
      sem.has_explicit_value = true;
    }

    if (!read_raw_bool(*raw_props, "accessibilityDisabled", sem.disabled)) {
      (void)read_raw_bool(*raw_props, "disabled", sem.disabled);
    }

    if (!read_raw_string(*raw_props, "testID", sem.test_id)) {
      (void)read_raw_string(*raw_props, "data-testid", sem.test_id);
    }

    (void)read_raw_string(*raw_props, "placeholder", sem.placeholder);
    (void)read_raw_bool(*raw_props, "secureTextEntry", sem.secure_text_entry);
    (void)read_raw_bool(*raw_props, "multiline", sem.multiline);
  }

  if (!sem.has_explicit_label && snap.kind == NVE_COMP_TEXT && snap.children.empty() &&
      !snap.text.empty()) {
    sem.label = snap.text;
  }

  return sem;
}

void apply_node_accessibility(
    NvePlatformView * _Nullable view,
    const nve::RenderedNodeSnapshot &snap,
    const NodeSemantics &sem) {
  if (!view) return;

  const BOOL isElement =
      (sem.role != A11yRole::None) &&
      (sem.role != A11yRole::Group || sem.has_explicit_label || !sem.hint.empty() ||
       !sem.test_id.empty() || sem.explicit_role);

  NSString *labelStr = nsstring_from_std_string(sem.label);
  NSString *hintStr = nsstring_from_std_string(sem.hint);
  NSString *valueStr = sem.has_explicit_value ? nsstring_from_std_string(sem.value) : nil;
  NSString *testIdStr = nsstring_from_std_string(sem.test_id);

#if TARGET_OS_OSX
  view.accessibilityElement = isElement;
  view.accessibilityRole = appkit_role_for(sem.role, sem.multiline);
  view.accessibilityLabel = labelStr;
  view.accessibilityHelp = hintStr;
  view.accessibilityEnabled = !sem.disabled;
  view.accessibilityIdentifier = testIdStr;
  view.identifier = testIdStr;
  if (sem.has_explicit_value) {
    view.accessibilityValue = valueStr ?: @"";
  } else if (![view isKindOfClass:[NSTextField class]]) {
    view.accessibilityValue = nil;
  }
  if (snap.kind == NVE_COMP_TEXT_INPUT && [view isKindOfClass:[NSControl class]]) {
    ((NSControl *)view).enabled = !sem.disabled;
  }
#else
  (void)snap;
  view.isAccessibilityElement = isElement;
  view.accessibilityLabel = labelStr;
  view.accessibilityHint = hintStr;
  if (sem.has_explicit_value) {
    view.accessibilityValue = valueStr ?: @"";
  } else if (![view isKindOfClass:[UITextField class]]) {
    view.accessibilityValue = nil;
  }
  view.accessibilityIdentifier = testIdStr;

  UIAccessibilityTraits traits = UIAccessibilityTraitNone;
  switch (sem.role) {
    case A11yRole::Button:
    case A11yRole::CheckBox:
      traits |= UIAccessibilityTraitButton;
      break;
    case A11yRole::Header:
      traits |= UIAccessibilityTraitHeader;
      break;
    case A11yRole::Text:
      traits |= UIAccessibilityTraitStaticText;
      break;
    case A11yRole::Image:
      traits |= UIAccessibilityTraitImage;
      break;
    case A11yRole::Link:
      traits |= UIAccessibilityTraitLink;
      break;
    default:
      break;
  }
  if (sem.disabled) {
    traits |= UIAccessibilityTraitNotEnabled;
  }
  view.accessibilityTraits = traits;
  if ([view isKindOfClass:[UIControl class]]) {
    ((UIControl *)view).enabled = !sem.disabled;
  }
#endif
}

BOOL is_text_input_composing_ime(NvePlatformView * _Nullable view) {
  if (!view) return NO;
#if TARGET_OS_OSX
  if ([view isKindOfClass:[NSTextField class]]) {
    NSText *editor = ((NSTextField *)view).currentEditor;
    if ([editor isKindOfClass:[NSTextView class]]) {
      NSTextView *tv = (NSTextView *)editor;
      if (tv.hasMarkedText || tv.markedRange.length > 0) {
        return YES;
      }
    }
  }
  return NO;
#else
  if ([view isKindOfClass:[UITextField class]]) {
    return ((UITextField *)view).markedTextRange != nil;
  }
  return NO;
#endif
}

void apply_text_input_props(
    NvePlatformView * _Nullable view,
    const nve::RenderedNodeSnapshot &snap,
    const NodeSemantics &sem) {
  if (!view) return;
  NSString *nextText =
      [[NSString alloc] initWithBytes:snap.text.data()
                               length:snap.text.size()
                             encoding:NSUTF8StringEncoding] ?: @"";
  NSString *placeholderStr = nsstring_from_std_string(sem.placeholder);

#if TARGET_OS_OSX
  if (![view isKindOfClass:[NSTextField class]]) return;
  NSTextField *field = (NSTextField *)view;
  field.placeholderString = placeholderStr;
  field.enabled = !sem.disabled;
  field.usesSingleLineMode = sem.multiline ? NO : YES;
  field.lineBreakMode = sem.multiline ? NSLineBreakByWordWrapping : NSLineBreakByClipping;
  if (field.cell != nil) {
    field.cell.wraps = sem.multiline ? YES : NO;
    field.cell.scrollable = sem.multiline ? NO : YES;
  }
  // Never clobber live IME composition (marked text) during a re-render.
  if (!is_text_input_composing_ime(field) && ![field.stringValue isEqualToString:nextText]) {
    field.stringValue = nextText;
  }
#else
  if (![view isKindOfClass:[UITextField class]]) return;
  UITextField *field = (UITextField *)view;
  field.placeholder = placeholderStr;
  field.secureTextEntry = sem.secure_text_entry ? YES : NO;
  field.enabled = !sem.disabled;
  // Never clobber live IME composition (`markedTextRange != nil`) during a re-render.
  if (field.markedTextRange == nil && ![(field.text ?: @"") isEqualToString:nextText]) {
    field.text = nextText;
  }
#endif
}

} // namespace nve::apple
