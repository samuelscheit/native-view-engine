#pragma once

#include "nve_abi.h"
#include "nve_rn_component.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>
#include <memory>
#include <mutex>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <dwrite.h>
#include <windows.h>
#endif

namespace nve::win {

/// Configuration for native Windows DirectWrite text measurement & layout.
struct TextMeasurerOptions {
  std::string font_family{"Segoe UI"};
  std::string locale{"en-US"};
};

/// RAII DirectWrite text measurer (with a portable compile-time fallback on non-Windows hosts
/// so shared host headers and unit tests can be syntax-checked cross-platform).
class DirectWriteTextMeasurer {
 public:
  explicit DirectWriteTextMeasurer(TextMeasurerOptions options = {})
      : options_(std::move(options)) {
#if defined(_WIN32)
    font_family_w_ = utf8_to_wide(options_.font_family.empty() ? "Segoe UI" : options_.font_family);
    locale_w_ = utf8_to_wide(options_.locale.empty() ? "en-US" : options_.locale);
    IDWriteFactory* raw_factory = nullptr;
    const HRESULT hr = ::DWriteCreateFactory(
        DWRITE_FACTORY_TYPE_SHARED,
        __uuidof(IDWriteFactory),
        reinterpret_cast<IUnknown**>(&raw_factory));
    if (SUCCEEDED(hr) && raw_factory != nullptr) {
      factory_ = raw_factory;
    }
#endif
  }

  ~DirectWriteTextMeasurer() { release_resources(); }

  DirectWriteTextMeasurer(const DirectWriteTextMeasurer&) = delete;
  DirectWriteTextMeasurer& operator=(const DirectWriteTextMeasurer&) = delete;

  DirectWriteTextMeasurer(DirectWriteTextMeasurer&& other) noexcept
      : options_(std::move(other.options_))
#if defined(_WIN32)
        ,
        font_family_w_(std::move(other.font_family_w_)),
        locale_w_(std::move(other.locale_w_)),
        factory_(other.factory_),
        format_cache_(std::move(other.format_cache_))
#endif
  {
#if defined(_WIN32)
    other.factory_ = nullptr;
    other.format_cache_.clear();
#endif
  }

  DirectWriteTextMeasurer& operator=(DirectWriteTextMeasurer&& other) noexcept {
    if (this != &other) {
      release_resources();
      options_ = std::move(other.options_);
#if defined(_WIN32)
      font_family_w_ = std::move(other.font_family_w_);
      locale_w_ = std::move(other.locale_w_);
      factory_ = other.factory_;
      format_cache_ = std::move(other.format_cache_);
      other.factory_ = nullptr;
      other.format_cache_.clear();
#endif
    }
    return *this;
  }

  bool is_valid() const noexcept {
#if defined(_WIN32)
    return factory_ != nullptr;
#else
    return true;
#endif
  }

  const TextMeasurerOptions& options() const noexcept { return options_; }

#if defined(_WIN32)
  IDWriteFactory* factory() const noexcept { return factory_; }

  static std::wstring utf8_to_wide(std::string_view utf8) {
    if (utf8.empty()) {
      return {};
    }
    if (utf8.size() > static_cast<size_t>(std::numeric_limits<int>::max())) {
      return {};
    }
    const int needed = ::MultiByteToWideChar(
        CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), nullptr, 0);
    if (needed <= 0) {
      return {};
    }
    std::wstring wide(static_cast<size_t>(needed), L'\0');
    const int written = ::MultiByteToWideChar(
        CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), wide.data(), needed);
    if (written <= 0) {
      return {};
    }
    return wide;
  }

  /// Create a DirectWrite `IDWriteTextLayout` using the shared factory and cached Segoe UI
  /// `IDWriteTextFormat`. Caller owns the returned COM reference (`Release()`).
  bool create_text_layout(
      std::string_view text,
      float font_size,
      float max_width,
      float max_height,
      IDWriteTextLayout** out_layout) const {
    if (out_layout == nullptr) {
      return false;
    }
    *out_layout = nullptr;
    if (factory_ == nullptr || text.empty()) {
      return false;
    }

    const float sanitized_font = sanitize_font_size(font_size);
    const std::wstring wide = utf8_to_wide(text);
    if (wide.empty() || wide.size() > static_cast<size_t>(std::numeric_limits<UINT32>::max())) {
      return false;
    }

    // Use a large finite bound instead of FLT_MAX so DirectWrite layout metrics never overflow.
    constexpr float kUnconstrainedExtent = 1000000.0f;
    const bool has_max_w = (max_width > 0.0f && std::isfinite(max_width));
    const bool has_max_h = (max_height > 0.0f && std::isfinite(max_height));
    const float layout_w = has_max_w ? max_width : kUnconstrainedExtent;
    const float layout_h = has_max_h ? max_height : kUnconstrainedExtent;

    std::lock_guard<std::mutex> lock(mutex_);
    IDWriteTextFormat* fmt = get_or_create_format_locked(sanitized_font);
    if (fmt == nullptr) {
      return false;
    }

    IDWriteTextLayout* layout = nullptr;
    const HRESULT hr = factory_->CreateTextLayout(
        wide.c_str(),
        static_cast<UINT32>(wide.size()),
        fmt,
        layout_w,
        layout_h,
        &layout);
    if (FAILED(hr) || layout == nullptr) {
      return false;
    }

    (void)layout->SetWordWrapping(
        has_max_w ? DWRITE_WORD_WRAPPING_WRAP : DWRITE_WORD_WRAPPING_NO_WRAP);
    *out_layout = layout;
    return true;
  }
#endif

  /// Measure UTF-8 text in logical DIPs (1/96 in), honoring `max_width` word wrapping when
  /// `max_width > 0` and finite. Empty text always returns `{0, 0}`.
  NveIntrinsicSize measure(std::string_view text, float font_size, float max_width) const {
    if (text.empty()) {
      return NveIntrinsicSize{0.0f, 0.0f};
    }

    const float sanitized_font = sanitize_font_size(font_size);

#if defined(_WIN32)
    IDWriteTextLayout* layout = nullptr;
    if (create_text_layout(text, sanitized_font, max_width, 1000000.0f, &layout)) {
      DWRITE_TEXT_METRICS metrics{};
      const HRESULT hr = layout->GetMetrics(&metrics);
      layout->Release();
      if (SUCCEEDED(hr)) {
        const float raw_w =
            std::max(metrics.width, metrics.widthIncludingTrailingWhitespace);
        float out_w = (std::isfinite(raw_w) && raw_w > 0.0f) ? std::ceil(raw_w) + 1.0f : 0.0f;
        if (max_width > 0.0f && std::isfinite(max_width) && out_w > max_width) {
          out_w = max_width;
        }
        const float out_h =
            (std::isfinite(metrics.height) && metrics.height > 0.0f) ? std::ceil(metrics.height)
                                                                     : 0.0f;
        return NveIntrinsicSize{out_w, out_h};
      }
    }
#endif

    // Portable / fallback glyph metric approximation.
    const float char_width = sanitized_font * 0.55f;
    const float line_height = std::ceil(sanitized_font * 1.35f);
    const float intrinsic_w = std::ceil(static_cast<float>(text.size()) * char_width + 2.0f);
    const bool has_max_w = (max_width > 0.0f && std::isfinite(max_width));
    const float out_w = has_max_w ? std::min(intrinsic_w, max_width) : intrinsic_w;
    float out_h = line_height;
    if (has_max_w && out_w > 0.0f && intrinsic_w > out_w) {
      out_h = std::ceil(intrinsic_w / out_w) * line_height;
    }
    return NveIntrinsicSize{out_w, out_h};
  }

 private:
  static float sanitize_font_size(float font_size) noexcept {
    return (std::isfinite(font_size) && font_size > 0.0f) ? font_size : 16.0f;
  }

  void release_resources() noexcept {
#if defined(_WIN32)
    std::lock_guard<std::mutex> lock(mutex_);
    for (auto& [_, fmt] : format_cache_) {
      if (fmt != nullptr) {
        fmt->Release();
      }
    }
    format_cache_.clear();
    if (factory_ != nullptr) {
      factory_->Release();
      factory_ = nullptr;
    }
#endif
  }

#if defined(_WIN32)
  IDWriteTextFormat* get_or_create_format_locked(float font_size) const {
    if (factory_ == nullptr) {
      return nullptr;
    }
    const uint32_t key = static_cast<uint32_t>(std::lround(font_size * 100.0f));
    auto it = format_cache_.find(key);
    if (it != format_cache_.end()) {
      return it->second;
    }

    IDWriteTextFormat* fmt = nullptr;
    const HRESULT hr = factory_->CreateTextFormat(
        font_family_w_.c_str(),
        nullptr,
        DWRITE_FONT_WEIGHT_REGULAR,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        font_size,
        locale_w_.c_str(),
        &fmt);
    if (FAILED(hr) || fmt == nullptr) {
      return nullptr;
    }
    (void)fmt->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
    (void)fmt->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
    format_cache_.emplace(key, fmt);
    return fmt;
  }
#endif

  TextMeasurerOptions options_;
#if defined(_WIN32)
  std::wstring font_family_w_;
  std::wstring locale_w_;
  IDWriteFactory* factory_{nullptr};
  mutable std::mutex mutex_;
  mutable std::unordered_map<uint32_t, IDWriteTextFormat*> format_cache_;
#endif
};

/// Build a `nve::TextMeasurerCallback` backed by a shared `DirectWriteTextMeasurer` instance,
/// ready to pass directly into `nve::NativeHostSession::set_text_measurer(...)`.
inline nve::TextMeasurerCallback make_directwrite_text_measurer(
    std::shared_ptr<DirectWriteTextMeasurer> measurer = nullptr) {
  if (!measurer) {
    measurer = std::make_shared<DirectWriteTextMeasurer>();
  }
  return [measurer = std::move(measurer)](
             std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
    return measurer->measure(text, font_size, max_width);
  };
}

} // namespace nve::win
