#include "NveQtImageCache.hpp"

#include <nve/host/ascii.hpp>
#include <nve/host/urls.hpp>

#include <QByteArray>
#include <QString>

#include <algorithm>

namespace nve::qt {

namespace {

QImage decode_url(std::string_view url) {
  QImage img;
  if (url.size() >= 11u && host::ascii_iequals(url.substr(0u, 11u), "data:image/")) {
    const size_t marker = url.find(";base64,");
    if (marker == std::string_view::npos || marker + 8u >= url.size()) return {};
    const std::string_view b64 = url.substr(marker + 8u);
    const QByteArray raw = QByteArray::fromBase64(QByteArray(b64.data(), static_cast<qsizetype>(b64.size())));
    if (raw.isEmpty() || !img.loadFromData(raw)) return {};
  } else if (const auto local = host::local_image_path(url)) {  // never a remote / UNC file
    // A bare path is `url` itself, used verbatim; a `file:` URL's path (host already checked
    // empty / `localhost`) is percent-decoded here, and the decoded path must still be local.
    // Not `QUrl::toLocalFile`: it turns any host, `localhost` included, into a `//host/` path.
    const QByteArray raw(local->data(), static_cast<qsizetype>(local->size()));
    const QByteArray decoded = local->data() == url.data() ? raw : QByteArray::fromPercentEncoding(raw);
    const std::string_view path(decoded.constData(), static_cast<size_t>(decoded.size()));
    if (!host::local_decoded_path(path) || !img.load(QString::fromUtf8(decoded))) return {};
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
  } else if (src.encoded.empty() ||
             !img.loadFromData(src.encoded.data(), static_cast<int>(std::min<size_t>(src.encoded.size(), INT32_MAX)))) {
    return {};
  }
  return img.isNull() ? QImage() : img.convertToFormat(QImage::Format_RGBA8888_Premultiplied);
}

NveQtImageCache::NveQtImageCache(size_t max_entries) : max_entries_(std::max<size_t>(1u, max_entries)) {}

QImage NveQtImageCache::get(const host::ImageSource& src) {
  if (src.url.empty() && src.encoded.empty()) return {};
  std::string key = host::image_cache_key(src);
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
