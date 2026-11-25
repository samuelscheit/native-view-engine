#pragma once

#include "NveQtCommon.hpp"

#include <QImage>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QQuickWindow>
#include <QRectF>
#include <QSGTexture>
#include <QSizeF>

#include <cmath>
#include <cstdint>
#include <cstring>
#include <memory>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace nve::qt {

/// Remove the last UTF-8 codepoint from a UTF-8 `std::string`.
void popUtf8Char(std::string& s);

/// Format effective `TextInput` display string (password masking, IME preedit append,
/// and dimmed placeholder fallback) and update `io_text_rgba`.
void resolveTextInputDisplay(
    std::string& io_text,
    uint32_t& io_text_rgba,
    const NodeSemantics* sem,
    const QString& preedit);

/// Rasterize UTF-8 text into a HiDPI premultiplied RGBA `QImage` using `QTextLayout` + `QPainter`.
QImage rasterizeTextImage(
    const std::string& text,
    float font_size,
    uint32_t text_rgba,
    float layout_w,
    float layout_h,
    qreal dpr,
    bool is_text_input,
    bool focused,
    QSizeF& out_logical_size);

/// Rasterize an `NVE_DRAW1` binary draw list into a HiDPI premultiplied RGBA `QImage`.
/// Returns a null `QImage` when `bytes` is empty or fails validation.
QImage rasterizeDrawListImage(
    std::span<const uint8_t> bytes,
    float layout_w,
    float layout_h,
    qreal dpr,
    QSizeF& out_logical_size);

/// Cached per-node GPU text texture owned on the Scene Graph render thread.
struct TextTextureCacheEntry {
  std::string text;
  float font_size{16.0f};
  uint32_t text_rgba{0};
  int quant_w{0};
  int quant_h{0};
  int quant_dpr{100};
  bool is_text_input{false};
  bool focused{false};

  std::unique_ptr<QSGTexture> texture;
  QSizeF logical_size;
  uint64_t last_used_generation{0};
  uint64_t texture_generation{0};

  bool matches(
      std::string_view next_text,
      float next_font_size,
      uint32_t next_text_rgba,
      int next_qw,
      int next_qh,
      int next_qdpr,
      bool next_input,
      bool next_focused) const noexcept {
    return text == next_text &&
           std::fabs(font_size - next_font_size) < 0.01f &&
           text_rgba == next_text_rgba &&
           quant_w == next_qw &&
           quant_h == next_qh &&
           quant_dpr == next_qdpr &&
           is_text_input == next_input &&
           focused == next_focused;
  }
};

/// Cached GPU image texture uploaded from `NveQtImageCache` (keyed by URI).
struct ImageTextureCacheEntry {
  std::unique_ptr<QSGTexture> texture;
  QSizeF image_size;
  uint64_t last_used_generation{0};
};

/// Cached per-node GPU canvas texture rasterized from `NVE_DRAW1` bytes, keyed by
/// `(node_id, quant_w, quant_h, quant_dpr, byte_hash, bytes)`.
struct CanvasTextureCacheEntry {
  int quant_w{0};
  int quant_h{0};
  int quant_dpr{100};
  uint64_t byte_hash{0};
  std::vector<uint8_t> bytes;

  std::unique_ptr<QSGTexture> texture;
  QSizeF logical_size;
  uint64_t last_used_generation{0};
  uint64_t texture_generation{0};

  bool matches(
      int next_qw,
      int next_qh,
      int next_qdpr,
      uint64_t next_hash,
      std::span<const uint8_t> next_bytes) const noexcept {
    return quant_w == next_qw &&
           quant_h == next_qh &&
           quant_dpr == next_qdpr &&
           byte_hash == next_hash &&
           bytes.size() == next_bytes.size() &&
           (next_bytes.empty() ||
            std::memcmp(bytes.data(), next_bytes.data(), next_bytes.size()) == 0);
  }
};

/// Render-thread GPU texture cache for rasterized text, decoded `Image`, and `NVE_DRAW1` canvas nodes.
class NveQtTextCache {
 public:
  NveQtTextCache() = default;
  ~NveQtTextCache() = default;

  NveQtTextCache(const NveQtTextCache&) = delete;
  NveQtTextCache& operator=(const NveQtTextCache&) = delete;

  /// Acquire or rasterize a HiDPI GPU text texture for `node_id`.
  /// Sets `out_rasterized` to true iff a fresh rasterization occurred.
  QSGTexture* acquireTextTexture(
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
      bool& out_rasterized);

  /// Acquire or upload a GPU `QSGTexture` for `uri` backed by `NveQtImageCache::shared()`.
  QSGTexture* acquireImageTexture(
      std::string_view uri,
      uint64_t scene_generation,
      QQuickWindow* win,
      QSizeF& out_image_size);

  /// Acquire or rasterize a HiDPI GPU canvas texture for `NVE_NODE_KIND_SKIA_CANVAS`, keyed by
  /// `(node_id, width, height, byte_hash)`. Returns `nullptr` for empty or corrupt draw lists.
  QSGTexture* acquireCanvasTexture(
      uint32_t node_id,
      std::span<const uint8_t> bytes,
      float layout_w,
      float layout_h,
      qreal dpr,
      int dpr_centis,
      uint64_t scene_generation,
      QQuickWindow* win,
      QSizeF& out_logical_size,
      bool& out_rasterized);

  /// Immediately release the cached GPU text texture for a removed or non-text node.
  void eraseText(uint32_t node_id);

  /// Immediately release the cached GPU canvas texture for a removed or non-canvas node.
  void eraseCanvas(uint32_t node_id);

  /// Release cached text textures whose `node_id` is not present in `live_text_nodes`.
  template <typename Predicate>
  void pruneTextIf(Predicate&& should_erase) {
    for (auto it = text_cache_.begin(); it != text_cache_.end();) {
      if (should_erase(it->first)) {
        it = text_cache_.erase(it);
      } else {
        ++it;
      }
    }
  }

  /// Evict image GPU textures not referenced by any live node when cache grows past limit.
  template <typename Predicate>
  void pruneImagesIf(Predicate&& should_erase) {
    for (auto it = image_cache_.begin(); it != image_cache_.end();) {
      if (should_erase(it->first)) {
        it = image_cache_.erase(it);
      } else {
        ++it;
      }
    }
  }

  /// Release cached canvas textures whose `node_id` is no longer a live canvas node.
  template <typename Predicate>
  void pruneCanvasIf(Predicate&& should_erase) {
    for (auto it = canvas_cache_.begin(); it != canvas_cache_.end();) {
      if (should_erase(it->first)) {
        it = canvas_cache_.erase(it);
      } else {
        ++it;
      }
    }
  }

  void clear();

  uint32_t textCount() const noexcept { return static_cast<uint32_t>(text_cache_.size()); }
  uint32_t imageCount() const noexcept { return static_cast<uint32_t>(image_cache_.size()); }
  uint32_t canvasCount() const noexcept { return static_cast<uint32_t>(canvas_cache_.size()); }
  uint64_t totalRasterizations() const noexcept { return total_rasterizations_; }
  uint64_t totalCacheHits() const noexcept { return total_cache_hits_; }
  uint64_t totalCanvasRasterizations() const noexcept { return total_canvas_rasterizations_; }
  uint64_t totalCanvasCacheHits() const noexcept { return total_canvas_cache_hits_; }
  void addRetainedCacheHits(uint64_t hits) noexcept { total_cache_hits_ += hits; }

 private:
  std::unordered_map<uint32_t, TextTextureCacheEntry> text_cache_;
  std::unordered_map<std::string, ImageTextureCacheEntry> image_cache_;
  std::unordered_map<uint32_t, CanvasTextureCacheEntry> canvas_cache_;
  uint64_t total_rasterizations_{0};
  uint64_t total_cache_hits_{0};
  uint64_t total_canvas_rasterizations_{0};
  uint64_t total_canvas_cache_hits_{0};
};

/// Resolved GPU texture and geometry rects for a leaf text, TextInput, Image, or Canvas node.
struct ResolvedContentTexture {
  QSGTexture* texture{nullptr};
  QRectF dst_rect;
  QRectF src_rect;
  bool use_src_rect{false};
  bool is_text{false};
  bool is_image{false};
  bool is_canvas{false};
  std::string image_uri;
};

/// Resolve (or acquire/rasterize) the GPU content texture for `node`.
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
    uint32_t& out_rasterized_count);

/// Outcome of applying a `QKeyEvent` to a focused `TextInput` buffer.
enum class TextKeyAction { Ignored, TextChanged, Submit };

TextKeyAction applyTextInputKeyPress(
    const QKeyEvent& event,
    std::string& io_text,
    const NodeSemantics* sem,
    const QString& active_preedit);

/// Apply a `QInputMethodEvent` to `io_text` and `io_preedit_buffers[node_id]`.
/// Returns true iff committed text changed.
bool applyTextInputImeEvent(
    const QInputMethodEvent& event,
    uint32_t node_id,
    std::string& io_text,
    std::unordered_map<uint32_t, QString>& io_preedit_buffers);

/// Refresh cached `NodeSemantics` and `ImageResizeMode` maps from `RenderedNodeSnapshot::raw_props`,
/// recording any changed node IDs into `io_dirty_ids` when `!skip_dirty_tracking`.
void refreshQuickNodeMetadata(
    const NveSceneView& view,
    const std::vector<nve::RenderedNodeSnapshot>& snapshots,
    std::unordered_map<uint32_t, NodeSemantics>& io_semantics,
    std::unordered_map<uint32_t, ImageResizeMode>& io_resize_modes,
    bool skip_dirty_tracking,
    std::unordered_set<uint32_t>& io_dirty_ids);

} // namespace nve::qt
