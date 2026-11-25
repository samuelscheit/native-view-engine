#include "NveQtTextCache.hpp"

#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QPainter>
#include <QPointF>
#include <QRectF>
#include <QTextLayout>
#include <QTextOption>

#include <algorithm>
#include <cmath>

namespace nve::qt {

void popUtf8Char(std::string& s) {
  while (!s.empty()) {
    const unsigned char c = static_cast<unsigned char>(s.back());
    s.pop_back();
    if ((c & 0xC0u) != 0x80u) {
      break;
    }
  }
}

void resolveTextInputDisplay(
    std::string& io_text,
    uint32_t& io_text_rgba,
    const NodeSemantics* sem,
    const QString& preedit) {
  if (sem != nullptr && sem->secureTextEntry && !io_text.empty()) {
    const QString qplain = QString::fromUtf8(io_text.data(), static_cast<qsizetype>(io_text.size()));
    QString masked;
    masked.fill(QChar(0x2022), qplain.size());
    const QByteArray utf8 = masked.toUtf8();
    io_text.assign(utf8.constData(), static_cast<size_t>(utf8.size()));
  }
  if (!preedit.isEmpty()) {
    const QByteArray pre_utf8 = preedit.toUtf8();
    io_text.append(pre_utf8.constData(), static_cast<size_t>(pre_utf8.size()));
  } else if (io_text.empty() && sem != nullptr && !sem->placeholder.isEmpty()) {
    const QByteArray ph_utf8 = sem->placeholder.toUtf8();
    io_text.assign(ph_utf8.constData(), static_cast<size_t>(ph_utf8.size()));
    if (io_text_rgba != 0u) {
      const uint32_t a = ((io_text_rgba >> 24) & 0xFFu) / 2u;
      io_text_rgba = (io_text_rgba & 0x00FFFFFFu) | (a << 24);
    } else {
      io_text_rgba = 0x8094A3B8u;
    }
  }
}

QImage rasterizeTextImage(
    const std::string& text,
    float font_size,
    uint32_t text_rgba,
    float layout_w,
    float layout_h,
    qreal dpr,
    bool is_text_input,
    bool focused,
    QSizeF& out_logical_size) {
  const qreal eff_dpr = (dpr > 0.0 && std::isfinite(dpr)) ? dpr : 1.0;
  const QFont font = makeNveFont(font_size);
  const QFontMetricsF fm(font);

  const qreal inset_x = is_text_input ? 4.0 : 0.0;
  const qreal logical_w = std::max<qreal>(1.0, std::ceil(static_cast<qreal>(layout_w)));
  const qreal logical_h = std::max<qreal>(
      std::ceil(fm.height()),
      std::ceil(static_cast<qreal>(layout_h)));
  out_logical_size = QSizeF(logical_w, logical_h);

  const int img_w = std::max(1, static_cast<int>(std::ceil(logical_w * eff_dpr)));
  const int img_h = std::max(1, static_cast<int>(std::ceil(logical_h * eff_dpr)));

  QImage img(img_w, img_h, QImage::Format_RGBA8888_Premultiplied);
  img.setDevicePixelRatio(eff_dpr);
  img.fill(Qt::transparent);

  const QString qtext = QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size()));
  const qreal wrap_width = std::max<qreal>(1.0, logical_w - 2.0 * inset_x);

  QTextLayout layout(qtext, font);
  QTextOption opt;
  opt.setAlignment(Qt::AlignLeft);
  opt.setWrapMode(
      is_text_input ? QTextOption::NoWrap : QTextOption::WrapAtWordBoundaryOrAnywhere);
  layout.setTextOption(opt);

  qreal text_height = 0.0;
  qreal last_line_width = 0.0;
  qreal last_line_y = 0.0;
  qreal last_line_h = fm.height();

  layout.beginLayout();
  while (true) {
    QTextLine line = layout.createLine();
    if (!line.isValid()) {
      break;
    }
    line.setLineWidth(wrap_width);
    line.setPosition(QPointF(inset_x, text_height));
    last_line_width = line.naturalTextWidth();
    last_line_y = text_height;
    last_line_h = line.height();
    text_height += line.height();
  }
  layout.endLayout();

  const qreal y_offset = std::max<qreal>(0.0, (logical_h - text_height) * 0.5);

  QPainter painter(&img);
  painter.setRenderHint(QPainter::Antialiasing, true);
  painter.setRenderHint(QPainter::TextAntialiasing, true);
  painter.setFont(font);
  QColor fg = qcolorFromRgba(text_rgba);
  if (fg.alpha() == 0 && is_text_input) {
    fg = QColor(255, 255, 255, 255);
  }
  painter.setPen(fg);
  layout.draw(&painter, QPointF(0.0, y_offset));

  if (is_text_input && focused) {
    const qreal caret_x = std::min(logical_w - 2.0, inset_x + last_line_width + 1.0);
    const qreal caret_y = y_offset + last_line_y + 1.0;
    const qreal caret_h = std::max<qreal>(1.0, last_line_h - 2.0);
    painter.fillRect(QRectF(caret_x, caret_y, 1.5, caret_h), fg);
  }
  painter.end();
  return img;
}

QSGTexture* NveQtTextCache::acquireTextTexture(
    uint32_t node_id,
    const std::string& text,
    float font_size,
    uint32_t text_rgba,
    float layout_w,
    float layout_h,
    qreal dpr,
    int dpr_centis,
    bool is_text_input,
    bool focused,
    uint64_t scene_generation,
    QQuickWindow* win,
    QSizeF& out_logical_size,
    bool& out_rasterized) {
  const int qw = std::max(1, static_cast<int>(std::lround(layout_w * 2.0f)));
  const int qh = std::max(1, static_cast<int>(std::lround(layout_h * 2.0f)));

  auto& entry = text_cache_[node_id];
  const bool hit = entry.texture_generation != 0u &&
                   (win == nullptr || entry.texture != nullptr) &&
                   entry.matches(
                       text,
                       font_size,
                       text_rgba,
                       qw,
                       qh,
                       dpr_centis,
                       is_text_input,
                       focused);
  if (hit) {
    entry.last_used_generation = scene_generation;
    out_logical_size = entry.logical_size;
    out_rasterized = false;
    return entry.texture.get();
  }

  QSizeF logical_size;
  const QImage img = rasterizeTextImage(
      text,
      font_size,
      text_rgba,
      layout_w,
      layout_h,
      dpr,
      is_text_input,
      focused,
      logical_size);

  entry.text = text;
  entry.font_size = font_size;
  entry.text_rgba = text_rgba;
  entry.quant_w = qw;
  entry.quant_h = qh;
  entry.quant_dpr = dpr_centis;
  entry.is_text_input = is_text_input;
  entry.focused = focused;
  entry.logical_size = logical_size;
  entry.last_used_generation = scene_generation;
  ++entry.texture_generation;
  ++total_rasterizations_;

  if (win != nullptr) {
    entry.texture.reset(
        win->createTextureFromImage(img, QQuickWindow::TextureHasAlphaChannel));
  } else {
    entry.texture.reset();
  }

  out_logical_size = entry.logical_size;
  out_rasterized = true;
  return entry.texture.get();
}

QSGTexture* NveQtTextCache::acquireImageTexture(
    std::string_view uri,
    uint64_t scene_generation,
    QQuickWindow* win,
    QSizeF& out_image_size) {
  if (uri.empty()) {
    return nullptr;
  }
  const std::string key(uri);
  auto it = image_cache_.find(key);
  if (it != image_cache_.end()) {
    it->second.last_used_generation = scene_generation;
    out_image_size = it->second.image_size;
    if (win != nullptr && it->second.texture == nullptr) {
      const QImage img = NveQtImageCache::shared().getImage(uri);
      if (!img.isNull()) {
        it->second.image_size = QSizeF(img.width(), img.height());
        it->second.texture.reset(
            win->createTextureFromImage(img, QQuickWindow::TextureHasAlphaChannel));
        out_image_size = it->second.image_size;
      }
    }
    return it->second.texture.get();
  }

  const QImage img = NveQtImageCache::shared().getImage(uri);
  if (img.isNull()) {
    return nullptr;
  }

  ImageTextureCacheEntry entry{};
  entry.image_size = QSizeF(img.width(), img.height());
  entry.last_used_generation = scene_generation;
  if (win != nullptr) {
    entry.texture.reset(
        win->createTextureFromImage(img, QQuickWindow::TextureHasAlphaChannel));
  }
  out_image_size = entry.image_size;
  auto [inserted, _] = image_cache_.emplace(key, std::move(entry));
  return inserted->second.texture.get();
}

void NveQtTextCache::eraseText(uint32_t node_id) {
  text_cache_.erase(node_id);
}

void NveQtTextCache::clear() {
  text_cache_.clear();
  image_cache_.clear();
}

ResolvedContentTexture resolveNodeContentTexture(
    NveQtTextCache& cache,
    const NveSceneView& view,
    const NveRenderNode& node,
    float w,
    float h,
    qreal dpr,
    int dpr_centis,
    uint64_t scene_generation,
    QQuickWindow* win,
    bool is_focused_input,
    const std::unordered_map<uint32_t, NodeSemantics>* node_semantics,
    const std::unordered_map<uint32_t, ImageResizeMode>* image_resize_modes,
    const std::unordered_map<uint32_t, std::string>* text_input_buffers,
    const std::unordered_map<uint32_t, QString>* ime_preedit_buffers,
    uint32_t& out_rasterized_count) {
  ResolvedContentTexture res{};
  const bool is_leaf_text = (node.flags & NVE_NODE_LEAF_TEXT) != 0u;
  const bool is_text_input = (node.kind == NVE_NODE_KIND_TEXT_INPUT);

  if ((is_leaf_text || is_text_input) && w > 0.0f && h > 0.0f) {
    std::string text_str;
    if (is_text_input && text_input_buffers != nullptr) {
      if (auto bit = text_input_buffers->find(node.node_id); bit != text_input_buffers->end()) {
        text_str = bit->second;
      } else {
        const std::string_view sv = nve::scene_node_text(view, node);
        text_str.assign(sv.data(), sv.size());
      }
    } else {
      const std::string_view sv = nve::scene_node_text(view, node);
      text_str.assign(sv.data(), sv.size());
    }

    uint32_t effective_text_rgba = node.text_rgba;
    if (is_text_input) {
      const NodeSemantics* sem = nullptr;
      if (node_semantics != nullptr) {
        if (auto sit = node_semantics->find(node.node_id); sit != node_semantics->end()) {
          sem = &sit->second;
        }
      }
      QString preedit;
      if (ime_preedit_buffers != nullptr) {
        if (auto pit = ime_preedit_buffers->find(node.node_id);
            pit != ime_preedit_buffers->end()) {
          preedit = pit->second;
        }
      }
      resolveTextInputDisplay(text_str, effective_text_rgba, sem, preedit);
    }

    if (!text_str.empty() || is_focused_input) {
      QSizeF logical_size;
      bool rasterized = false;
      res.texture = cache.acquireTextTexture(
          node.node_id, text_str, node.font_size, effective_text_rgba, w, h, dpr, dpr_centis,
          is_text_input, is_focused_input, scene_generation, win, logical_size, rasterized);
      if (rasterized) ++out_rasterized_count;
      if (res.texture != nullptr) {
        res.dst_rect = QRectF(0.0, 0.0, logical_size.width(), logical_size.height());
        res.is_text = true;
      }
    } else {
      cache.eraseText(node.node_id);
    }
    return res;
  }

  cache.eraseText(node.node_id);
  if (node.kind == NVE_NODE_KIND_IMAGE && w > 0.0f && h > 0.0f) {
    const std::string_view uri = nve::scene_node_text(view, node);
    if (!uri.empty()) {
      res.image_uri.assign(uri.data(), uri.size());
      QSizeF img_size;
      res.texture = cache.acquireImageTexture(uri, scene_generation, win, img_size);
      if (res.texture != nullptr) {
        ImageResizeMode mode = ImageResizeMode::Cover;
        if (image_resize_modes != nullptr) {
          if (auto mit = image_resize_modes->find(node.node_id);
              mit != image_resize_modes->end()) {
            mode = mit->second;
          }
        }
        if (computeImageDrawRects(
                mode, QRectF(0.0, 0.0, w, h), img_size, res.dst_rect, res.src_rect)) {
          res.use_src_rect = true;
          res.is_image = true;
        } else {
          res.texture = nullptr;
        }
      }
    }
  }
  return res;
}

TextKeyAction applyTextInputKeyPress(
    const QKeyEvent& event,
    std::string& io_text,
    const NodeSemantics* sem,
    const QString& active_preedit) {
  if (event.key() == Qt::Key_Return || event.key() == Qt::Key_Enter) {
    if (!active_preedit.isEmpty()) return TextKeyAction::Ignored;
    if (sem != nullptr && sem->multiline) {
      io_text.push_back('\n');
      return TextKeyAction::TextChanged;
    }
    return TextKeyAction::Submit;
  }
  if (event.key() == Qt::Key_Backspace) {
    if (io_text.empty()) return TextKeyAction::Ignored;
    popUtf8Char(io_text);
    return TextKeyAction::TextChanged;
  }
  const QString text = event.text();
  if (!text.isEmpty() && !text.at(0).isNull() && text.at(0).isPrint()) {
    const QByteArray utf8 = text.toUtf8();
    io_text.append(utf8.constData(), static_cast<size_t>(utf8.size()));
    return TextKeyAction::TextChanged;
  }
  return TextKeyAction::Ignored;
}

bool applyTextInputImeEvent(
    const QInputMethodEvent& event,
    uint32_t node_id,
    std::string& io_text,
    std::unordered_map<uint32_t, QString>& io_preedit_buffers) {
  QString qcur = QString::fromUtf8(io_text.data(), static_cast<qsizetype>(io_text.size()));
  const qsizetype cursor = qcur.size();
  const int rlen = event.replacementLength();
  const QString commit = event.commitString();
  const QString preedit = event.preeditString();

  bool committed = false;
  if (rlen > 0) {
    const qsizetype start = std::clamp<qsizetype>(cursor + event.replacementStart(), 0, qcur.size());
    const qsizetype len = std::min<qsizetype>(rlen, qcur.size() - start);
    if (len > 0) {
      qcur.remove(start, len);
      committed = true;
    }
    if (!commit.isEmpty()) {
      qcur.insert(start, commit);
      committed = true;
    }
  } else if (!commit.isEmpty()) {
    qcur.append(commit);
    committed = true;
  }

  if (preedit.isEmpty()) {
    io_preedit_buffers.erase(node_id);
  } else {
    io_preedit_buffers[node_id] = preedit;
  }
  if (committed) {
    const QByteArray utf8 = qcur.toUtf8();
    io_text.assign(utf8.constData(), static_cast<size_t>(utf8.size()));
  }
  return committed;
}

void refreshQuickNodeMetadata(
    const NveSceneView& view,
    const std::vector<nve::RenderedNodeSnapshot>& snapshots,
    std::unordered_map<uint32_t, NodeSemantics>& io_semantics,
    std::unordered_map<uint32_t, ImageResizeMode>& io_resize_modes,
    bool skip_dirty_tracking,
    std::unordered_set<uint32_t>& io_dirty_ids) {
  const auto raw_props = extractRawPropsByNode(snapshots);
  std::unordered_map<uint32_t, NodeSemantics> next_sem;
  next_sem.reserve(view.node_count);
  std::unordered_map<uint32_t, ImageResizeMode> next_modes;
  for (uint32_t i = 0u; i < view.node_count; ++i) {
    const NveRenderNode& node = view.nodes[i];
    const auto rp_it = raw_props.find(node.node_id);
    const nve::RawPropMap* rp = (rp_it != raw_props.end()) ? &rp_it->second : nullptr;
    NodeSemantics sem = extractNodeSemantics(node, nve::scene_node_text(view, node), rp);
    const auto prev_sem = io_semantics.find(node.node_id);
    if (!skip_dirty_tracking && (prev_sem == io_semantics.end() || prev_sem->second != sem)) {
      io_dirty_ids.insert(node.node_id);
    }
    next_sem.emplace(node.node_id, std::move(sem));
    if (node.kind == NVE_NODE_KIND_IMAGE) {
      const ImageResizeMode mode = extractImageResizeMode(rp);
      const auto prev_m = io_resize_modes.find(node.node_id);
      if (!skip_dirty_tracking && (prev_m == io_resize_modes.end() || prev_m->second != mode)) {
        io_dirty_ids.insert(node.node_id);
      }
      next_modes.emplace(node.node_id, mode);
    }
  }
  io_semantics = std::move(next_sem);
  io_resize_modes = std::move(next_modes);
}

} // namespace nve::qt
