// DirectWrite text layout and measurement for the Windows host (Windows only).
//
// `DirectWriteTextMeasurer` is installed as the runtime's text measurer (`measure_trampoline`
// matches `NveRuntimeMeasureTextFn`) and builds the `IDWriteTextLayout`s the Direct2D painter
// draws, so measured and painted text agree (family / size / weight / style / wrapping).
#pragma once

#if !defined(_WIN32)
#error "NveWinTextMeasurer.hpp is Windows-only"
#endif

#include "NveWinVisualTree.hpp"

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
    (void)layout->SetWordWrapping(style.wraps() && bounded_w ? DWRITE_WORD_WRAPPING_WRAP : DWRITE_WORD_WRAPPING_NO_WRAP);
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

  /// Measured size in DIPs (the runtime's `NveRuntimeMeasureTextFn` contract).
  [[nodiscard]] NveIntrinsicSize measure(std::string_view text, float font_size, uint16_t font_weight,
                                         float max_width) const {
    TextPaint style;
    style.font_size = std::isfinite(font_size) && font_size > 0.0f ? font_size : 16.0f;
    style.font_weight = font_weight;
    IDWriteTextLayout* layout = create_layout(text, style, max_width, 0.0f);
    if (layout == nullptr) return NveIntrinsicSize{0.0f, 0.0f};
    DWRITE_TEXT_METRICS m{};
    const HRESULT hr = layout->GetMetrics(&m);
    layout->Release();
    if (FAILED(hr)) return NveIntrinsicSize{0.0f, 0.0f};
    float w = std::ceil(std::max(m.width, m.widthIncludingTrailingWhitespace));
    if (std::isfinite(max_width) && max_width > 0.0f) w = std::min(w, max_width);
    return NveIntrinsicSize{std::isfinite(w) ? w : 0.0f, std::isfinite(m.height) ? std::ceil(m.height) : 0.0f};
  }

  static void measure_trampoline(void* ctx, NveStrSlice text, float font_size, uint32_t font_weight, float max_width,
                                 float* out_width, float* out_height) {
    const std::string_view sv(reinterpret_cast<const char*>(text.ptr), text.ptr != nullptr ? text.len : 0u);
    const NveIntrinsicSize s = static_cast<const DirectWriteTextMeasurer*>(ctx)->measure(
        sv, font_size, static_cast<uint16_t>(std::clamp<uint32_t>(font_weight, 1u, 1000u)), max_width);
    *out_width = s.width;
    *out_height = s.height;
  }

 private:
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
    const std::wstring family = s.font_family.empty() ? std::wstring(L"Segoe UI") : wide(s.font_family);
    // Sizes are sealed finite / positive by `read_node`; bound them so the key cannot overflow.
    const float size = std::clamp(s.font_size, 1.0f, kMaxFontSize);
    std::wstring key = family;
    key += L'|' + std::to_wstring(std::lround(size * 100.0f)) + L'|' + std::to_wstring(s.font_weight) + L'|' +
           std::to_wstring(s.font_style);
    if (const auto it = formats_.find(key); it != formats_.end()) {
      lru_.splice(lru_.begin(), lru_, it->second.at);
      return it->second.format;
    }
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
