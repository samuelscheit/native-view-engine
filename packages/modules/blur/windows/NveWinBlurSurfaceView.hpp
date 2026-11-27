#pragma once

#include <cstdint>
#include <optional>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

namespace nve::blur::win {

enum class BlurMaterial : uint32_t {
  UltraThin = 0,
  Thin = 1,
  Regular = 2,
  Thick = 3,
  Chrome = 4,
  SystemMaterial = 5,
};

struct BlurSurfaceProps {
  BlurMaterial material{BlurMaterial::Regular};
  float intensity{1.0f};
  std::optional<uint32_t> tint_rgba;
};

/// Win32 DWM system backdrop type values matching `DWM_SYSTEMBACKDROP_TYPE` (Win11 22H2+).
enum class WinSystemBackdropType : uint32_t {
  Auto = 0u,
  None = 1u,
  MainWindow = 2u,       // Mica
  TransientWindow = 3u,  // Acrylic
  TabbedWindow = 4u,     // Mica Alt
};

/// Unpacked RGBA color decoded from `0xRRGGBBAA` packed native tint values.
struct WinTintColor {
  uint8_t r8{0u};
  uint8_t g8{0u};
  uint8_t b8{0u};
  uint8_t a8{0u};
  float r{0.0f};
  float g{0.0f};
  float b{0.0f};
  float a{0.0f};
};

WinTintColor unpack_win_tint_rrggbbaa(uint32_t rrggbbaa) noexcept;
WinSystemBackdropType map_win_backdrop_type(BlurMaterial material) noexcept;

/// Win32 Mica / Acrylic backdrop descriptor and Direct2D translucent frosted tint state for
/// `nve.blur_surface`.
class NveWinBlurSurfaceView {
 public:
  NveWinBlurSurfaceView() = default;

  void apply_props(const BlurSurfaceProps& props) noexcept;
  void mark_unmounted() noexcept { unmounted_ = true; }

#if defined(_WIN32)
  /// Applies `DWMWA_SYSTEMBACKDROP_TYPE` (attribute 38) to `hwnd` via `dwmapi.dll` if available.
  bool apply_to_hwnd(HWND hwnd) const noexcept;
#endif

  BlurMaterial material() const noexcept { return material_; }
  WinSystemBackdropType backdrop_type() const noexcept { return backdrop_type_; }
  float intensity() const noexcept { return intensity_; }
  float blur_radius() const noexcept { return blur_radius_; }
  float effective_alpha() const noexcept { return effective_alpha_; }
  bool has_tint() const noexcept { return has_tint_; }
  uint32_t tint_rgba() const noexcept { return tint_rgba_; }
  WinTintColor tint_color() const noexcept { return tint_color_; }
  uint32_t update_count() const noexcept { return update_count_; }
  bool unmounted() const noexcept { return unmounted_; }

 private:
  BlurMaterial material_{BlurMaterial::Regular};
  WinSystemBackdropType backdrop_type_{WinSystemBackdropType::MainWindow};
  float intensity_{1.0f};
  float blur_radius_{24.0f};
  float effective_alpha_{0.72f};
  bool has_tint_{false};
  uint32_t tint_rgba_{0u};
  WinTintColor tint_color_{};
  uint32_t update_count_{0u};
  bool unmounted_{false};
};

} // namespace nve::blur::win
