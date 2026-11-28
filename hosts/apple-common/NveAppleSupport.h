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

#include <nve/generated/nve_abi.generated.h>
#include <nve/generated/nve_runtime_abi.generated.h>
#include <nve/generated/nve_tables.generated.h>

#include <cstdint>
#include <string>
#include <string_view>

namespace nve::apple {

/// Native view class realized for a runtime node, derived from its attached core tables
/// (`nve.text_input` > `nve.image` > `nve.scroll` > leaf `nve.text` > container).
enum class NodeKind : uint8_t {
  Container = 0,
  Text,
  TextInput,
  Image,
  Scroll,
};

/// Canonical cross-platform accessibility roles (`nve.accessibility.role` or inferred).
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

/// Accessibility + text-input semantics of one node (from `nve.accessibility`, `nve.text_input`).
struct NodeSemantics {
  A11yRole role{A11yRole::Group};
  bool explicit_role{false};
  std::string label;
  bool has_explicit_label{false};
  std::string hint;
  std::string value;
  bool has_explicit_value{false};
  bool disabled{false};
  bool hidden{false};
  std::string placeholder;
  bool secure_text_entry{false};
  bool multiline{false};
};

/// Paint style of one node resolved from `nve.visual` + inherited `nve.typography`.
struct NodeStyle {
  uint32_t background_rgba{0u};
  uint32_t border_rgba{0u};
  float border_width{0.0f};
  float border_radius{0.0f};
  float opacity{1.0f};
  int32_t z_index{0};
  bool clips_children{false};
  bool visible{true};
  uint32_t text_rgba{0xFF000000u};
  float font_size{16.0f};
  uint16_t font_weight{400u};
  bool italic{false};
  uint32_t text_align{0u};
  uint32_t max_lines{0u};
};

/// One fully resolved runtime node, as handed to the platform view factory / updater.
struct NodeView {
  uint32_t node_id{0u};  // raw `NodeId` (index:24 | generation:8)
  NodeKind kind{NodeKind::Container};
  CGRect frame{};        // absolute surface coordinates
  NodeStyle style{};
  NodeSemantics semantics{};
  std::string text;      // leaf text, or the text-input value
  bool interactive{false};
  uint32_t image_resource{0u};
  ImageResizeMode image_fit{ImageResizeMode::Cover};
  uint32_t image_tint_rgba{0u};
};

/// Monotonic `std::chrono::steady_clock` timestamp in microseconds.
uint64_t current_timestamp_us() noexcept;

/// Packed little-endian RGBA8888 (R in the low byte, A in the high byte) to `NSColor` / `UIColor`.
NvePlatformColor * _Nonnull color_from_rgba(uint32_t rgba);

/// Uniform error codes emitted by `open_nve_dylib` under a caller-supplied NSError domain.
enum class DylibErrorCode : NSInteger {
  OpenFailed = 1,
  MissingEntryPoint = 2,
  MissingSurfaceInterface = 3,
};

struct LoadedDylib {
  void * _Nullable handle{nullptr};
  NveGetInterfaceFn _Nullable get_interface{nullptr};
};

/// `dlopen` a Rust NVE `cdylib`, resolve `nve_get_interface`, and require `NVE_IFACE_SURFACE`.
/// On failure closes any opened handle, fills `*out_error` (code 1/2/3), and returns `false`.
bool open_nve_dylib(
    NSString * _Nullable dylib_path,
    NSString * _Nonnull error_domain,
    LoadedDylib * _Nonnull out_dylib,
    NSError * _Nullable * _Nullable out_error);

/// Safely `dlclose` a handle if non-null and zero the pointer.
void close_dylib(void * _Nullable &handle) noexcept;

/// `NveRuntimeMeasureTextFn` for runtime layout: the system font the views paint (size / CSS
/// weight / italic, `platform_font`) via `boundingRectWithSize:` (macOS adds
/// `NSTextFieldCell`'s 4pt horizontal inset). Apple views do not paint family, letter spacing,
/// line height, or max lines yet, so measurement ignores them too (paint parity).
void native_text_measure(void *_Nullable ctx, NveStrSlice text, const NveRuntimeTextStyle *_Nonnull style,
                         float max_width, float *_Nonnull out_width, float *_Nonnull out_height);

/// System font for `size` / CSS `weight` / italic.
NvePlatformFont * _Nonnull platform_font(float size, uint16_t weight, bool italic);

/// `CALayer` background, border, corner radius, opacity, z-order, and clipping from `style`.
void apply_common_layer_style(CALayer * _Nullable layer, const NodeStyle &style);

/// Native AppKit (`NSAccessibility`) / UIKit (`UIAccessibility`) semantics for a mounted view.
void apply_node_accessibility(NvePlatformView * _Nullable view, NodeKind kind, const NodeSemantics &sem);

/// `YES` while a native text input has active IME marked-text composition.
BOOL is_text_input_composing_ime(NvePlatformView * _Nullable view);

/// Placeholder, secure entry, multiline, enabled state, and IME-safe text synchronization.
void apply_text_input_props(NvePlatformView * _Nullable view, std::string_view text, const NodeSemantics &sem);

}  // namespace nve::apple
