#import "NveAppleSupport.h"

#include "nve_value.hpp"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cmath>
#include <dlfcn.h>
#include <string_view>

namespace nve::apple {

namespace {

NSError *make_dylib_error(NSString *domain, DylibErrorCode code, NSString *description) {
  return [NSError errorWithDomain:domain
                             code:static_cast<NSInteger>(code)
                         userInfo:@{NSLocalizedDescriptionKey : description}];
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

A11yRole infer_default_role(const NveRenderNode &node) noexcept {
  switch (node.kind) {
    case NVE_NODE_KIND_TEXT_INPUT:
      return A11yRole::TextBox;
    case NVE_NODE_KIND_SCROLL_VIEW:
      return A11yRole::ScrollArea;
    case NVE_NODE_KIND_IMAGE:
      return A11yRole::Image;
    case NVE_NODE_KIND_PRESSABLE:
      return A11yRole::Button;
    default:
      break;
  }
  if ((node.flags & NVE_NODE_INTERACTIVE) != 0u) {
    return A11yRole::Button;
  }
  if (node.kind == NVE_NODE_KIND_TEXT) {
    return A11yRole::Text;
  }
  return A11yRole::Group;
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

void apply_common_layer_style(CALayer * _Nullable layer, const NveRenderNode &node) {
  if (!layer) return;
  layer.cornerRadius = static_cast<CGFloat>(node.border_radius);
  layer.borderWidth = static_cast<CGFloat>(node.border_width);
  layer.borderColor = color_from_rgba(node.border_rgba).CGColor;
  layer.zPosition = static_cast<CGFloat>(node.z_index);
  if (node.border_radius > 0.0f) {
    layer.masksToBounds = YES;
  }
}

NodeSemantics extract_node_semantics(
    const NveRenderNode &node,
    std::string_view text,
    const nve::RawPropMap * _Nullable raw_props) {
  NodeSemantics sem{};
  sem.role = infer_default_role(node);

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

    if (node.kind == NVE_NODE_KIND_IMAGE) {
      std::string resize_str;
      if (read_raw_string(*raw_props, "resizeMode", resize_str) ||
          read_raw_string(*raw_props, "resize_mode", resize_str)) {
        sem.resize_mode = parse_image_resize_mode(resize_str);
      }
    }
  }

  if (!sem.has_explicit_label && node.kind == NVE_NODE_KIND_TEXT &&
      (node.flags & NVE_NODE_LEAF_TEXT) != 0u && !text.empty()) {
    sem.label.assign(text.data(), text.size());
  }

  return sem;
}

void apply_node_accessibility(
    NvePlatformView * _Nullable view,
    const NveRenderNode &node,
    const NodeSemantics &sem) {
  if (!view) return;

  const BOOL isElement =
      (sem.role != A11yRole::None) &&
      (sem.role != A11yRole::Group || sem.has_explicit_label || !sem.hint.empty() ||
       !sem.test_id.empty() || sem.explicit_role);

  NSString *labelStr = nsstring_or_nil_from_sv(sem.label);
  NSString *hintStr = nsstring_or_nil_from_sv(sem.hint);
  NSString *valueStr = sem.has_explicit_value ? nsstring_or_nil_from_sv(sem.value) : nil;
  NSString *testIdStr = nsstring_or_nil_from_sv(sem.test_id);

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
  if (node.kind == NVE_NODE_KIND_TEXT_INPUT && [view isKindOfClass:[NSControl class]]) {
    ((NSControl *)view).enabled = !sem.disabled;
  }
#else
  (void)node;
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
    std::string_view text,
    const NodeSemantics &sem) {
  if (!view) return;
  NSString *nextText = nsstring_from_sv(text);
  NSString *placeholderStr = nsstring_or_nil_from_sv(sem.placeholder);

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
