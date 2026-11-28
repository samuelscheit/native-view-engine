#import "NveAppleSupport.h"

#import <CoreText/CoreText.h>

#include <nve/host/ascii.hpp>
#include <nve/host/font_family.hpp>
#include <nve/host/runtime_session.hpp>
#include <nve/host/scene_nodes.hpp>

#include <algorithm>
#include <chrono>
#include <cmath>

namespace nve::apple {

namespace {

CGFloat weight_trait_for(uint16_t weight) noexcept {
  if (weight < 150u) return -0.8;
  if (weight < 250u) return -0.6;
  if (weight < 350u) return -0.4;
  if (weight < 450u) return 0.0;
  if (weight < 550u) return 0.23;
  if (weight < 650u) return 0.3;
  if (weight < 750u) return 0.4;
  if (weight < 850u) return 0.56;
  return 0.62;
}

NvePlatformFont *apply_italic(NvePlatformFont *font, CGFloat pt, bool italic) {
  if (!italic || font == nil) return font;
#if TARGET_OS_OSX
  (void)pt;
  return [[NSFontManager sharedFontManager] convertFont:font toHaveTrait:NSItalicFontMask] ?: font;
#else
  UIFontDescriptor *d = [font.fontDescriptor fontDescriptorWithSymbolicTraits:UIFontDescriptorTraitItalic];
  return d != nil ? [UIFont fontWithDescriptor:d size:pt] : font;
#endif
}

NvePlatformFont *system_font_with_weight(CGFloat pt, CGFloat w, bool italic) {
#if TARGET_OS_OSX
  return apply_italic([NSFont systemFontOfSize:pt weight:w], pt, italic);
#else
  return apply_italic([UIFont systemFontOfSize:pt weight:w], pt, italic);
#endif
}

NvePlatformFont *named_family_font(std::string_view name, CGFloat pt, CGFloat w, bool italic) {
  NSString *nsName = nsstring_from_sv(name);
  if (nsName.length == 0u) return nil;
  NSDictionary *attrs = @{(__bridge NSString *)kCTFontFamilyNameAttribute : nsName};
  CTFontDescriptorRef baseDesc = CTFontDescriptorCreateWithAttributes((__bridge CFDictionaryRef)attrs);
  if (baseDesc == nullptr) return nil;
  NSSet *mandatory = [NSSet setWithObject:(__bridge NSString *)kCTFontFamilyNameAttribute];
  CTFontDescriptorRef matched = CTFontDescriptorCreateMatchingFontDescriptor(baseDesc, (__bridge CFSetRef)mandatory);
  CFRelease(baseDesc);
  if (matched == nullptr) return nil;

  NSMutableDictionary *traits = [NSMutableDictionary dictionaryWithObject:@(w) forKey:(__bridge NSString *)kCTFontWeightTrait];
  if (italic) {
    traits[(__bridge NSString *)kCTFontSymbolicTrait] = @(kCTFontTraitItalic);
  }
  CTFontDescriptorRef styled = CTFontDescriptorCreateCopyWithAttributes(
      matched, (__bridge CFDictionaryRef)@{(__bridge NSString *)kCTFontTraitsAttribute : traits});
  CFRelease(matched);
  if (styled == nullptr) return nil;
  CTFontRef ctFont = CTFontCreateWithFontDescriptor(styled, pt, nullptr);
  CFRelease(styled);
  if (ctFont == nullptr) return nil;
  NvePlatformFont *out = (__bridge_transfer NvePlatformFont *)ctFont;
  return apply_italic(out, pt, italic);
}

NvePlatformFont *generic_family_font(host::GenericFamily generic, CGFloat pt, CGFloat w, bool italic) {
  switch (generic) {
    case host::GenericFamily::SystemUi:
    case host::GenericFamily::SansSerif:
      return system_font_with_weight(pt, w, italic);
    case host::GenericFamily::Monospace: {
#if TARGET_OS_OSX
      NSFont *mono = [NSFont monospacedSystemFontOfSize:pt weight:w];
#else
      UIFont *mono = [UIFont monospacedSystemFontOfSize:pt weight:w];
#endif
      return apply_italic(mono, pt, italic);
    }
    case host::GenericFamily::Serif: {
#if TARGET_OS_OSX
      NSFontDescriptor *desc =
          [[NSFont systemFontOfSize:pt weight:w].fontDescriptor fontDescriptorWithDesign:NSFontDescriptorSystemDesignSerif];
      NSFont *serif = desc != nil ? [NSFont fontWithDescriptor:desc size:pt] : nil;
#else
      UIFontDescriptor *desc =
          [[UIFont systemFontOfSize:pt weight:w].fontDescriptor fontDescriptorWithDesign:UIFontDescriptorSystemDesignSerif];
      UIFont *serif = desc != nil ? [UIFont fontWithDescriptor:desc size:pt] : nil;
#endif
      if (serif != nil) return apply_italic(serif, pt, italic);
      return named_family_font("Georgia", pt, w, italic) ?: named_family_font("Times New Roman", pt, w, italic);
    }
    case host::GenericFamily::Cursive:
      return named_family_font("Snell Roundhand", pt, w, italic)
                 ?: named_family_font("Bradley Hand", pt, w, italic)
                        ?: named_family_font("Apple Chancery", pt, w, italic);
    case host::GenericFamily::Fantasy:
      return named_family_font("Papyrus", pt, w, italic);
    case host::GenericFamily::None:
      return nil;
  }
  return nil;
}

NSTextAlignment text_alignment_for(uint32_t align) noexcept {
  switch (align) {
    case NVE_TEXT_ALIGN_CENTER: return NSTextAlignmentCenter;
    case NVE_TEXT_ALIGN_RIGHT:
    case NVE_TEXT_ALIGN_END: return NSTextAlignmentRight;
    case NVE_TEXT_ALIGN_JUSTIFY: return NSTextAlignmentJustified;
    case NVE_TEXT_ALIGN_LEFT: return NSTextAlignmentLeft;
    default:
#if TARGET_OS_OSX
      return NSTextAlignmentLeft;
#else
      return NSTextAlignmentNatural;
#endif
  }
}

CGSize text_bounds(NSString *str, NSDictionary<NSAttributedStringKey, id> *attrs, CGFloat limit, uint32_t max_lines) {
  if (max_lines > 0u) {
    NSTextStorage *storage = [[NSTextStorage alloc] initWithString:str attributes:attrs];
    NSLayoutManager *lm = [[NSLayoutManager alloc] init];
    NSTextContainer *tc = [[NSTextContainer alloc] initWithSize:CGSizeMake(limit, CGFLOAT_MAX)];
    tc.lineFragmentPadding = 0.0;
    tc.maximumNumberOfLines = static_cast<NSUInteger>(max_lines);
    NSParagraphStyle *para = attrs[NSParagraphStyleAttributeName];
    if (para != nil) tc.lineBreakMode = para.lineBreakMode;
    [lm addTextContainer:tc];
    [storage addLayoutManager:lm];
    [lm ensureLayoutForTextContainer:tc];
    return [lm usedRectForTextContainer:tc].size;
  }
  constexpr NSStringDrawingOptions kOptions = NSStringDrawingUsesLineFragmentOrigin | NSStringDrawingUsesFontLeading;
#if TARGET_OS_OSX
  return [str boundingRectWithSize:NSMakeSize(limit, CGFLOAT_MAX) options:kOptions attributes:attrs].size;
#else
  return [str boundingRectWithSize:CGSizeMake(limit, CGFLOAT_MAX) options:kOptions attributes:attrs context:nil].size;
#endif
}

std::string_view trim_ascii(std::string_view s) noexcept {
  while (!s.empty() && (s.front() == ' ' || s.front() == '\t' || s.front() == '\r' || s.front() == '\n')) s.remove_prefix(1u);
  while (!s.empty() && (s.back() == ' ' || s.back() == '\t' || s.back() == '\r' || s.back() == '\n')) s.remove_suffix(1u);
  return s;
}

bool starts_with_iequals(std::string_view s, std::string_view prefix) noexcept {
  return s.size() >= prefix.size() && host::ascii_iequals(s.substr(0u, prefix.size()), prefix);
}

}  // namespace

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

NvePlatformFont *platform_font(float size, uint16_t weight, bool italic, std::string_view font_family) {
  const CGFloat pt = static_cast<CGFloat>(size > 0.0f && std::isfinite(size) ? size : 16.0f);
  const CGFloat w = weight_trait_for(weight);
  NvePlatformFont *resolved = nil;
  if (!font_family.empty()) {
    host::for_each_font_family(font_family, [&](std::string_view entry, bool quoted) {
      if (!quoted) {
        const host::GenericFamily generic = host::generic_font_family(entry);
        if (generic != host::GenericFamily::None) {
          resolved = generic_family_font(generic, pt, w, italic);
          return resolved == nil;
        }
      }
      resolved = named_family_font(entry, pt, w, italic);
      return resolved == nil;
    });
  }
  return resolved != nil ? resolved : system_font_with_weight(pt, w, italic);
}

TextSpec text_spec_from_style(const NodeStyle &style) noexcept {
  return TextSpec{
      .font_family = style.font_family,
      .font_size = style.font_size,
      .font_weight = style.font_weight,
      .italic = style.italic,
      .letter_spacing = style.letter_spacing,
      .line_height_unit = style.line_height_unit,
      .line_height = style.line_height,
      .max_lines = style.max_lines,
      .text_align = style.text_align,
      .text_decoration = style.text_decoration,
      .white_space = style.white_space,
      .word_break = style.word_break,
      .overflow_wrap = style.overflow_wrap,
      .text_rgba = style.text_rgba,
  };
}

TextSpec text_spec_from_runtime_style(const NveRuntimeTextStyle &style) noexcept {
  return TextSpec{
      .font_family = host::str_view(style.font_family),
      .font_size = style.font_size,
      .font_weight = style.font_weight,
      .italic = style.font_style != NVE_FONT_STYLE_NORMAL,
      .letter_spacing = style.letter_spacing,
      .line_height_unit = style.line_height_unit,
      .line_height = style.line_height,
      .max_lines = style.max_lines,
      .text_align = NVE_TEXT_ALIGN_START,
      .text_decoration = 0u,
      .white_space = style.white_space,
      .word_break = style.word_break,
      .overflow_wrap = style.overflow_wrap,
      .text_rgba = 0xFF000000u,
  };
}

NSLineBreakMode line_break_mode_for(const TextSpec &spec, bool for_min_content) noexcept {
  const host::TextWrap wrap = host::text_wrap(spec.white_space, spec.word_break, spec.overflow_wrap);
  const host::TextWrap probe = for_min_content ? host::min_content_wrap(wrap) : wrap;
  if (probe == host::TextWrap::None) return NSLineBreakByClipping;
  if (probe == host::TextWrap::BreakAll || (for_min_content && probe == host::TextWrap::Anywhere)) {
    return NSLineBreakByCharWrapping;
  }
  return NSLineBreakByWordWrapping;
}

NSDictionary<NSAttributedStringKey, id> *text_attributes(const TextSpec &spec, bool include_color, bool for_min_content) {
  NSMutableParagraphStyle *para = [[NSMutableParagraphStyle alloc] init];
  para.lineBreakMode = line_break_mode_for(spec, for_min_content);
  para.alignment = text_alignment_for(spec.text_align);
  const double lh = host::line_height_points(spec.line_height_unit, spec.line_height, spec.font_size);
  if (lh > 0.0) {
    para.minimumLineHeight = static_cast<CGFloat>(lh);
    para.maximumLineHeight = static_cast<CGFloat>(lh);
  }
  NSMutableDictionary<NSAttributedStringKey, id> *attrs = [NSMutableDictionary dictionaryWithCapacity:6u];
  attrs[NSFontAttributeName] = platform_font(spec.font_size, spec.font_weight, spec.italic, spec.font_family);
  attrs[NSParagraphStyleAttributeName] = para;
  if (std::isfinite(spec.letter_spacing) && spec.letter_spacing != 0.0f) {
    attrs[NSKernAttributeName] = @(spec.letter_spacing);
  }
  if ((spec.text_decoration & NVE_TEXT_DECORATION_UNDERLINE) != 0u) {
    attrs[NSUnderlineStyleAttributeName] = @(NSUnderlineStyleSingle);
  }
  if ((spec.text_decoration & NVE_TEXT_DECORATION_LINE_THROUGH) != 0u) {
    attrs[NSStrikethroughStyleAttributeName] = @(NSUnderlineStyleSingle);
  }
  if (include_color) {
    attrs[NSForegroundColorAttributeName] = color_from_rgba(spec.text_rgba);
  }
  return attrs;
}

NSAttributedString *attributed_text(std::string_view text, const TextSpec &spec) {
  NSString *str = nsstring_from_sv(text);
  return [[NSAttributedString alloc] initWithString:str attributes:text_attributes(spec, true, false)];
}

void native_text_measure(void *ctx, NveStrSlice text, const NveRuntimeTextStyle *style, float max_width,
                         float *out_width, float *out_height) {
  (void)ctx;
  *out_width = 0.0f;
  *out_height = 0.0f;
  if (text.ptr == nullptr || text.len == 0u || style == nullptr) return;
  @autoreleasepool {
    NSString *str = [[NSString alloc] initWithBytes:text.ptr length:text.len encoding:NSUTF8StringEncoding];
    if (str == nil) return;
    const TextSpec spec = text_spec_from_runtime_style(*style);
    const host::TextWrap wrap = host::text_wrap(spec.white_space, spec.word_break, spec.overflow_wrap);
    const bool is_min_content = max_width == 0.0f;
    const host::TextWrap probe = is_min_content ? host::min_content_wrap(wrap) : wrap;
    NSDictionary<NSAttributedStringKey, id> *attrs = text_attributes(spec, false, is_min_content);

    CGFloat limit = CGFLOAT_MAX;
    if (probe != host::TextWrap::None) {
      if (is_min_content) {
        // Min-content probe (CSS Text 3 §5.5): `Word`/`BreakWord` use the widest word;
        // `Anywhere`/`BreakAll` use the widest non-whitespace composed character sequence.
        __block CGFloat widest = 0.0;
        NSMutableSet<NSString *> *seen = [NSMutableSet set];
        if (probe == host::TextWrap::Anywhere || probe == host::TextWrap::BreakAll) {
          static NSCharacterSet *const kNonWhitespace = NSCharacterSet.whitespaceAndNewlineCharacterSet.invertedSet;
          [str enumerateSubstringsInRange:NSMakeRange(0u, str.length)
                                  options:NSStringEnumerationByComposedCharacterSequences
                               usingBlock:^(NSString *cluster, NSRange, NSRange, BOOL *) {
                                 if (cluster.length == 0u ||
                                     [cluster rangeOfCharacterFromSet:kNonWhitespace].location == NSNotFound ||
                                     [seen containsObject:cluster]) {
                                   return;
                                 }
                                 [seen addObject:cluster];
                                 widest = std::max(widest, std::ceil(text_bounds(cluster, attrs, CGFLOAT_MAX, 0u).width));
                               }];
        } else {
          for (NSString *word in [str componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet]) {
            if (word.length == 0u || [seen containsObject:word]) continue;
            [seen addObject:word];
            widest = std::max(widest, std::ceil(text_bounds(word, attrs, CGFLOAT_MAX, 0u).width));
          }
        }
        limit = widest > 0.0 ? widest : CGFLOAT_MAX;
      } else if (max_width > 0.0f && std::isfinite(max_width)) {
        limit = static_cast<CGFloat>(max_width);
      }
    }
    const CGSize bounds = text_bounds(str, attrs, limit, spec.max_lines);
#if TARGET_OS_OSX
    constexpr double kHorizontalSlack = 4.0;  // NSTextFieldCell 2pt leading + trailing inset
#else
    constexpr double kHorizontalSlack = 0.0;
#endif
    *out_width = static_cast<float>(std::ceil(bounds.width) + kHorizontalSlack);
    *out_height = static_cast<float>(std::ceil(bounds.height));
  }
}

std::optional<std::string_view> canonical_pair_transport(std::string_view raw) noexcept {
  const std::string_view s = trim_ascii(raw);
  if (s.empty() || host::ascii_iequals(s, "auto") || host::ascii_iequals(s, "happy-eyeballs")) return "auto";
  if (host::ascii_iequals(s, "quic") || host::ascii_iequals(s, "udp")) return "quic";
  if (host::ascii_iequals(s, "wt") || host::ascii_iequals(s, "webtransport") || host::ascii_iequals(s, "h3")) {
    return "webtransport";
  }
  if (host::ascii_iequals(s, "wss") || host::ascii_iequals(s, "tls")) return "wss";
  if (host::ascii_iequals(s, "ws") || host::ascii_iequals(s, "tcp") || host::ascii_iequals(s, "websocket")) {
    return "ws";
  }
  return std::nullopt;
}

std::optional<std::string> apply_pair_transport(std::string_view uri, std::string_view transport) {
  const std::string_view trimmed_uri = trim_ascii(uri);
  const std::string_view trimmed_transport = trim_ascii(transport);
  if (trimmed_transport.empty()) return std::string(trimmed_uri);
  const std::optional<std::string_view> canon = canonical_pair_transport(trimmed_transport);
  if (!canon) return std::nullopt;
  if (trimmed_uri.empty() || *canon == "auto") return std::string(trimmed_uri);

  if (starts_with_iequals(trimmed_uri, "nve://")) {
    const size_t qpos = trimmed_uri.find('?');
    if (qpos == std::string_view::npos) {
      return std::string(trimmed_uri) + "?transport=" + std::string(*canon);
    }
    std::string out(trimmed_uri.substr(0u, qpos + 1u));
    std::string_view query = trimmed_uri.substr(qpos + 1u);
    bool first = true;
    while (!query.empty()) {
      const size_t amp = query.find('&');
      const std::string_view part = amp == std::string_view::npos ? query : query.substr(0u, amp);
      query = amp == std::string_view::npos ? std::string_view{} : query.substr(amp + 1u);
      if (part.empty()) continue;
      const size_t eq = part.find('=');
      const std::string_view key = eq == std::string_view::npos ? part : part.substr(0u, eq);
      if (host::ascii_iequals(key, "transport")) continue;
      if (!first) out.push_back('&');
      out.append(part);
      first = false;
    }
    if (!first) out.push_back('&');
    out.append("transport=");
    out.append(*canon);
    return out;
  }

  std::string_view rest = trimmed_uri;
  for (std::string_view prefix : {"quic://", "udp://", "wt://", "https://", "wss://", "ws://", "tcp://"}) {
    if (starts_with_iequals(rest, prefix)) {
      rest.remove_prefix(prefix.size());
      break;
    }
  }
  const std::string_view scheme = (*canon == "quic")           ? "quic://"
                                  : (*canon == "webtransport") ? "wt://"
                                  : (*canon == "wss")          ? "wss://"
                                                               : "ws://";
  std::string out;
  out.reserve(scheme.size() + rest.size());
  out.append(scheme);
  out.append(rest);
  return out;
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
