require "json"

package = JSON.parse(File.read(File.join(__dir__, "package.json")))

Pod::Spec.new do |s|
  s.name         = "NveCamera"
  s.version      = package["version"]
  s.summary      = package["description"]
  s.homepage     = "https://github.com/native-view-engine/native-view-engine"
  s.license      = package["license"]
  s.authors      = { "NVE" => "dev@nve.dev" }
  s.source       = { :path => "." }

  # Supports both iOS (UIKit) and macOS (AppKit / react-native-macos)
  s.ios.deployment_target = "15.0"
  s.osx.deployment_target = "13.0"

  # Thin wrapper referencing shared camera, C++ hybrid, and Nitro bridge sources (zero duplication)
  s.source_files = [
    "../apple/NveAppleCamera.{h,mm}",
    "../cxx/include/**/*.{h,hpp}",
    "../../nitro-bridge/include/**/*.{h,hpp}",
    "../../nitro-bridge/src/nve_nitro_c_api.cpp",
    "cxx/include/**/*.{h,hpp}",
  ]

  s.public_header_files = [
    "../apple/NveAppleCamera.h",
    "../cxx/include/nve_camera.hpp",
    "../cxx/include/nve_camera_codec.hpp",
    "../cxx/include/nve_vision_camera_hybrid.hpp",
    "../../nitro-bridge/include/nve_nitro_core.hpp",
    "../../nitro-bridge/include/nve_nitro_promise.hpp",
    "cxx/include/nve_rn_camera_jsi.hpp",
    "cxx/include/nve_rn_camera_jsi_convert.hpp",
  ]

  s.frameworks = "Foundation"
  s.ios.frameworks = "UIKit"
  s.osx.frameworks = "AppKit"

  s.pod_target_xcconfig = {
    "CLANG_CXX_LANGUAGE_STANDARD" => "c++20",
    "CLANG_ENABLE_OBJC_ARC" => "YES",
    "GCC_PREPROCESSOR_DEFINITIONS" => "$(inherited) NVE_WITH_RN_JSI=1",
    "HEADER_SEARCH_PATHS" => [
      "\"$(PODS_TARGET_SRCROOT)/../apple\"",
      "\"$(PODS_TARGET_SRCROOT)/../cxx/include\"",
      "\"$(PODS_TARGET_SRCROOT)/../../nitro-bridge/include\"",
      "\"$(PODS_TARGET_SRCROOT)/cxx/include\"",
      "\"$(PODS_TARGET_SRCROOT)/../../../../hosts/apple-common\"",
      "\"$(PODS_TARGET_SRCROOT)/../../../../cxx/nve-fabric-bridge/include\"",
    ].join(" "),
  }

  if respond_to?(:install_modules_dependencies, true)
    install_modules_dependencies(s)
  else
    s.dependency "React-Core"
  end
end
