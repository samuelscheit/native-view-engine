#include "NveWinBlurSurfaceView.hpp"

#include <algorithm>
#include <cmath>

namespace nve::blur::win {

namespace {

float base_blur_radius(BlurMaterial mat) noexcept {
  switch (mat) {
    case BlurMaterial::UltraThin: return 12.0f;
    case BlurMaterial::Thin: return 18.0f;
    case BlurMaterial::Regular: return 24.0f;
    case BlurMaterial::Thick: return 32.0f;
    case BlurMaterial::Chrome: return 28.0f;
    case BlurMaterial::SystemMaterial: return 24.0f;
  }
  return 24.0f;
}

float base_material_alpha(BlurMaterial mat) noexcept {
  switch (mat) {
    case BlurMaterial::UltraThin: return 0.45f;
    case BlurMaterial::Thin: return 0.60f;
    case BlurMaterial::Regular: return 0.72f;
    case BlurMaterial::Thick: return 0.85f;
    case BlurMaterial::Chrome: return 0.80f;
    case BlurMaterial::SystemMaterial: return 0.75f;
  }
  return 0.72f;
}

} // namespace

WinTintColor unpack_win_tint_rrggbbaa(uint32_t rrggbbaa) noexcept {
  WinTintColor out{};
  out.r8 = static_cast<uint8_t>((rrggbbaa >> 24u) & 0xFFu);
  out.g8 = static_cast<uint8_t>((rrggbbaa >> 16u) & 0xFFu);
  out.b8 = static_cast<uint8_t>((rrggbbaa >> 8u) & 0xFFu);
  out.a8 = static_cast<uint8_t>(rrggbbaa & 0xFFu);
  out.r = static_cast<float>(out.r8) / 255.0f;
  out.g = static_cast<float>(out.g8) / 255.0f;
  out.b = static_cast<float>(out.b8) / 255.0f;
  out.a = static_cast<float>(out.a8) / 255.0f;
  return out;
}

WinSystemBackdropType map_win_backdrop_type(BlurMaterial material) noexcept {
  switch (material) {
    case BlurMaterial::UltraThin:
    case BlurMaterial::Thin:
      return WinSystemBackdropType::TransientWindow;
    case BlurMaterial::Regular:
    case BlurMaterial::SystemMaterial:
      return WinSystemBackdropType::MainWindow;
    case BlurMaterial::Thick:
    case BlurMaterial::Chrome:
      return WinSystemBackdropType::TabbedWindow;
  }
  return WinSystemBackdropType::MainWindow;
}

void NveWinBlurSurfaceView::apply_props(const BlurSurfaceProps& props) noexcept {
  const float clamped =
      std::isfinite(props.intensity) ? std::clamp(props.intensity, 0.0f, 1.0f) : 1.0f;
  material_ = props.material;
  backdrop_type_ = map_win_backdrop_type(props.material);
  intensity_ = clamped;
  blur_radius_ = base_blur_radius(props.material) * clamped;
  effective_alpha_ = base_material_alpha(props.material) * clamped;
  has_tint_ = props.tint_rgba.has_value();
  tint_rgba_ = props.tint_rgba.value_or(0u);
  tint_color_ = has_tint_ ? unpack_win_tint_rrggbbaa(tint_rgba_) : WinTintColor{};
  ++update_count_;
}

#if defined(_WIN32)
bool NveWinBlurSurfaceView::apply_to_hwnd(HWND hwnd) const noexcept {
  if (hwnd == nullptr) return false;
  HMODULE dwm = GetModuleHandleW(L"dwmapi.dll");
  if (dwm == nullptr) {
    dwm = LoadLibraryW(L"dwmapi.dll");
  }
  if (dwm == nullptr) return false;
  using DwmSetWindowAttributeFn = HRESULT(WINAPI*)(HWND, DWORD, LPCVOID, DWORD);
  auto fn = reinterpret_cast<DwmSetWindowAttributeFn>(
      GetProcAddress(dwm, "DwmSetWindowAttribute"));
  if (fn == nullptr) return false;
  constexpr DWORD kDwmwaSystemBackdropType = 38u;
  const uint32_t backdrop = static_cast<uint32_t>(backdrop_type_);
  return SUCCEEDED(fn(hwnd, kDwmwaSystemBackdropType, &backdrop, sizeof(backdrop)));
}
#endif

} // namespace nve::blur::win
