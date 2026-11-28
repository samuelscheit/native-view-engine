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
#include <optional>
#include <string>
#include <string_view>

namespace nve::apple {

/// Native view class realized for a runtime node, derived from its attached core & canvas tables
/// (`nve.text_input` > `nve.image` > `nve.scroll` > leaf `nve.text` > `nve.canvas` > container).
enum class NodeKind : uint8_t {
  Container = 0,
  Text,
  TextInput,
  Image,
  Scroll,
  Canvas,
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
  bool read_only{false};
  bool hidden{false};
  std::string placeholder;
  bool secure_text_entry{false};
  bool multiline{false};
  uint32_t max_length{0u};
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
  std::string font_family;
  float font_size{16.0f};
  uint16_t font_weight{400u};
  bool italic{false};
  float letter_spacing{0.0f};
  uint8_t line_height_unit{NVE_LINE_HEIGHT_AUTO};
  float line_height{0.0f};
  uint32_t text_align{0u};
  uint32_t max_lines{0u};
  uint8_t text_decoration{0u};
  uint8_t white_space{NVE_WHITE_SPACE_NORMAL};
  uint8_t word_break{NVE_WORD_BREAK_NORMAL};
  uint8_t overflow_wrap{NVE_OVERFLOW_WRAP_NORMAL};
};

/// Unified typography specification shared by `native_text_measure` and AppKit/UIKit label painting.
struct TextSpec {
  std::string_view font_family;
  float font_size{16.0f};
  uint16_t font_weight{400u};
  bool italic{false};
  float letter_spacing{0.0f};
  uint8_t line_height_unit{NVE_LINE_HEIGHT_AUTO};
  float line_height{0.0f};
  uint32_t max_lines{0u};
  uint32_t text_align{NVE_TEXT_ALIGN_START};
  uint8_t text_decoration{0u};
  uint8_t white_space{NVE_WHITE_SPACE_NORMAL};
  uint8_t word_break{NVE_WORD_BREAK_NORMAL};
  uint8_t overflow_wrap{NVE_OVERFLOW_WRAP_NORMAL};
  uint32_t text_rgba{0xFF000000u};
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
  uint32_t canvas_draw_list{0u};
  uint32_t canvas_generation{0u};
};

/// Monotonic `std::chrono::steady_clock` timestamp in microseconds.
uint64_t current_timestamp_us() noexcept;

/// Packed little-endian RGBA8888 (R in the low byte, A in the high byte) to `NSColor` / `UIColor`.
NvePlatformColor *_Nonnull color_from_rgba(uint32_t rgba);

/// Resolves a CSS `font-family` list (`nve/host/font_family.hpp`) at `size` / CSS `weight` / italic.
NvePlatformFont *_Nonnull platform_font(
    float size,
    uint16_t weight,
    bool italic,
    std::string_view font_family = {});

/// Converts `NodeStyle` or `NveRuntimeTextStyle` into the shared `TextSpec`.
TextSpec text_spec_from_style(const NodeStyle &style) noexcept;
TextSpec text_spec_from_runtime_style(const NveRuntimeTextStyle &style) noexcept;

/// Resolved `NSLineBreakMode` for `spec` (matching `nve::host::text_wrap` / `min_content_wrap`).
NSLineBreakMode line_break_mode_for(const TextSpec &spec, bool for_min_content = false) noexcept;

/// Shared `NSAttributedString` attributes honoring `font_family`, `letter_spacing`, `line_height`,
/// `text_decoration` (underline + strikethrough), alignment, and wrap mode.
NSDictionary<NSAttributedStringKey, id> *_Nonnull text_attributes(
    const TextSpec &spec,
    bool include_color,
    bool for_min_content = false);

/// Shared attributed string for painting labels identically to `native_text_measure`.
NSAttributedString *_Nonnull attributed_text(std::string_view text, const TextSpec &spec);

/// `NveRuntimeMeasureTextFn` for runtime layout using the exact same `text_attributes` /
/// `platform_font` / `NSLayoutManager` (`max_lines`) / wrap rules as AppKit & UIKit labels.
void native_text_measure(void *_Nullable ctx, NveStrSlice text, const NveRuntimeTextStyle *_Nonnull style,
                         float max_width, float *_Nonnull out_width, float *_Nonnull out_height);

/// Parsed components of an `nve://pair?...` deep link (preserving canonical TLS `cert_hash` and
/// `quic`/`wss` query parameters intact in `endpoint`).
struct PairUrlConfig {
  std::string endpoint;
  std::string token;
  std::string transport;
};

/// Parses an `nve://pair?...` deep link. Preserves canonical `format_wire_uri()` URLs intact so
/// `quic`, `wss`, `path`, and `cert_hash` reach `nve-wire`'s `parse_wire_uri`.
std::optional<PairUrlConfig> parse_pair_url(std::string_view url);

/// Canonicalizes `--pair-transport` values (`auto`, `quic`/`udp`, `wt`/`webtransport`/`h3`,
/// `wss`/`tls`, `ws`/`tcp`/`websocket`) matching `nve-wire-core`'s `NveWireTransportKind::parse`.
std::optional<std::string_view> canonical_pair_transport(std::string_view raw) noexcept;

/// Applies an optional `--pair-transport` override onto `uri` (`nve://pair?...` query param or
/// scheme rewrite for `host:port` / `ws(s)://` / `quic://` / `wt://`). Returns `std::nullopt`
/// when `transport` is invalid or `uri` carries an unrecognized `scheme://`.
std::optional<std::string> apply_pair_transport(std::string_view uri, std::string_view transport);

/// `CALayer` background, border, corner radius, opacity, z-order, and clipping from `style`.
void apply_common_layer_style(CALayer *_Nullable layer, const NodeStyle &style);

/// Native AppKit (`NSAccessibility`) / UIKit (`UIAccessibility`) semantics for a mounted view.
void apply_node_accessibility(NvePlatformView *_Nullable view, NodeKind kind, const NodeSemantics &sem);

/// `YES` while a native text input (`NSTextField`/`UITextField`/`UITextView`) has active IME composition.
BOOL is_text_input_composing_ime(NvePlatformView *_Nullable view);

/// Writes `text` into a native text input while preserving and clamping its caret/selection.
void write_text_input_preserving_selection(NvePlatformView *_Nullable view, NSString *_Nonnull text);

/// Placeholder, secure entry, multiline, read-only, enabled state, and optional IME-safe text write.
void apply_text_input_props(NvePlatformView *_Nullable view, std::string_view text, const NodeSemantics &sem,
                            bool apply_text = true);

}  // namespace nve::apple
