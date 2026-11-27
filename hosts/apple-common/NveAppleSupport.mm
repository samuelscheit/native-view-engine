#import "NveAppleSupport.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <dlfcn.h>

namespace nve::apple {

uint64_t current_timestamp_us() noexcept {
  return static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(
                                   std::chrono::steady_clock::now().time_since_epoch())
                                   .count());
}

NvePlatformColor *color_from_rgba(uint32_t rgba) {
  const CGFloat r = static_cast<CGFloat>(rgba & 0xFFu) / 255.0;
  const CGFloat g = static_cast<CGFloat>((rgba >> 8u) & 0xFFu) / 255.0;
  const CGFloat b = static_cast<CGFloat>((rgba >> 16u) & 0xFFu) / 255.0;
  const CGFloat a = static_cast<CGFloat>((rgba >> 24u) & 0xFFu) / 255.0;
#if TARGET_OS_OSX
  return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
#else
  return [UIColor colorWithRed:r green:g blue:b alpha:a];
#endif
}

static NSError *dylib_error(NSString *domain, DylibErrorCode code, NSString *description) {
  return [NSError errorWithDomain:domain
                             code:static_cast<NSInteger>(code)
                         userInfo:@{NSLocalizedDescriptionKey : description}];
}

bool open_nve_dylib(NSString *dylib_path, NSString *error_domain, LoadedDylib *out_dylib, NSError **out_error) {
  *out_dylib = LoadedDylib{};
  if (dylib_path.length == 0) {
    if (out_error) *out_error = dylib_error(error_domain, DylibErrorCode::OpenFailed, @"Empty dylib path");
    return false;
  }
  void *handle = dlopen(dylib_path.UTF8String, RTLD_NOW | RTLD_LOCAL);
  if (handle == nullptr) {
    const char *err = dlerror();
    if (out_error) {
      *out_error = dylib_error(error_domain, DylibErrorCode::OpenFailed, err ? @(err) : @"dlopen failed");
    }
    return false;
  }
  auto get_iface = reinterpret_cast<NveGetInterfaceFn>(dlsym(handle, "nve_get_interface"));
  if (get_iface == nullptr) {
    dlclose(handle);
    if (out_error) {
      *out_error = dylib_error(error_domain, DylibErrorCode::MissingEntryPoint, @"Missing nve_get_interface symbol");
    }
    return false;
  }
  if (get_iface(1u /* NVE_IFACE_SURFACE */) == nullptr) {
    dlclose(handle);
    if (out_error) {
      *out_error = dylib_error(error_domain, DylibErrorCode::MissingSurfaceInterface, @"Missing NVE_IFACE_SURFACE interface");
    }
    return false;
  }
  out_dylib->handle = handle;
  out_dylib->get_interface = get_iface;
  return true;
}

void close_dylib(void *&handle) noexcept {
  if (handle != nullptr) {
    dlclose(handle);
    handle = nullptr;
  }
}

NvePlatformFont *platform_font(float size, uint16_t weight, bool italic) {
  const CGFloat pt = static_cast<CGFloat>(size > 0.0f && std::isfinite(size) ? size : 16.0f);
  CGFloat w = 0.0;
  if (weight < 150u) w = -0.8; else if (weight < 250u) w = -0.6; else if (weight < 350u) w = -0.4;
  else if (weight < 450u) w = 0.0; else if (weight < 550u) w = 0.23; else if (weight < 650u) w = 0.3;
  else if (weight < 750u) w = 0.4; else if (weight < 850u) w = 0.56; else w = 0.62;
#if TARGET_OS_OSX
  NSFont *font = [NSFont systemFontOfSize:pt weight:w];
  if (italic) font = [[NSFontManager sharedFontManager] convertFont:font toHaveTrait:NSItalicFontMask];
  return font;
#else
  UIFont *font = [UIFont systemFontOfSize:pt weight:w];
  if (italic) {
    UIFontDescriptor *d = [font.fontDescriptor fontDescriptorWithSymbolicTraits:UIFontDescriptorTraitItalic];
    if (d != nil) font = [UIFont fontWithDescriptor:d size:pt];
  }
  return font;
#endif
}

void native_text_measure(void *ctx, NveStrSlice text, float font_size, uint32_t font_weight, float max_width,
                         float *out_width, float *out_height) {
  (void)ctx;
  *out_width = 0.0f;
  *out_height = 0.0f;
  if (text.ptr == nullptr || text.len == 0u) return;
  @autoreleasepool {
    NSString *str = [[NSString alloc] initWithBytes:text.ptr length:text.len encoding:NSUTF8StringEncoding];
    if (str == nil) return;
    const CGFloat limit = (max_width > 0.0f && std::isfinite(max_width)) ? static_cast<CGFloat>(max_width) : CGFLOAT_MAX;
    NSDictionary *attrs = @{NSFontAttributeName : platform_font(font_size, static_cast<uint16_t>(std::min(font_weight, 1000u)), false)};
#if TARGET_OS_OSX
    const NSRect bounds = [str boundingRectWithSize:NSMakeSize(limit, CGFLOAT_MAX)
                                            options:NSStringDrawingUsesLineFragmentOrigin | NSStringDrawingUsesFontLeading
                                         attributes:attrs];
    constexpr double kHorizontalSlack = 4.0;  // NSTextFieldCell 2pt leading + trailing inset
#else
    const CGRect bounds = [str boundingRectWithSize:CGSizeMake(limit, CGFLOAT_MAX)
                                            options:NSStringDrawingUsesLineFragmentOrigin | NSStringDrawingUsesFontLeading
                                         attributes:attrs
                                            context:nil];
    constexpr double kHorizontalSlack = 0.0;
#endif
    *out_width = static_cast<float>(std::ceil(bounds.size.width) + kHorizontalSlack);
    *out_height = static_cast<float>(std::ceil(bounds.size.height));
  }
}

void apply_common_layer_style(CALayer *layer, const NodeStyle &style) {
  if (layer == nil) return;
  layer.backgroundColor = (style.background_rgba >> 24u) != 0u ? color_from_rgba(style.background_rgba).CGColor : nil;
  layer.cornerRadius = static_cast<CGFloat>(style.border_radius);
  layer.borderWidth = static_cast<CGFloat>(style.border_width);
  layer.borderColor = (style.border_rgba >> 24u) != 0u ? color_from_rgba(style.border_rgba).CGColor : nil;
  layer.opacity = style.visible ? style.opacity : 0.0f;
  layer.zPosition = static_cast<CGFloat>(style.z_index);
  layer.masksToBounds = style.clips_children || style.border_radius > 0.0f;
}

#if TARGET_OS_OSX
static NSAccessibilityRole appkit_role_for(A11yRole role, bool multiline) {
  switch (role) {
    case A11yRole::Button: return NSAccessibilityButtonRole;
    case A11yRole::Text:
    case A11yRole::Header: return NSAccessibilityStaticTextRole;
    case A11yRole::TextBox: return multiline ? NSAccessibilityTextAreaRole : NSAccessibilityTextFieldRole;
    case A11yRole::ScrollArea: return NSAccessibilityScrollAreaRole;
    case A11yRole::Image: return NSAccessibilityImageRole;
    case A11yRole::List: return NSAccessibilityListRole;
    case A11yRole::ListItem: return NSAccessibilityGroupRole;
    case A11yRole::CheckBox: return NSAccessibilityCheckBoxRole;
    case A11yRole::Link: return NSAccessibilityLinkRole;
    case A11yRole::None:
    case A11yRole::Group: return NSAccessibilityGroupRole;
  }
  return NSAccessibilityGroupRole;
}
#endif

void apply_node_accessibility(NvePlatformView *view, NodeKind kind, const NodeSemantics &sem) {
  if (view == nil) return;
  const BOOL isElement = !sem.hidden && sem.role != A11yRole::None &&
                         (sem.role != A11yRole::Group || sem.has_explicit_label || !sem.hint.empty() || sem.explicit_role);
  NSString *labelStr = nsstring_or_nil_from_sv(sem.label);
  NSString *hintStr = nsstring_or_nil_from_sv(sem.hint);
  NSString *valueStr = sem.has_explicit_value ? nsstring_or_nil_from_sv(sem.value) : nil;
#if TARGET_OS_OSX
  view.accessibilityElement = isElement;
  view.accessibilityRole = appkit_role_for(sem.role, sem.multiline);
  view.accessibilityLabel = labelStr;
  view.accessibilityHelp = hintStr;
  view.accessibilityEnabled = !sem.disabled;
  if (sem.has_explicit_value) {
    view.accessibilityValue = valueStr ?: @"";
  } else if (![view isKindOfClass:[NSTextField class]]) {
    view.accessibilityValue = nil;
  }
  if (kind == NodeKind::TextInput && [view isKindOfClass:[NSControl class]]) {
    ((NSControl *)view).enabled = !sem.disabled;
  }
#else
  (void)kind;
  view.isAccessibilityElement = isElement;
  view.accessibilityElementsHidden = sem.hidden;
  view.accessibilityLabel = labelStr;
  view.accessibilityHint = hintStr;
  if (sem.has_explicit_value) {
    view.accessibilityValue = valueStr ?: @"";
  } else if (![view isKindOfClass:[UITextField class]]) {
    view.accessibilityValue = nil;
  }
  UIAccessibilityTraits traits = UIAccessibilityTraitNone;
  switch (sem.role) {
    case A11yRole::Button:
    case A11yRole::CheckBox: traits |= UIAccessibilityTraitButton; break;
    case A11yRole::Header: traits |= UIAccessibilityTraitHeader; break;
    case A11yRole::Text: traits |= UIAccessibilityTraitStaticText; break;
    case A11yRole::Image: traits |= UIAccessibilityTraitImage; break;
    case A11yRole::Link: traits |= UIAccessibilityTraitLink; break;
    default: break;
  }
  if (sem.disabled) traits |= UIAccessibilityTraitNotEnabled;
  view.accessibilityTraits = traits;
  if ([view isKindOfClass:[UIControl class]]) ((UIControl *)view).enabled = !sem.disabled;
#endif
}

BOOL is_text_input_composing_ime(NvePlatformView *view) {
  if (view == nil) return NO;
#if TARGET_OS_OSX
  if ([view isKindOfClass:[NSTextField class]]) {
    NSText *editor = ((NSTextField *)view).currentEditor;
    if ([editor isKindOfClass:[NSTextView class]]) {
      NSTextView *tv = (NSTextView *)editor;
      return tv.hasMarkedText || tv.markedRange.length > 0;
    }
  }
  return NO;
#else
  return [view isKindOfClass:[UITextField class]] && ((UITextField *)view).markedTextRange != nil;
#endif
}

void apply_text_input_props(NvePlatformView *view, std::string_view text, const NodeSemantics &sem) {
  if (view == nil) return;
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
  if (field.markedTextRange == nil && ![(field.text ?: @"") isEqualToString:nextText]) {
    field.text = nextText;
  }
#endif
}

}  // namespace nve::apple
