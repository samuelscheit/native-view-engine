// Bounded-LRU `QImage` cache for `nve.res.image_src` resources: `data:image/...;base64,` URLs,
// `file://` and absolute-path URLs, and inline encoded image bytes (keyed by content hash).
#pragma once

#include <nve/host/core_resources.hpp>

#include <QImage>

#include <cstdint>
#include <list>
#include <string>
#include <unordered_map>

namespace nve::qt {

/// Decodes `src` into a premultiplied image (null when unsupported or undecodable).
[[nodiscard]] QImage decode_image(const host::ImageSource& src);

class NveQtImageCache {
 public:
  explicit NveQtImageCache(size_t max_entries = 64u);

  /// Cached decode of `src` (implicitly shared; null when undecodable, which is not cached).
  [[nodiscard]] QImage get(const host::ImageSource& src);

  void clear();
  [[nodiscard]] size_t size() const noexcept { return entries_.size(); }
  [[nodiscard]] uint64_t hits() const noexcept { return hits_; }
  [[nodiscard]] uint64_t misses() const noexcept { return misses_; }

 private:
  struct Entry {
    QImage image;
    std::list<std::string>::iterator lru;
  };
  size_t max_entries_;
  std::list<std::string> lru_;
  std::unordered_map<std::string, Entry> entries_;
  uint64_t hits_ = 0u;
  uint64_t misses_ = 0u;
};

}  // namespace nve::qt
