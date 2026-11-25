#pragma once

#include "nve_host_runtime.hpp"

#include <QAccessible>
#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QRectF>
#include <QString>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace nve::qt {

/// Canonical cross-platform accessibility roles inferred from `RenderedNodeSnapshot` and raw props.
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

/// Normalized accessibility & TextInput metadata extracted from `raw_props` + node kind/event_mask.
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

inline A11yRole inferDefaultRole(const nve::RenderedNodeSnapshot& snap) noexcept {
  if (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & nve::kEventMaskPress) != 0u) {
    return A11yRole::Button;
  }
  switch (snap.kind) {
    case NVE_COMP_TEXT:
      return A11yRole::Text;
    case NVE_COMP_TEXT_INPUT:
      return A11yRole::TextBox;
    case NVE_COMP_SCROLL_VIEW:
      return A11yRole::ScrollArea;
    case NVE_COMP_IMAGE:
      return A11yRole::Image;
    default:
      return A11yRole::Group;
  }
}

inline void decodeRawPropsFromCommandBuffer(
    const uint8_t* data,
    size_t len,
    std::unordered_map<uint32_t, nve::RawPropMap>& out) {
  if (data == nullptr || len < 24u) {
    return;
  }
  size_t pos = 0;
  auto read_pod = [&](auto& v) -> bool {
    if (sizeof(v) > len - pos) return false;
    std::memcpy(&v, data + pos, sizeof(v));
    pos += sizeof(v);
    return true;
  };
  auto skip = [&](size_t n) -> bool {
    if (n > len - pos) return false;
    pos += n;
    return true;
  };

  uint32_t magic = 0;
  uint16_t version = 0;
  uint16_t flags = 0;
  uint32_t surface_id = 0;
  uint64_t frame_id = 0;
  uint32_t op_count = 0;
  if (!read_pod(magic) || !read_pod(version) || !read_pod(flags) || !read_pod(surface_id) ||
      !read_pod(frame_id) || !read_pod(op_count)) {
    return;
  }
  if (magic != NVE_ABI_MAGIC || version != NVE_ABI_VERSION) {
    return;
  }

  for (uint32_t i = 0; i < op_count; ++i) {
    uint8_t op = 0;
    if (!read_pod(op)) return;
    switch (static_cast<NveOpcode>(op)) {
      case NVE_OP_CREATE_NODE: {
        uint16_t name_len = 0;
        if (!skip(6u) || !read_pod(name_len) || !skip(name_len)) return;
        break;
      }
      case NVE_OP_REMOVE_NODE:
        if (!skip(4u)) return;
        break;
      case NVE_OP_SET_CHILDREN: {
        uint32_t count = 0;
        if (!skip(4u) || !read_pod(count)) return;
        if (static_cast<size_t>(count) > (len - pos) / sizeof(uint32_t)) return;
        pos += static_cast<size_t>(count) * sizeof(uint32_t);
        break;
      }
      case NVE_OP_SET_TEXT: {
        uint32_t tlen = 0;
        if (!skip(4u) || !read_pod(tlen) || !skip(tlen)) return;
        break;
      }
      case NVE_OP_SET_LAYOUT:
        if (!skip(81u)) return;
        break;
      case NVE_OP_SET_STYLE:
        if (!skip(36u)) return;
        break;
      case NVE_OP_SET_RAW_PROP: {
        uint32_t node_id = 0;
        uint16_t key_len = 0;
        if (!read_pod(node_id) || !read_pod(key_len) || key_len > len - pos) return;
        std::string key(reinterpret_cast<const char*>(data + pos), key_len);
        pos += key_len;
        uint8_t tag = 0;
        if (!read_pod(tag)) return;
        nve::RawPropValue val{nullptr};
        switch (tag) {
          case 0:
            val = nullptr;
            break;
          case 1: {
            uint8_t b = 0;
            if (!read_pod(b)) return;
            val = (b != 0);
            break;
          }
          case 2: {
            int64_t v = 0;
            if (!read_pod(v)) return;
            val = v;
            break;
          }
          case 3: {
            double v = 0.0;
            if (!read_pod(v)) return;
            val = v;
            break;
          }
          case 4: {
            uint32_t vlen = 0;
            if (!read_pod(vlen) || vlen > len - pos) return;
            val = std::string(reinterpret_cast<const char*>(data + pos), vlen);
            pos += vlen;
            break;
          }
          case 5: {
            uint32_t vlen = 0;
            if (!read_pod(vlen) || vlen > len - pos) return;
            val = std::vector<uint8_t>(data + pos, data + pos + vlen);
            pos += vlen;
            break;
          }
          default:
            return;
        }
        out[node_id][std::move(key)] = std::move(val);
        break;
      }
      case NVE_OP_SET_EVENT_MASK:
        if (!skip(12u)) return;
        break;
      case NVE_OP_DISPATCH_COMMAND: {
        uint16_t nlen = 0;
        uint32_t plen = 0;
        if (!skip(4u) || !read_pod(nlen) || !skip(nlen) || !read_pod(plen) || !skip(plen)) return;
        break;
      }
      case NVE_OP_COMMIT:
        if (!skip(1u)) return;
        break;
      default:
        return;
    }
  }
}

template <typename Snap>
inline std::unordered_map<uint32_t, nve::RawPropMap> extractRawPropsImpl(
    NveFabricHostHandle host,
    const std::vector<Snap>& snapshots) {
  std::unordered_map<uint32_t, nve::RawPropMap> result;
  if constexpr (requires(const Snap& s) { s.raw_props; }) {
    for (const auto& snap : snapshots) {
      if (!snap.raw_props.empty()) {
        result.emplace(snap.node_id, snap.raw_props);
      }
    }
  } else {
    (void)snapshots;
    if (host == nullptr) {
      return result;
    }
    const std::vector<uint8_t> frame0 = nve::fabric_host_serialize_frame0(host, 1u);
    decodeRawPropsFromCommandBuffer(frame0.data(), frame0.size(), result);
  }
  return result;
}

} // namespace detail

inline std::unordered_map<uint32_t, nve::RawPropMap> extractRawPropsByNode(
    NveFabricHostHandle host,
    const std::vector<nve::RenderedNodeSnapshot>& snapshots) {
  return detail::extractRawPropsImpl(host, snapshots);
}

inline NodeSemantics extractNodeSemantics(
    const nve::RenderedNodeSnapshot& snap,
    const nve::RawPropMap* raw_props) {
  NodeSemantics sem{};
  sem.role = detail::inferDefaultRole(snap);

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

  if (!sem.hasExplicitLabel && snap.kind == NVE_COMP_TEXT && snap.children.empty() &&
      !snap.text.empty()) {
    sem.label = detail::qstringFromUtf8(snap.text);
  }

  sem.accessibleRole = toQAccessibleRole(sem.role);
  return sem;
}

inline QColor qcolorFromRgba(uint32_t rgba) noexcept {
  const auto c = nve::unpack_rgba(rgba);
  return QColor(c.r8, c.g8, c.b8, c.a8);
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
