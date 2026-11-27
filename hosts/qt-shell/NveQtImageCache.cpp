#include "NveQtImageCache.hpp"

#include <nve/host/xxh64.hpp>

#include <QByteArray>
#include <QString>
#include <QUrl>

#include <algorithm>
#include <cctype>
#include <cstdio>

namespace nve::qt {

namespace {

bool starts_with_ci(std::string_view s, std::string_view prefix) noexcept {
  return s.size() >= prefix.size() && std::equal(prefix.begin(), prefix.end(), s.begin(), [](char p, char c) {
           return p == static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
         });
}

QImage decode_url(std::string_view url) {
  QImage img;
  if (starts_with_ci(url, "data:image/")) {
    const size_t marker = url.find(";base64,");
    if (marker == std::string_view::npos || marker + 8u >= url.size()) return {};
    const std::string_view b64 = url.substr(marker + 8u);
    const QByteArray raw = QByteArray::fromBase64(QByteArray(b64.data(), static_cast<qsizetype>(b64.size())));
    if (raw.isEmpty() || !img.loadFromData(raw)) return {};
  } else if (starts_with_ci(url, "file://") || (!url.empty() && url.front() == '/')) {
    const QString qurl = QString::fromUtf8(url.data(), static_cast<qsizetype>(url.size()));
    const QString path = url.front() == '/' ? qurl : QUrl(qurl).toLocalFile();
    if (path.isEmpty() || !img.load(path)) return {};
  } else {
    return {};  // remote URLs are not fetched by the desktop host
  }
  return img;
}

}  // namespace

QImage decode_image(const host::ImageSource& src) {
  QImage img;
  if (src.is_url()) {
    img = decode_url(src.url);
  } else if (!img.loadFromData(src.encoded.data(), static_cast<int>(std::min<size_t>(src.encoded.size(), INT32_MAX)))) {
    return {};
  }
  return img.isNull() ? QImage() : img.convertToFormat(QImage::Format_RGBA8888_Premultiplied);
}

NveQtImageCache::NveQtImageCache(size_t max_entries) : max_entries_(std::max<size_t>(1u, max_entries)) {}

std::string NveQtImageCache::key_of(const host::ImageSource& src) {
  if (src.is_url()) return std::string(src.url);
  char key[48];
  std::snprintf(key, sizeof(key), "bytes:%016llx:%zu", static_cast<unsigned long long>(host::xxh64(src.encoded)),
                src.encoded.size());
  return key;
}

QImage NveQtImageCache::get(const host::ImageSource& src) {
  if (src.url.empty() && src.encoded.empty()) return {};
  std::string key = key_of(src);
  if (const auto it = entries_.find(key); it != entries_.end()) {
    lru_.splice(lru_.begin(), lru_, it->second.lru);
    ++hits_;
    return it->second.image;
  }
  ++misses_;
  QImage decoded = decode_image(src);
  if (decoded.isNull()) return {};
  while (entries_.size() >= max_entries_ && !lru_.empty()) {
    entries_.erase(lru_.back());
    lru_.pop_back();
  }
  lru_.push_front(key);
  entries_.emplace(std::move(key), Entry{decoded, lru_.begin()});
  return decoded;
}

void NveQtImageCache::clear() {
  entries_.clear();
  lru_.clear();
}

}  // namespace nve::qt
