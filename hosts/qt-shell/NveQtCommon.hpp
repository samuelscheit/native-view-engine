#pragma once

#include "nve_draw_list.hpp"
#include "nve_hash.generated.hpp"
#include "nve_host_runtime.hpp"

#include <QAccessible>
#include <QByteArray>
#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QImage>
#include <QPaintDevice>
#include <QPainter>
#include <QPainterPath>
#include <QPen>
#include <QPixmap>
#include <QPointF>
#include <QRectF>
#include <QSizeF>
#include <QString>
#include <QUrl>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <list>
#include <mutex>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve::qt {

/// Canonical cross-platform accessibility roles inferred from `NveRenderNode` and `raw_props`.
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

/// Image content-fit mode matching React Native's `ImageResizeMode`.
enum class ImageResizeMode : uint8_t {
  Cover = 0,
  Contain,
  Stretch,
  Center,
};

/// Normalized accessibility & TextInput metadata extracted from `raw_props` + `NveRenderNode`.
struct NodeSemantics {
  A11yRole role{A11yRole::Group};
  QAccessible::Role accessibleRole{QAccessible::Grouping};
  bool explicitRole{false};
  QString label;
  bool hasExplicitLabel{false};
  QString hint;
  QString value;
  bool hasExplicitValue{false};
  bool disabled{false};
  QString testId;
  QString placeholder;
  bool secureTextEntry{false};
  bool multiline{false};

  bool operator==(const NodeSemantics& other) const noexcept = default;
  bool operator!=(const NodeSemantics& other) const noexcept = default;
};

inline QAccessible::Role toQAccessibleRole(A11yRole role) noexcept {
  switch (role) {
    case A11yRole::Button:
      return QAccessible::Button;
    case A11yRole::Text:
      return QAccessible::StaticText;
    case A11yRole::Header:
      return QAccessible::Heading;
    case A11yRole::TextBox:
      return QAccessible::EditableText;
    case A11yRole::ScrollArea:
      return QAccessible::Pane;
    case A11yRole::Image:
      return QAccessible::Graphic;
    case A11yRole::List:
      return QAccessible::List;
    case A11yRole::ListItem:
      return QAccessible::ListItem;
    case A11yRole::CheckBox:
      return QAccessible::CheckBox;
    case A11yRole::Link:
      return QAccessible::Link;
    case A11yRole::Group:
      return QAccessible::Grouping;
    case A11yRole::None:
      return QAccessible::NoRole;
  }
  return QAccessible::Grouping;
}

namespace detail {

inline std::string asciiLower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

inline QString qstringFromUtf8(std::string_view sv) {
  return QString::fromUtf8(sv.data(), static_cast<qsizetype>(sv.size()));
}

inline bool readRawString(const nve::RawPropMap& props, const char* key, std::string& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  return false;
}

inline bool readRawValueAsString(const nve::RawPropMap& props, const char* key, std::string& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  if (const auto* i = std::get_if<int64_t>(&it->second)) {
    out = std::to_string(*i);
    return true;
  }
  if (const auto* d = std::get_if<double>(&it->second)) {
    if (std::isfinite(*d)) {
      out = std::to_string(*d);
      return true;
    }
    return false;
  }
  if (const auto* b = std::get_if<bool>(&it->second)) {
    out = *b ? "true" : "false";
    return true;
  }
  return false;
}

inline bool readRawBool(const nve::RawPropMap& props, const char* key, bool& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* b = std::get_if<bool>(&it->second)) {
    out = *b;
    return true;
  }
  if (const auto* i = std::get_if<int64_t>(&it->second)) {
    out = (*i != 0);
    return true;
  }
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    const std::string lower = asciiLower(*s);
    if (lower == "true" || lower == "1" || lower == "yes") {
      out = true;
      return true;
    }
    if (lower == "false" || lower == "0" || lower == "no") {
      out = false;
      return true;
    }
  }
  return false;
}

inline bool parseA11yRole(std::string_view raw, A11yRole& out) {
  const std::string lower = asciiLower(raw);
  if (lower == "button") {
    out = A11yRole::Button;
    return true;
  }
  if (lower == "text" || lower == "statictext" || lower == "label") {
    out = A11yRole::Text;
    return true;
  }
  if (lower == "header" || lower == "heading") {
    out = A11yRole::Header;
    return true;
  }
  if (lower == "textbox" || lower == "textinput" || lower == "search" || lower == "searchbox") {
    out = A11yRole::TextBox;
    return true;
  }
  if (lower == "scrollarea" || lower == "scrollview") {
    out = A11yRole::ScrollArea;
    return true;
  }
  if (lower == "image" || lower == "img") {
    out = A11yRole::Image;
    return true;
  }
  if (lower == "list") {
    out = A11yRole::List;
    return true;
  }
  if (lower == "listitem") {
    out = A11yRole::ListItem;
    return true;
  }
  if (lower == "checkbox" || lower == "switch") {
    out = A11yRole::CheckBox;
    return true;
  }
  if (lower == "link") {
    out = A11yRole::Link;
    return true;
  }
  if (lower == "group" || lower == "region") {
    out = A11yRole::Group;
    return true;
  }
  if (lower == "none" || lower == "presentation") {
    out = A11yRole::None;
    return true;
  }
  return false;
}

inline A11yRole inferDefaultRole(const NveRenderNode& node) noexcept {
  if (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u) {
    return A11yRole::Button;
  }
  switch (node.kind) {
    case NVE_NODE_KIND_TEXT:
      return A11yRole::Text;
    case NVE_NODE_KIND_TEXT_INPUT:
      return A11yRole::TextBox;
    case NVE_NODE_KIND_SCROLL_VIEW:
      return A11yRole::ScrollArea;
    case NVE_NODE_KIND_IMAGE:
      return A11yRole::Image;
    default:
      return A11yRole::Group;
  }
}

} // namespace detail

/// Read `RenderedNodeSnapshot::raw_props` directly without any command-buffer decode fallback.
inline std::unordered_map<uint32_t, nve::RawPropMap> extractRawPropsByNode(
    const std::vector<nve::RenderedNodeSnapshot>& snapshots) {
  std::unordered_map<uint32_t, nve::RawPropMap> result;
  for (const auto& snap : snapshots) {
    if (!snap.raw_props.empty()) {
      result.emplace(snap.node_id, snap.raw_props);
    }
  }
  return result;
}

/// Parse `ImageResizeMode` from a UTF-8 string (`"cover"`, `"contain"`, `"stretch"`, `"center"`).
inline ImageResizeMode parseImageResizeMode(std::string_view raw) {
  const std::string lower = detail::asciiLower(raw);
  if (lower == "contain") {
    return ImageResizeMode::Contain;
  }
  if (lower == "stretch" || lower == "fill") {
    return ImageResizeMode::Stretch;
  }
  if (lower == "center") {
    return ImageResizeMode::Center;
  }
  return ImageResizeMode::Cover;
}

inline ImageResizeMode extractImageResizeMode(const nve::RawPropMap* raw_props) {
  if (raw_props == nullptr) {
    return ImageResizeMode::Cover;
  }
  std::string mode_str;
  if (detail::readRawString(*raw_props, "resizeMode", mode_str) ||
      detail::readRawString(*raw_props, "resize_mode", mode_str)) {
    return parseImageResizeMode(mode_str);
  }
  return ImageResizeMode::Cover;
}

/// Compute target destination rectangle and image-pixel source crop rectangle for all 4 resize modes.
inline bool computeImageDrawRects(
    ImageResizeMode mode,
    const QRectF& dstBounds,
    const QSizeF& imgSize,
    QRectF& outDstRect,
    QRectF& outSrcRect) noexcept {
  const qreal dw = dstBounds.width();
  const qreal dh = dstBounds.height();
  const qreal iw = imgSize.width();
  const qreal ih = imgSize.height();
  if (dw <= 0.0 || dh <= 0.0 || iw <= 0.0 || ih <= 0.0) {
    return false;
  }

  switch (mode) {
    case ImageResizeMode::Stretch:
      outDstRect = dstBounds;
      outSrcRect = QRectF(0.0, 0.0, iw, ih);
      return true;
    case ImageResizeMode::Contain: {
      const qreal scale = std::min(dw / iw, dh / ih);
      const qreal tw = iw * scale;
      const qreal th = ih * scale;
      outDstRect = QRectF(
          dstBounds.x() + (dw - tw) * 0.5,
          dstBounds.y() + (dh - th) * 0.5,
          tw,
          th);
      outSrcRect = QRectF(0.0, 0.0, iw, ih);
      return true;
    }
    case ImageResizeMode::Center: {
      const qreal drawW = std::min(dw, iw);
      const qreal drawH = std::min(dh, ih);
      outDstRect = QRectF(
          dstBounds.x() + (dw - drawW) * 0.5,
          dstBounds.y() + (dh - drawH) * 0.5,
          drawW,
          drawH);
      outSrcRect = QRectF(
          (iw - drawW) * 0.5,
          (ih - drawH) * 0.5,
          drawW,
          drawH);
      return true;
    }
    case ImageResizeMode::Cover:
    default: {
      const qreal scale = std::max(dw / iw, dh / ih);
      const qreal cropW = std::min(iw, dw / scale);
      const qreal cropH = std::min(ih, dh / scale);
      outDstRect = dstBounds;
      outSrcRect = QRectF(
          (iw - cropW) * 0.5,
          (ih - cropH) * 0.5,
          cropW,
          cropH);
      return true;
    }
  }
}

/// Extract accessibility & TextInput metadata directly from `NveRenderNode` + optional `raw_props`.
inline NodeSemantics extractNodeSemantics(
    const NveRenderNode& node,
    std::string_view text,
    const nve::RawPropMap* raw_props) {
  NodeSemantics sem{};
  sem.role = detail::inferDefaultRole(node);

  if (raw_props != nullptr) {
    std::string role_str;
    if (detail::readRawString(*raw_props, "accessibilityRole", role_str) ||
        detail::readRawString(*raw_props, "role", role_str)) {
      A11yRole parsed = sem.role;
      if (detail::parseA11yRole(role_str, parsed)) {
        sem.role = parsed;
        sem.explicitRole = true;
      }
    }

    std::string label_str;
    if (detail::readRawString(*raw_props, "accessibilityLabel", label_str) ||
        detail::readRawString(*raw_props, "aria-label", label_str)) {
      sem.label = detail::qstringFromUtf8(label_str);
      sem.hasExplicitLabel = !sem.label.isEmpty();
    }

    std::string hint_str;
    if (detail::readRawString(*raw_props, "accessibilityHint", hint_str)) {
      sem.hint = detail::qstringFromUtf8(hint_str);
    }

    std::string value_str;
    if (detail::readRawValueAsString(*raw_props, "accessibilityValue", value_str)) {
      sem.value = detail::qstringFromUtf8(value_str);
      sem.hasExplicitValue = true;
    }

    if (!detail::readRawBool(*raw_props, "accessibilityDisabled", sem.disabled)) {
      (void)detail::readRawBool(*raw_props, "disabled", sem.disabled);
    }

    std::string test_id_str;
    if (detail::readRawString(*raw_props, "testID", test_id_str) ||
        detail::readRawString(*raw_props, "data-testid", test_id_str)) {
      sem.testId = detail::qstringFromUtf8(test_id_str);
    }

    std::string placeholder_str;
    if (detail::readRawString(*raw_props, "placeholder", placeholder_str)) {
      sem.placeholder = detail::qstringFromUtf8(placeholder_str);
    }

    (void)detail::readRawBool(*raw_props, "secureTextEntry", sem.secureTextEntry);
    (void)detail::readRawBool(*raw_props, "multiline", sem.multiline);
  }

  if (!sem.hasExplicitLabel && (node.flags & NVE_NODE_LEAF_TEXT) != 0u && !text.empty()) {
    sem.label = detail::qstringFromUtf8(text);
  }

  sem.accessibleRole = toQAccessibleRole(sem.role);
  return sem;
}

/// Shared bounded-LRU `QImage` / `QPixmap` decoder & cache supporting `data:image/...;base64,...`
/// and `file://...` URIs across both `NveQtWidget` and `NveQtQuickItem`.
class NveQtImageCache {
 public:
  explicit NveQtImageCache(size_t max_entries = 64u)
      : max_entries_(std::max<size_t>(1u, max_entries)) {}

  static NveQtImageCache& shared() {
    static NveQtImageCache instance(64u);
    return instance;
  }

  /// Decode `uri` directly into a premultiplied RGBA `QImage` without caching.
  static QImage decodeUri(std::string_view uri) {
    if (uri.empty()) {
      return {};
    }

    QImage img;
    if (uri.size() > 11u && detail::asciiLower(uri.substr(0, 11)) == "data:image/") {
      const size_t marker = uri.find(";base64,");
      if (marker == std::string_view::npos) {
        return {};
      }
      const std::string_view b64 = uri.substr(marker + 8u);
      if (b64.empty()) {
        return {};
      }
      const QByteArray raw = QByteArray::fromBase64(
          QByteArray::fromRawData(b64.data(), static_cast<qsizetype>(b64.size())));
      if (raw.isEmpty() || !img.loadFromData(raw)) {
        return {};
      }
    } else if (uri.size() > 7u && detail::asciiLower(uri.substr(0, 7)) == "file://") {
      const QString quri = detail::qstringFromUtf8(uri);
      QString local_path = QUrl(quri).toLocalFile();
      if (local_path.isEmpty()) {
        local_path = detail::qstringFromUtf8(uri.substr(7u));
      }
      if (local_path.isEmpty() || !img.load(local_path)) {
        return {};
      }
    } else {
      return {};
    }

    if (img.format() != QImage::Format_RGBA8888_Premultiplied) {
      img = img.convertToFormat(QImage::Format_RGBA8888_Premultiplied);
    }
    return img;
  }

  /// Return a cached premultiplied `QImage` (decoding & inserting into the LRU on miss).
  QImage getImage(std::string_view uri) {
    if (uri.empty()) {
      return {};
    }
    std::lock_guard<std::mutex> lock(mutex_);
    const std::string key(uri);
    auto it = entries_.find(key);
    if (it != entries_.end()) {
      touchLocked(it->second);
      ++hits_;
      return it->second.image;
    }

    ++misses_;
    QImage decoded = decodeUri(uri);
    if (decoded.isNull()) {
      return {};
    }
    Entry& inserted = insertLocked(key, std::move(decoded));
    return inserted.image;
  }

  /// Return a cached `QPixmap` (materialized lazily from the cached `QImage` on the GUI thread).
  QPixmap getPixmap(std::string_view uri) {
    if (uri.empty()) {
      return {};
    }
    std::lock_guard<std::mutex> lock(mutex_);
    const std::string key(uri);
    auto it = entries_.find(key);
    if (it != entries_.end()) {
      touchLocked(it->second);
      ++hits_;
      if (it->second.pixmap.isNull() && !it->second.image.isNull()) {
        it->second.pixmap = QPixmap::fromImage(it->second.image);
      }
      return it->second.pixmap;
    }

    ++misses_;
    QImage decoded = decodeUri(uri);
    if (decoded.isNull()) {
      return {};
    }
    Entry& inserted = insertLocked(key, std::move(decoded));
    inserted.pixmap = QPixmap::fromImage(inserted.image);
    return inserted.pixmap;
  }

  size_t size() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return entries_.size();
  }

  size_t capacity() const noexcept { return max_entries_; }

  uint64_t hits() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return hits_;
  }

  uint64_t misses() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return misses_;
  }

  void clear() {
    std::lock_guard<std::mutex> lock(mutex_);
    lru_.clear();
    entries_.clear();
  }

 private:
  struct Entry {
    QImage image;
    QPixmap pixmap;
    std::list<std::string>::iterator lru_it;
  };

  void touchLocked(Entry& entry) {
    lru_.splice(lru_.begin(), lru_, entry.lru_it);
    entry.lru_it = lru_.begin();
  }

  Entry& insertLocked(const std::string& key, QImage img) {
    while (entries_.size() >= max_entries_ && !lru_.empty()) {
      const std::string& victim = lru_.back();
      entries_.erase(victim);
      lru_.pop_back();
    }
    lru_.push_front(key);
    Entry entry{};
    entry.image = std::move(img);
    entry.lru_it = lru_.begin();
    auto [it, _] = entries_.emplace(key, std::move(entry));
    return it->second;
  }

  size_t max_entries_{64u};
  mutable std::mutex mutex_;
  std::list<std::string> lru_;
  std::unordered_map<std::string, Entry> entries_;
  uint64_t hits_{0};
  uint64_t misses_{0};
};

inline QColor qcolorFromRgba(uint32_t rgba) noexcept {
  const auto c = nve::unpack_rgba(rgba);
  return QColor(c.r8, c.g8, c.b8, c.a8);
}

inline QColor qcolorFromDrawColor(nve::DrawColorRgba c) noexcept {
  return QColor(c.r, c.g, c.b, c.a);
}

/// Stream-decode and paint all 8 `NVE_DRAW1` opcodes (`Clear`, `FillRect`, `FillRoundedRect`,
/// `FillCircle`, `StrokeLine`, `StrokePolyline`, `ClipRect`, `RestoreClip`) onto `painter`
/// inside `bounds`. Validates the stream first and guarantees balanced `painter.save()` /
/// `painter.restore()` even on corrupt payloads or unbalanced clip stacks.
inline nve::DrawListDecodeResult renderDrawListToPainter(
    QPainter& painter,
    QRectF bounds,
    std::span<const uint8_t> bytes) {
  if (bytes.empty()) {
    return nve::DrawListDecodeResult{nve::DrawListError::Ok, 0u, 0u};
  }
  const nve::DrawListDecodeResult check = nve::validate_draw_list(bytes);
  if (!check.ok()) {
    return check;
  }

  const qreal bw = std::max<qreal>(0.0, bounds.width());
  const qreal bh = std::max<qreal>(0.0, bounds.height());
  if (bw <= 0.0 || bh <= 0.0) {
    return nve::DrawListDecodeResult{nve::DrawListError::Ok, 0u, bytes.size()};
  }

  const QRectF local_bounds(0.0, 0.0, bw, bh);
  const bool is_image_device =
      (painter.device() != nullptr && painter.device()->devType() == QInternal::Image);

  painter.save();
  painter.setRenderHint(QPainter::Antialiasing, true);
  if (bounds.x() != 0.0 || bounds.y() != 0.0) {
    painter.translate(bounds.topLeft());
  }
  painter.setClipRect(local_bounds, Qt::IntersectClip);

  struct QPainterDrawVisitor {
    QPainter& p;
    QRectF local_rect;
    bool source_clear;
    uint32_t clip_depth{0u};

    void on_clear(const nve::DrawClearOp& op) {
      if (source_clear) {
        const auto prev_mode = p.compositionMode();
        p.setCompositionMode(QPainter::CompositionMode_Source);
        p.fillRect(local_rect, qcolorFromDrawColor(op.color));
        p.setCompositionMode(prev_mode);
      } else if (op.color.a > 0u) {
        p.fillRect(local_rect, qcolorFromDrawColor(op.color));
      }
    }

    void on_fill_rect(const nve::DrawFillRectOp& op) {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
        return;
      }
      p.fillRect(
          QRectF(
              static_cast<qreal>(op.x),
              static_cast<qreal>(op.y),
              static_cast<qreal>(op.w),
              static_cast<qreal>(op.h)),
          qcolorFromDrawColor(op.color));
    }

    void on_fill_rounded_rect(const nve::DrawFillRoundedRectOp& op) {
      if (op.w <= 0.0f || op.h <= 0.0f || op.color.a == 0u) {
        return;
      }
      const QRectF rc(
          static_cast<qreal>(op.x),
          static_cast<qreal>(op.y),
          static_cast<qreal>(op.w),
          static_cast<qreal>(op.h));
      const qreal max_r = std::min(rc.width(), rc.height()) * 0.5;
      const qreal r = std::clamp(static_cast<qreal>(op.radius), 0.0, max_r);
      if (r <= 0.0) {
        p.fillRect(rc, qcolorFromDrawColor(op.color));
        return;
      }
      QPainterPath path;
      path.addRoundedRect(rc, r, r);
      p.fillPath(path, qcolorFromDrawColor(op.color));
    }

    void on_fill_circle(const nve::DrawFillCircleOp& op) {
      if (op.radius <= 0.0f || op.color.a == 0u) {
        return;
      }
      p.setPen(Qt::NoPen);
      p.setBrush(qcolorFromDrawColor(op.color));
      const qreal r = static_cast<qreal>(op.radius);
      p.drawEllipse(QPointF(static_cast<qreal>(op.cx), static_cast<qreal>(op.cy)), r, r);
    }

    void on_stroke_line(const nve::DrawStrokeLineOp& op) {
      if (op.width <= 0.0f || op.color.a == 0u) {
        return;
      }
      QPen pen(
          qcolorFromDrawColor(op.color),
          static_cast<qreal>(op.width),
          Qt::SolidLine,
          Qt::RoundCap,
          Qt::RoundJoin);
      p.setPen(pen);
      p.setBrush(Qt::NoBrush);
      p.drawLine(
          QPointF(static_cast<qreal>(op.x0), static_cast<qreal>(op.y0)),
          QPointF(static_cast<qreal>(op.x1), static_cast<qreal>(op.y1)));
    }

    void on_stroke_polyline(const nve::DrawStrokePolylineOp& op) {
      if (op.points.size() < 2u || op.width <= 0.0f || op.color.a == 0u) {
        return;
      }
      QPen pen(
          qcolorFromDrawColor(op.color),
          static_cast<qreal>(op.width),
          Qt::SolidLine,
          Qt::RoundCap,
          Qt::RoundJoin);
      p.setPen(pen);
      p.setBrush(Qt::NoBrush);
      QPainterPath path;
      const nve::DrawPoint p0 = op.points[0u];
      path.moveTo(static_cast<qreal>(p0.x), static_cast<qreal>(p0.y));
      for (size_t i = 1u; i < op.points.size(); ++i) {
        const nve::DrawPoint pt = op.points[i];
        path.lineTo(static_cast<qreal>(pt.x), static_cast<qreal>(pt.y));
      }
      p.drawPath(path);
    }

    void on_clip_rect(const nve::DrawClipRectOp& op) {
      p.save();
      ++clip_depth;
      const qreal cw = (op.w > 0.0f) ? static_cast<qreal>(op.w) : 0.0;
      const qreal ch = (op.h > 0.0f) ? static_cast<qreal>(op.h) : 0.0;
      p.setClipRect(
          QRectF(static_cast<qreal>(op.x), static_cast<qreal>(op.y), cw, ch),
          Qt::IntersectClip);
    }

    void on_restore_clip(const nve::DrawRestoreClipOp&) {
      if (clip_depth > 0u) {
        p.restore();
        --clip_depth;
      }
    }
  };

  QPainterDrawVisitor visitor{painter, local_bounds, is_image_device, 0u};
  const nve::DrawListDecodeResult res = nve::decode_draw_list(bytes, visitor);
  while (visitor.clip_depth > 0u) {
    painter.restore();
    --visitor.clip_depth;
  }
  painter.restore();
  return res;
}

inline QFont makeNveFont(float font_size) {
  QFont font;
  if (QGuiApplication::instance() != nullptr) {
    font = QGuiApplication::font();
  }
  const int px =
      std::max(1, static_cast<int>(std::lround(font_size > 0.0f ? font_size : 16.0f)));
  font.setPixelSize(px);
  return font;
}

inline nve::TextMeasurerCallback makeSharedQtTextMeasurer() {
  return [](std::string_view text, float font_size, float max_width) -> NveIntrinsicSize {
    if (text.empty()) {
      return NveIntrinsicSize{0.0f, 0.0f};
    }
    const QString qtext = QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size()));
    const QFont font = makeNveFont(font_size);
    const QFontMetricsF fm(font);
    const bool constrained = (max_width > 0.0f && std::isfinite(max_width));
    const qreal constraint_w = constrained ? static_cast<qreal>(max_width) : 1.0e6;
    const QRectF bounds = fm.boundingRect(
        QRectF(0.0, 0.0, constraint_w, 1.0e6),
        Qt::AlignLeft | Qt::AlignTop | Qt::TextWordWrap,
        qtext);

    qreal w = bounds.width();
    qreal h = std::max(bounds.height(), fm.height());

    if (!qtext.contains(QChar('\n'))) {
      const qreal adv = fm.horizontalAdvance(qtext);
      if (!constrained || adv <= constraint_w) {
        w = std::max(w, adv);
      }
    }

    // Add 2px horizontal slack so word-wrapped labels and QTextLayout never wrap the final glyph.
    qreal final_w = std::ceil(w) + 2.0;
    if (constrained) {
      final_w = std::min(final_w, std::ceil(constraint_w));
    }
    return NveIntrinsicSize{
        static_cast<float>(final_w),
        static_cast<float>(std::ceil(h)),
    };
  };
}

} // namespace nve::qt
