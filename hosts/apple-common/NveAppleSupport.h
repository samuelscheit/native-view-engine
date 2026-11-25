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

#include "nve_abi.h"
#include "nve_rn_component.hpp"

#include <cstdint>

namespace nve::apple {

inline constexpr uint64_t kEventMaskPress = 1ULL << 0;

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

/// Convert a decoded NVE event into an Objective-C payload dictionary.
NSDictionary<NSString *, id> * _Nonnull payload_dictionary_from_event(const nve::DecodedEvent &ev);

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

/// Apply platform-agnostic `CALayer` border, corner radius, and z-index properties from a node snapshot.
void apply_common_layer_style(CALayer * _Nullable layer, const nve::RenderedNodeSnapshot &snap);

} // namespace nve::apple
