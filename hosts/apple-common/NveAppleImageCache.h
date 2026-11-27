#pragma once

#import <Foundation/Foundation.h>
#import <TargetConditionals.h>

#if TARGET_OS_OSX
#import <AppKit/AppKit.h>
typedef NSImage NvePlatformImage;
typedef NSImageView NvePlatformImageView;
#else
#import <UIKit/UIKit.h>
typedef UIImage NvePlatformImage;
typedef UIImageView NvePlatformImageView;
#endif

#include <cstddef>
#include <cstdint>
#include <string_view>

namespace nve::apple {

/// Cross-platform image scaling mode matching React Native `Image.resizeMode`.
enum class ImageResizeMode : uint8_t {
  Cover = 0,
  Contain = 1,
  Stretch = 2,
  Center = 3,
  /// CSS `scale-down`: `Center` when the image fits, else `Contain`.
  ScaleDown = 4,
};

/// Parse `resizeMode` / `resize_mode` raw prop string (case-insensitive, defaults to `Cover`).
ImageResizeMode parse_image_resize_mode(std::string_view raw) noexcept;

/// Apply `ImageResizeMode` and clipping (`clipsToBounds` / `masksToBounds`) to an `NSImageView`
/// (macOS) or `UIImageView` (iOS).
void apply_image_resize_mode(
    NvePlatformImageView * _Nullable image_view,
    ImageResizeMode mode,
    float border_radius);

/// Bounded-LRU decoded image cache supporting `data:image/...;base64,...` and `file://...` URIs.
class AppleImageCache {
 public:
  explicit AppleImageCache(size_t max_entries = 128u);
  ~AppleImageCache();

  AppleImageCache(const AppleImageCache &) = delete;
  AppleImageCache &operator=(const AppleImageCache &) = delete;

  /// Decode or return cached `NSImage` / `UIImage` for `src`. Returns `nil` on invalid input.
  NvePlatformImage * _Nullable image_for_source(std::string_view src);

  /// Clear all cached images.
  void clear() noexcept;

  /// Number of entries currently cached.
  size_t size() const noexcept;

  /// Maximum entry capacity.
  size_t capacity() const noexcept;

 private:
  struct Impl;
  Impl * _Nonnull impl_;
};

/// Shared process-wide bounded-LRU image cache used by Apple host views.
AppleImageCache &shared_image_cache();

/// Decodes encoded image bytes (PNG, JPEG, …); `nil` for empty, undecodable, or 0x0 images.
NvePlatformImage *_Nullable decode_image_data(NSData *_Nullable data);

} // namespace nve::apple
