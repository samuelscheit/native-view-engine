// react-native-windows 0.83+ Composition Fabric component hosting one `NveWinHost` surface.
//
// An optional layer ON TOP of the Windows host: built only as the `nve_rnw_component` target
// (`-DNVE_WITH_RNW_FABRIC=ON`); the host library never includes or links it. Slated to move to
// `packages/modules/react-native-fabric`.
#pragma once

#if defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

#include "NveWinHost.hpp"

#include <winrt/Microsoft.ReactNative.Composition.Input.h>
#include <winrt/Microsoft.ReactNative.Composition.h>
#include <winrt/Microsoft.ReactNative.h>
#include <winrt/Microsoft.UI.Composition.h>
#include <winrt/Microsoft.UI.Content.h>
#include <winrt/Windows.Foundation.h>

#include <functional>
#include <memory>
#include <string>

namespace nve::win {

struct NveRnwComponentRegistrationConfig {
  std::wstring component_name{L"NveNativeView"};
  NveGetInterfaceFn default_interface{nullptr};
  std::string default_dylib_path;
  bool use_content_island{false};
  // Both receive the view's host: null when no runtime / DirectWrite is available (an empty view).
  std::function<winrt::Microsoft::UI::Content::ContentIsland(
      winrt::Microsoft::ReactNative::Composition::ContentIslandComponentView const&, std::shared_ptr<NveWinHost>)>
      create_content_island;
  std::function<void(winrt::Microsoft::ReactNative::Composition::ViewComponentView const&, std::shared_ptr<NveWinHost>)>
      on_view_initialized;
};

/// The surface of a registered component view (`nullptr` once destroyed).
std::shared_ptr<NveWinHost> get_nve_host_from_component_view(winrt::Microsoft::ReactNative::ComponentView const& view);

/// Registers `config.component_name` with the package's Fabric builder; `false` without Fabric.
bool register_nve_rnw_component(winrt::Microsoft::ReactNative::IReactPackageBuilder const& package_builder,
                                NveRnwComponentRegistrationConfig config = {});

}  // namespace nve::win

#endif  // defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC
