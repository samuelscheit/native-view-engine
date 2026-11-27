#import "NveAppleImageCache.h"

#import <QuartzCore/QuartzCore.h>

#include <algorithm>
#include <cctype>
#include <list>
#include <string>
#include <unordered_map>
#include <utility>

namespace nve::apple {

namespace {

std::string ascii_lower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

bool starts_with_icase(std::string_view text, std::string_view prefix) noexcept {
  if (text.size() < prefix.size()) {
    return false;
  }
  for (size_t i = 0; i < prefix.size(); ++i) {
    if (std::tolower(static_cast<unsigned char>(text[i])) !=
        std::tolower(static_cast<unsigned char>(prefix[i]))) {
      return false;
    }
  }
  return true;
}

NvePlatformImage * _Nullable decode_image_data(NSData * _Nullable data) {
  if (data == nil || data.length == 0u) {
    return nil;
  }
#if TARGET_OS_OSX
  NSImage *img = [[NSImage alloc] initWithData:data];
  if (img == nil || img.size.width <= 0.0 || img.size.height <= 0.0) {
    return nil;
  }
  return img;
#else
  UIImage *img = [[UIImage alloc] initWithData:data];
  if (img == nil || img.size.width <= 0.0 || img.size.height <= 0.0) {
    return nil;
  }
  return img;
#endif
}

NvePlatformImage * _Nullable decode_image_source(std::string_view src) {
  if (src.empty()) {
    return nil;
  }
  @autoreleasepool {
    if (starts_with_icase(src, "data:")) {
      const size_t comma = src.find(',');
      if (comma == std::string_view::npos || comma + 1u >= src.size()) {
        return nil;
      }
      const std::string header = ascii_lower(src.substr(0u, comma));
      if (header.find("data:image/") != 0u || header.find(";base64") == std::string::npos) {
        return nil;
      }
      const std::string_view payload = src.substr(comma + 1u);
      NSString *b64 = [[NSString alloc] initWithBytes:payload.data()
                                               length:payload.size()
                                             encoding:NSASCIIStringEncoding];
      if (b64 == nil || b64.length == 0u) {
        return nil;
      }
      NSData *decoded =
          [[NSData alloc] initWithBase64EncodedString:b64
                                              options:NSDataBase64DecodingIgnoreUnknownCharacters];
      return decode_image_data(decoded);
    }

    if (starts_with_icase(src, "file://")) {
      NSString *urlStr = [[NSString alloc] initWithBytes:src.data()
                                                  length:src.size()
                                                encoding:NSUTF8StringEncoding];
      if (urlStr == nil) {
        return nil;
      }
      NSURL *url = [NSURL URLWithString:urlStr];
      if (url == nil || !url.isFileURL) {
        // Handle unescaped spaces in local file:// paths gracefully.
        const std::string_view rawPath = src.substr(7u);
        NSString *pathStr = [[NSString alloc] initWithBytes:rawPath.data()
                                                     length:rawPath.size()
                                                   encoding:NSUTF8StringEncoding];
        url = pathStr != nil ? [NSURL fileURLWithPath:pathStr] : nil;
      }
      if (url == nil || !url.isFileURL) {
        return nil;
      }
      NSData *fileData = [NSData dataWithContentsOfURL:url];
      return decode_image_data(fileData);
    }
  }
  return nil;
}

} // namespace

ImageResizeMode parse_image_resize_mode(std::string_view raw) noexcept {
  if (raw.empty()) {
    return ImageResizeMode::Cover;
  }
  const std::string lower = ascii_lower(raw);
  if (lower == "contain") {
    return ImageResizeMode::Contain;
  }
  if (lower == "stretch") {
    return ImageResizeMode::Stretch;
  }
  if (lower == "center") {
    return ImageResizeMode::Center;
  }
  return ImageResizeMode::Cover;
}

void apply_image_resize_mode(
    NvePlatformImageView * _Nullable image_view,
    ImageResizeMode mode,
    float border_radius) {
  if (image_view == nil) {
    return;
  }
  const BOOL hasRadius = border_radius > 0.0f;
#if TARGET_OS_OSX
  image_view.wantsLayer = YES;
  image_view.imageAlignment = NSImageAlignCenter;
  switch (mode) {
    case ImageResizeMode::Cover:
      image_view.imageScaling = NSImageScaleProportionallyUpOrDown;
      image_view.layer.contentsGravity = kCAGravityResizeAspectFill;
      image_view.layer.masksToBounds = YES;
      break;
    case ImageResizeMode::Contain:
      image_view.imageScaling = NSImageScaleProportionallyUpOrDown;
      image_view.layer.contentsGravity = kCAGravityResizeAspect;
      image_view.layer.masksToBounds = hasRadius;
      break;
    case ImageResizeMode::Stretch:
      image_view.imageScaling = NSImageScaleAxesIndependently;
      image_view.layer.contentsGravity = kCAGravityResize;
      image_view.layer.masksToBounds = hasRadius;
      break;
    case ImageResizeMode::ScaleDown:
      image_view.imageScaling = NSImageScaleProportionallyDown;
      image_view.layer.contentsGravity = kCAGravityCenter;
      image_view.layer.masksToBounds = YES;
      break;
    case ImageResizeMode::Center:
      image_view.imageScaling = NSImageScaleNone;
      image_view.layer.contentsGravity = kCAGravityCenter;
      image_view.layer.masksToBounds = YES;
      break;
  }
#else
  switch (mode) {
    case ImageResizeMode::Cover:
      image_view.contentMode = UIViewContentModeScaleAspectFill;
      image_view.clipsToBounds = YES;
      break;
    case ImageResizeMode::Contain:
      image_view.contentMode = UIViewContentModeScaleAspectFit;
      image_view.clipsToBounds = hasRadius;
      break;
    case ImageResizeMode::Stretch:
      image_view.contentMode = UIViewContentModeScaleToFill;
      image_view.clipsToBounds = hasRadius;
      break;
    case ImageResizeMode::Center:
      image_view.contentMode = UIViewContentModeCenter;
      image_view.clipsToBounds = YES;
      break;
    case ImageResizeMode::ScaleDown: {
      const CGSize img = image_view.image.size;
      const CGSize box = image_view.bounds.size;
      image_view.contentMode = (img.width > box.width || img.height > box.height) ? UIViewContentModeScaleAspectFit
                                                                                 : UIViewContentModeCenter;
      image_view.clipsToBounds = YES;
      break;
    }
  }
#endif
}

struct AppleImageCache::Impl {
  struct Entry {
    std::string key;
    NvePlatformImage *image;
  };
  using ListType = std::list<Entry>;
  using MapType = std::unordered_map<std::string, ListType::iterator>;

  explicit Impl(size_t max_entries) : capacity(std::max<size_t>(1u, max_entries)) {}

  size_t capacity;
  ListType lru;
  MapType index;
};

AppleImageCache::AppleImageCache(size_t max_entries) : impl_(new Impl(max_entries)) {}

AppleImageCache::~AppleImageCache() {
  delete impl_;
}

NvePlatformImage * _Nullable AppleImageCache::image_for_source(std::string_view src) {
  if (src.empty()) {
    return nil;
  }
  std::string key(src);
  auto it = impl_->index.find(key);
  if (it != impl_->index.end()) {
    impl_->lru.splice(impl_->lru.begin(), impl_->lru, it->second);
    return it->second->image;
  }

  NvePlatformImage *decoded = decode_image_source(src);
  if (decoded == nil) {
    return nil;
  }

  if (impl_->lru.size() >= impl_->capacity) {
    const auto &oldest = impl_->lru.back();
    impl_->index.erase(oldest.key);
    impl_->lru.pop_back();
  }

  impl_->lru.push_front(Impl::Entry{key, decoded});
  impl_->index.emplace(std::move(key), impl_->lru.begin());
  return decoded;
}

void AppleImageCache::clear() noexcept {
  impl_->index.clear();
  impl_->lru.clear();
}

size_t AppleImageCache::size() const noexcept {
  return impl_->lru.size();
}

size_t AppleImageCache::capacity() const noexcept {
  return impl_->capacity;
}

AppleImageCache &shared_image_cache() {
  static AppleImageCache *kShared = new AppleImageCache(128u);
  return *kShared;
}

} // namespace nve::apple
