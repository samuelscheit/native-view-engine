#pragma once

#import <Foundation/Foundation.h>
#import <QuartzCore/QuartzCore.h>
#import <TargetConditionals.h>

#if TARGET_OS_OSX
#import <AppKit/AppKit.h>
typedef NSView NvePlatformView;
typedef NSColor NvePlatformColor;
typedef NSFont NvePlatformFont;
#else
#import <UIKit/UIKit.h>
typedef UIView NvePlatformView;
typedef UIColor NvePlatformColor;
typedef UIFont NvePlatformFont;
#endif

#import "NveAppleImageCache.h"
#import "NveAppleValueBridge.h"

#include "nve_abi.generated.h"
#include "nve_rn_component.hpp"
#include "nve_scene_abi.generated.h"

#include <cstdint>
#include <string>
#include <string_view>

namespace nve::apple {

inline constexpr uint64_t kEventMaskPress = 1ULL << 0;

/// Canonical cross-platform accessibility roles inferred from `NveRenderNode` and raw props.
enum class A11yRole : uint8_t {
  None = 0,
  Group,
  Button,
  Text,
  Header,
  TextBox,
  ScrollArea,
  Image,
  List,
  ListItem,
  CheckBox,
  Link,
};

/// Normalized accessibility, TextInput, and Image metadata extracted from `raw_props` + `NveRenderNode`.
struct NodeSemantics {
  A11yRole role{A11yRole::Group};
  bool explicit_role{false};
  std::string label;
  bool has_explicit_label{false};
  std::string hint;
  std::string value;
  bool has_explicit_value{false};
  bool disabled{false};
  std::string test_id;
  std::string placeholder;
  bool secure_text_entry{false};
  bool multiline{false};
  ImageResizeMode resize_mode{ImageResizeMode::Cover};
};

/// Uniform error codes emitted by `open_nve_dylib` under a caller-supplied NSError domain.
enum class DylibErrorCode : NSInteger {
  OpenFailed = 1,
  MissingEntryPoint = 2,
  MissingSurfaceInterface = 3,
};

struct LoadedDylib {
  void * _Nullable handle{nullptr};
  NveGetInterfaceFn _Nullable get_interface{nullptr};
  NveModule module{};
};

/// Monotonic `std::chrono::steady_clock` timestamp in microseconds.
uint64_t current_timestamp_us() noexcept;

/// Convert packed little-endian RGBA8888 (R in low byte, A in high byte) to `NSColor` / `UIColor`.
NvePlatformColor * _Nonnull color_from_rgba(uint32_t rgba);

/// Open a Rust NVE `cdylib`, resolve `nve_get_interface`, and validate `NVE_IFACE_SURFACE`.
/// On failure, closes any opened handle, populates `*out_error` (when non-null) with
/// `error_domain` and `DylibErrorCode` (1/2/3), and returns `false`.
bool open_nve_dylib(
    NSString * _Nullable dylib_path,
    NSString * _Nonnull error_domain,
    LoadedDylib * _Nonnull out_dylib,
    NSError * _Nullable * _Nullable out_error);

/// Safely `dlclose` a handle if non-null and zero the pointer.
void close_dylib(void * _Nullable &handle) noexcept;

/// Build a native `NSString::boundingRectWithSize:` text measurer callback for Yoga.
/// On macOS uses `NSFont` + 4pt horizontal slack for `NSTextFieldCell` insets; on iOS uses `UIFont` with 0pt slack.
nve::TextMeasurerCallback make_native_text_measurer();

/// Register `make_native_text_measurer()` on `host` (no-op if `host == nullptr`).
void install_native_text_measurer(NveFabricHostHandle _Nullable host);

/// Apply platform-agnostic `CALayer` border, corner radius, and z-index properties from `NveRenderNode`.
void apply_common_layer_style(CALayer * _Nullable layer, const NveRenderNode &node);

/// Extract normalized accessibility, TextInput, and Image semantics from an `NveRenderNode`
/// and optional `raw_props` entry.
NodeSemantics extract_node_semantics(
    const NveRenderNode &node,
    std::string_view text,
    const nve::RawPropMap * _Nullable raw_props);

/// Apply native AppKit (`NSAccessibility`) or UIKit (`UIAccessibility`) semantics to a mounted view.
void apply_node_accessibility(
    NvePlatformView * _Nullable view,
    const NveRenderNode &node,
    const NodeSemantics &sem);

/// Returns `YES` when a native text input currently has active IME marked text composition.
BOOL is_text_input_composing_ime(NvePlatformView * _Nullable view);

/// Apply native `NSTextField` / `UITextField` placeholder, secureTextEntry, multiline, enabled state,
/// and IME-safe text synchronization without clobbering active marked text.
void apply_text_input_props(
    NvePlatformView * _Nullable view,
    std::string_view text,
    const NodeSemantics &sem);

} // namespace nve::apple
