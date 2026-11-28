// DirectWrite text layout and measurement for the Windows host (Windows only).
//
// `DirectWriteTextMeasurer` is installed as the runtime's text measurer (`measure_trampoline`
// matches `NveRuntimeMeasureTextFn`) and builds the `IDWriteTextLayout`s the Direct2D painter
// draws. Both go through `create_layout` with a `TextPaint`: the measurer's from the node's
// resolved `NveRuntimeTextStyle`, the painter's from the scene, so measured and painted text
// agree on family (first installed entry of the CSS list), size, weight, style, letter spacing,
// line height, white-space, and max lines.
#pragma once

#if !defined(_WIN32)
#error "NveWinTextMeasurer.hpp is Windows-only"
#endif

#include "NveWinVisualTree.hpp"

#include <nve/host/font_family.hpp>
#include <nve/host/utf16.hpp>

#include <dwrite.h>
#include <dwrite_1.h>
#include <windows.h>

#include <algorithm>
#include <cmath>
#include <limits>
#include <list>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace nve::win {

/// UTF-8 -> UTF-16 (`wchar_t`); invalid sequences become U+FFFD.
[[nodiscard]] inline std::wstring wide(std::string_view utf8) {
  const std::u16string u16 = host::utf8_to_utf16_lossy(utf8);
  return std::wstring(u16.begin(), u16.end());
}

/// UTF-16 (`wchar_t`) -> UTF-8; empty for lone surrogates.
[[nodiscard]] inline std::string narrow(std::wstring_view w) {
  const std::u16string u16(w.begin(), w.end());
  return host::utf16_to_utf8(u16).value_or(std::string());
}

class DirectWriteTextMeasurer {
 public:
  DirectWriteTextMeasurer() {
    (void)::DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, __uuidof(IDWriteFactory),
                                reinterpret_cast<IUnknown**>(&factory_));
    wchar_t locale[LOCALE_NAME_MAX_LENGTH] = {};
    if (::GetUserDefaultLocaleName(locale, LOCALE_NAME_MAX_LENGTH) > 0) locale_ = locale;
  }
  ~DirectWriteTextMeasurer() {
    for (auto& [key, entry] : formats_) entry.format->Release();
    if (factory_ != nullptr) factory_->Release();
  }
  DirectWriteTextMeasurer(const DirectWriteTextMeasurer&) = delete;
  DirectWriteTextMeasurer& operator=(const DirectWriteTextMeasurer&) = delete;

  [[nodiscard]] bool valid() const noexcept { return factory_ != nullptr; }
  [[nodiscard]] IDWriteFactory* factory() const noexcept { return factory_; }

  /// Layout of `text` styled by `style` in a `max_width` x `max_height` box (<= 0 / non-finite:
  /// unconstrained); the caller `Release()`s it. `nullptr` for empty text or DirectWrite failure.
  [[nodiscard]] IDWriteTextLayout* create_layout(std::string_view text, const TextPaint& style, float max_width,
                                                 float max_height) const {
    if (factory_ == nullptr || text.empty()) return nullptr;
    const std::wstring w = wide(text);
    if (w.empty() || w.size() > std::numeric_limits<UINT32>::max()) return nullptr;
    IDWriteTextFormat* fmt = format(style);
    if (fmt == nullptr) return nullptr;
    const bool bounded_w = std::isfinite(max_width) && max_width > 0.0f;
    const bool bounded_h = std::isfinite(max_height) && max_height > 0.0f;
    IDWriteTextLayout* layout = nullptr;
    if (FAILED(factory_->CreateTextLayout(w.c_str(), static_cast<UINT32>(w.size()), fmt,
                                          bounded_w ? std::min(max_width, kUnbounded) : kUnbounded,
                                          bounded_h ? std::min(max_height, kUnbounded) : kUnbounded, &layout))) {
      return nullptr;
    }
    const DWRITE_TEXT_RANGE all{0u, static_cast<UINT32>(w.size())};
    (void)layout->SetWordWrapping(bounded_w ? word_wrapping(style.wrap()) : DWRITE_WORD_WRAPPING_NO_WRAP);
    (void)layout->SetTextAlignment(alignment(style.text_align));
    if ((style.text_decoration & NVE_TEXT_DECORATION_UNDERLINE) != 0u) (void)layout->SetUnderline(TRUE, all);
    if ((style.text_decoration & NVE_TEXT_DECORATION_LINE_THROUGH) != 0u) (void)layout->SetStrikethrough(TRUE, all);
    if (style.letter_spacing != 0.0f) {
      IDWriteTextLayout1* l1 = nullptr;
      if (SUCCEEDED(layout->QueryInterface(__uuidof(IDWriteTextLayout1), reinterpret_cast<void**>(&l1)))) {
        (void)l1->SetCharacterSpacing(0.0f, style.letter_spacing, 0.0f, all);
        l1->Release();
      }
    }
    if (const float lh = line_height(style); lh > 0.0f) {
      (void)layout->SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, lh, lh * 0.8f);
    }
    if (style.max_lines > 0u) clamp_lines(layout, style.max_lines);
    return layout;
  }

  /// Measured size in DIPs of `text` laid out exactly as the painter lays it out (the runtime's
  /// `NveRuntimeMeasureTextFn` contract; `max_width` INFINITY = unconstrained).
  [[nodiscard]] NveIntrinsicSize measure(std::string_view text, const NveRuntimeTextStyle& style,
                                         float max_width) const {
    const TextPaint paint = TextPaint::from(style);
    const host::TextWrap wrap = paint.wrap();
    // `max_width == 0` probes min-content (CSS Text 3 §5.5): `Word` and `BreakWord` resolve to
    // the longest word (ceiled to whole pixels); `Anywhere` and `BreakAll` break between
    // characters (one cluster per line); `None` ignores `max_width`.
    const bool min_content = max_width == 0.0f && wrap != host::TextWrap::None;
    const host::TextWrap probe_wrap = min_content ? host::min_content_wrap(wrap) : wrap;
    TextPaint probe = paint;
    if (min_content && wrap == host::TextWrap::BreakWord) {
      probe.overflow_wrap = NVE_OVERFLOW_WRAP_NORMAL;
      probe.word_break = NVE_WORD_BREAK_NORMAL;
    }
    IDWriteTextLayout* layout = create_layout(text, probe, min_content ? kNarrowest : max_width, 0.0f);
    if (layout == nullptr) return NveIntrinsicSize{0.0f, 0.0f};
    FLOAT min_width = 0.0f;
    if (min_content && probe_wrap == host::TextWrap::Word && SUCCEEDED(layout->DetermineMinWidth(&min_width)) &&
        min_width > 0.0f) {
      layout->Release();
      layout = create_layout(text, probe, std::ceil(min_width), 0.0f);
      if (layout == nullptr) return NveIntrinsicSize{0.0f, 0.0f};
    }
    DWRITE_TEXT_METRICS m{};
    const HRESULT hr = layout->GetMetrics(&m);
    layout->Release();
    if (FAILED(hr)) return NveIntrinsicSize{0.0f, 0.0f};
    float w = std::ceil(std::max(m.width, m.widthIncludingTrailingWhitespace));
    if (std::isfinite(max_width) && max_width > 0.0f) w = std::min(w, max_width);
    return NveIntrinsicSize{std::isfinite(w) ? w : 0.0f, std::isfinite(m.height) ? std::ceil(m.height) : 0.0f};
  }

  static void measure_trampoline(void* ctx, NveStrSlice text, const NveRuntimeTextStyle* style, float max_width,
                                 float* out_width, float* out_height) {
    const NveIntrinsicSize s =
        static_cast<const DirectWriteTextMeasurer*>(ctx)->measure(host::str_view(text), *style, max_width);
    *out_width = s.width;
    *out_height = s.height;
  }

  /// First installed family of the CSS list `list` (unquoted generics, ASCII case-insensitive,
  /// mapped to Windows' defaults), else Segoe UI: `CreateTextFormat` and GDI's `CreateFontW` take
  /// one name and silently substitute unknown ones (the painter, the measurer, and the native
  /// EDIT controls all resolve through this).
  [[nodiscard]] std::wstring resolve_family(std::string_view list) const {
    std::wstring found;
    IDWriteFontCollection* fonts = nullptr;
    if (factory_ == nullptr || FAILED(factory_->GetSystemFontCollection(&fonts, FALSE)) || fonts == nullptr) {
      return L"Segoe UI";
    }
    host::for_each_font_family(list, [&](std::string_view name, bool quoted) {
      const wchar_t* generic = quoted ? nullptr : generic_face(host::generic_font_family(name));
      const std::wstring candidate = generic != nullptr ? std::wstring(generic) : wide(name);
      UINT32 index = 0u;
      BOOL exists = FALSE;
      if (SUCCEEDED(fonts->FindFamilyName(candidate.c_str(), &index, &exists)) && exists) found = candidate;
      return found.empty();
    });
    fonts->Release();
    return found.empty() ? std::wstring(L"Segoe UI") : found;
  }

 private:
  /// Windows' face of a CSS generic family (Chromium's defaults); null for a named family.
  static const wchar_t* generic_face(host::GenericFamily generic) noexcept {
    switch (generic) {
      case host::GenericFamily::Serif: return L"Times New Roman";
      case host::GenericFamily::SansSerif:
      case host::GenericFamily::SystemUi: return L"Segoe UI";
      case host::GenericFamily::Monospace: return L"Consolas";
      case host::GenericFamily::Cursive: return L"Comic Sans MS";
      case host::GenericFamily::Fantasy: return L"Impact";
      case host::GenericFamily::None: break;
    }
    return nullptr;
  }

  /// DirectWrite wrapping of a resolved `TextWrap` (`WHOLE_WORD`: an over-long word overflows,
  /// as CSS `normal`; `CHARACTER`: `break-all`; `EMERGENCY_BREAK`: words, else characters).
  static DWRITE_WORD_WRAPPING word_wrapping(host::TextWrap wrap) noexcept {
    switch (wrap) {
      case host::TextWrap::None: return DWRITE_WORD_WRAPPING_NO_WRAP;
      case host::TextWrap::Word: return DWRITE_WORD_WRAPPING_WHOLE_WORD;
      case host::TextWrap::BreakWord:
      case host::TextWrap::Anywhere: return DWRITE_WORD_WRAPPING_EMERGENCY_BREAK;
      case host::TextWrap::BreakAll: return DWRITE_WORD_WRAPPING_CHARACTER;
    }
    return DWRITE_WORD_WRAPPING_WRAP;
  }

  static DWRITE_TEXT_ALIGNMENT alignment(uint8_t align) noexcept {
    switch (align) {
      case NVE_TEXT_ALIGN_CENTER: return DWRITE_TEXT_ALIGNMENT_CENTER;
      case NVE_TEXT_ALIGN_END:
      case NVE_TEXT_ALIGN_RIGHT: return DWRITE_TEXT_ALIGNMENT_TRAILING;
      case NVE_TEXT_ALIGN_JUSTIFY: return DWRITE_TEXT_ALIGNMENT_JUSTIFIED;
      default: return DWRITE_TEXT_ALIGNMENT_LEADING;
    }
  }

  static float line_height(const TextPaint& s) noexcept {
    const float v = s.line_height.value;
    if (!(std::isfinite(v) && v > 0.0f)) return 0.0f;
    if (s.line_height.unit == NVE_LENGTH_POINTS) return v;
    if (s.line_height.unit == NVE_LENGTH_PERCENT) return s.font_size * v / 100.0f;
    return 0.0f;
  }

  /// Ellipsizes after `max_lines` lines: the layout box ends at that line and trims with "...".
  void clamp_lines(IDWriteTextLayout* layout, uint32_t max_lines) const {
    UINT32 count = 0u;
    (void)layout->GetLineMetrics(nullptr, 0u, &count);
    if (count <= max_lines) return;
    std::vector<DWRITE_LINE_METRICS> lines(count);
    if (FAILED(layout->GetLineMetrics(lines.data(), count, &count))) return;
    float h = 0.0f;
    for (uint32_t i = 0; i < max_lines && i < count; ++i) h += lines[i].height;
    (void)layout->SetMaxHeight(std::min(layout->GetMaxHeight(), h));
    IDWriteInlineObject* ellipsis = nullptr;
    if (SUCCEEDED(factory_->CreateEllipsisTrimmingSign(layout, &ellipsis))) {
      const DWRITE_TRIMMING trimming{DWRITE_TRIMMING_GRANULARITY_CHARACTER, 0u, 0u};
      (void)layout->SetTrimming(&trimming, ellipsis);
      ellipsis->Release();
    }
  }

  IDWriteTextFormat* format(const TextPaint& s) const {
    // Sizes are sealed finite / positive by `read_node`; bound them so the key cannot overflow.
    const float size = std::clamp(s.font_size, 1.0f, kMaxFontSize);
    std::wstring key = wide(s.font_family);  // the authored list: resolved once per format
    key += L'|' + std::to_wstring(std::lround(size * 100.0f)) + L'|' + std::to_wstring(s.font_weight) + L'|' +
           std::to_wstring(s.font_style);
    if (const auto it = formats_.find(key); it != formats_.end()) {
      lru_.splice(lru_.begin(), lru_, it->second.at);
      return it->second.format;
    }
    const std::wstring family = resolve_family(s.font_family);
    const DWRITE_FONT_STYLE style = s.font_style == NVE_FONT_STYLE_ITALIC    ? DWRITE_FONT_STYLE_ITALIC
                                    : s.font_style == NVE_FONT_STYLE_OBLIQUE ? DWRITE_FONT_STYLE_OBLIQUE
                                                                             : DWRITE_FONT_STYLE_NORMAL;
    IDWriteTextFormat* fmt = nullptr;
    if (FAILED(factory_->CreateTextFormat(family.c_str(), nullptr,
                                          static_cast<DWRITE_FONT_WEIGHT>(std::clamp<int>(s.font_weight, 1, 999)), style,
                                          DWRITE_FONT_STRETCH_NORMAL, size, locale_.c_str(), &fmt))) {
      return nullptr;
    }
    (void)fmt->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
    if (formats_.size() >= kMaxFormats) {  // evict the least recently used (layouts keep their own copy)
      const auto victim = formats_.find(lru_.back());
      victim->second.format->Release();
      formats_.erase(victim);
      lru_.pop_back();
    }
    lru_.push_front(key);
    formats_.emplace(std::move(key), Format{fmt, lru_.begin()});
    return fmt;
  }

  static constexpr float kUnbounded = 1.0e6f;  // finite: DirectWrite metrics overflow at FLT_MAX
  static constexpr float kNarrowest = 1.0f;    // min-content probe width: one cluster per line
  static constexpr float kMaxFontSize = 4096.0f;
  static constexpr size_t kMaxFormats = 64u;
  struct Format {
    IDWriteTextFormat* format = nullptr;
    std::list<std::wstring>::iterator at;
  };

  IDWriteFactory* factory_ = nullptr;
  std::wstring locale_ = L"en-us";
  mutable std::unordered_map<std::wstring, Format> formats_;  // family|size|weight|style
  mutable std::list<std::wstring> lru_;                       // most recent first
};

}  // namespace nve::win
