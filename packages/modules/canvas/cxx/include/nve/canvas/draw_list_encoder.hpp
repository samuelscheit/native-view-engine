// `NVE_DRAW1` encoder (C++ apps, demos, and host test fixtures); byte-identical to `DrawListBuilder`.
#pragma once

#include "draw_list.hpp"

namespace nve::canvas {

class DrawListEncoder {
 public:
  explicit DrawListEncoder(size_t capacity_bytes = 256u) {
    buf_.reserve(std::max(capacity_bytes, kDrawListHeaderSize));
    buf_.insert(buf_.end(), kDrawListMagic.begin(), kDrawListMagic.end());
    detail::write_le_u16(buf_, kDrawListVersion);
    detail::write_le_u32(buf_, 0u);
  }

  uint32_t op_count() const noexcept { return op_count_; }

  DrawListEncoder& clear(DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::Clear));
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& fill_rect(float x, float y, float w, float h, DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::FillRect));
    detail::write_le_f32(buf_, x);
    detail::write_le_f32(buf_, y);
    detail::write_le_f32(buf_, w);
    detail::write_le_f32(buf_, h);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& fill_rounded_rect(
      float x,
      float y,
      float w,
      float h,
      float radius,
      DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::FillRoundedRect));
    detail::write_le_f32(buf_, x);
    detail::write_le_f32(buf_, y);
    detail::write_le_f32(buf_, w);
    detail::write_le_f32(buf_, h);
    detail::write_le_f32(buf_, radius);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& fill_circle(float cx, float cy, float radius, DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::FillCircle));
    detail::write_le_f32(buf_, cx);
    detail::write_le_f32(buf_, cy);
    detail::write_le_f32(buf_, radius);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& stroke_line(
      float x0,
      float y0,
      float x1,
      float y1,
      float width,
      DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::StrokeLine));
    detail::write_le_f32(buf_, x0);
    detail::write_le_f32(buf_, y0);
    detail::write_le_f32(buf_, x1);
    detail::write_le_f32(buf_, y1);
    detail::write_le_f32(buf_, width);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& stroke_polyline(
      std::span<const DrawPoint> points,
      float width,
      DrawColorRgba color) {
    const size_t clamped = std::min(points.size(), kMaxPolylinePoints);
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::StrokePolyline));
    detail::write_le_f32(buf_, width);
    detail::write_le_u32(buf_, color.to_u32());
    detail::write_le_u32(buf_, static_cast<uint32_t>(clamped));
    for (size_t i = 0u; i < clamped; ++i) {
      detail::write_le_f32(buf_, points[i].x);
      detail::write_le_f32(buf_, points[i].y);
    }
    ++op_count_;
    return *this;
  }

  DrawListEncoder& clip_rect(float x, float y, float w, float h) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::ClipRect));
    detail::write_le_f32(buf_, x);
    detail::write_le_f32(buf_, y);
    detail::write_le_f32(buf_, w);
    detail::write_le_f32(buf_, h);
    ++op_count_;
    return *this;
  }

  DrawListEncoder& restore_clip() {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::RestoreClip));
    ++op_count_;
    return *this;
  }

  /// Appends a Studio-style telemetry sparkline matching `CanvasOp::sparkline`.
  DrawListEncoder& sparkline(
      std::span<const float> samples,
      float width,
      float height,
      DrawColorRgba stroke_color,
      DrawColorRgba background_color) {
    const float w = (std::isfinite(width) && width > 0.0f) ? width : 16.0f;
    const float h = (std::isfinite(height) && height > 0.0f) ? height : 16.0f;
    const float pad_x = std::clamp(w * 0.04f, 4.0f, 12.0f);
    const float pad_y = std::clamp(h * 0.14f, 4.0f, 12.0f);
    const float plot_w = std::max(w - pad_x * 2.0f, 1.0f);
    const float plot_h = std::max(h - pad_y * 2.0f, 1.0f);

    const DrawColorRgba grid_color = DrawColorRgba::rgba(148u, 163u, 184u, 52u);
    const DrawColorRgba baseline_color = DrawColorRgba::rgba(148u, 163u, 184u, 90u);

    clear(DrawColorRgba::rgba(0u, 0u, 0u, 0u));
    fill_rounded_rect(0.0f, 0.0f, w, h, std::min(8.0f, h * 0.25f), background_color);
    clip_rect(1.0f, 1.0f, std::max(w - 2.0f, 1.0f), std::max(h - 2.0f, 1.0f));

    const float mid_y = pad_y + plot_h * 0.5f;
    stroke_line(pad_x, pad_y, pad_x + plot_w, pad_y, 1.0f, grid_color);
    stroke_line(pad_x, mid_y, pad_x + plot_w, mid_y, 1.0f, grid_color);
    stroke_line(pad_x, pad_y + plot_h, pad_x + plot_w, pad_y + plot_h, 1.0f, baseline_color);

    std::vector<float> clean;
    clean.reserve(std::min(samples.size(), kMaxPolylinePoints));
    for (float s : samples) {
      if (std::isfinite(s)) {
        clean.push_back(s);
        if (clean.size() >= kMaxPolylinePoints) break;
      }
    }

    if (clean.empty()) {
      restore_clip();
      return *this;
    }

    if (clean.size() == 1u) {
      const float cy = pad_y + plot_h * 0.5f;
      stroke_line(pad_x, cy, pad_x + plot_w, cy, 2.0f, stroke_color);
      fill_circle(pad_x + plot_w, cy, 3.5f, stroke_color);
      restore_clip();
      return *this;
    }

    float min_v = std::numeric_limits<float>::infinity();
    float max_v = -std::numeric_limits<float>::infinity();
    for (float v : clean) {
      min_v = std::min(min_v, v);
      max_v = std::max(max_v, v);
    }
    const float span = max_v - min_v;
    const float denom = static_cast<float>(clean.size() - 1u);

    std::vector<DrawPoint> pts;
    pts.reserve(clean.size());
    for (size_t idx = 0u; idx < clean.size(); ++idx) {
      const float t = static_cast<float>(idx) / denom;
      const float norm = (std::fabs(span) <= std::numeric_limits<float>::epsilon())
                             ? 0.5f
                             : std::clamp((clean[idx] - min_v) / span, 0.0f, 1.0f);
      pts.push_back(DrawPoint{pad_x + t * plot_w, pad_y + (1.0f - norm) * plot_h});
    }

    const DrawPoint last = pts.back();
    const float bar_w = std::clamp(plot_w / static_cast<float>(clean.size()), 3.0f, 10.0f);
    const float bar_h = std::max(pad_y + plot_h - last.y, 2.0f);
    fill_rect(
        std::max(last.x - bar_w * 0.5f, pad_x),
        last.y,
        bar_w,
        bar_h,
        DrawColorRgba::rgba(stroke_color.r, stroke_color.g, stroke_color.b, 48u));
    stroke_polyline(pts, 2.0f, stroke_color);
    fill_circle(last.x, last.y, 3.5f, stroke_color);
    restore_clip();
    return *this;
  }

  std::vector<uint8_t> finish() {
    buf_[11u] = static_cast<uint8_t>(op_count_ & 0xFFu);
    buf_[12u] = static_cast<uint8_t>((op_count_ >> 8u) & 0xFFu);
    buf_[13u] = static_cast<uint8_t>((op_count_ >> 16u) & 0xFFu);
    buf_[14u] = static_cast<uint8_t>((op_count_ >> 24u) & 0xFFu);
    return buf_;
  }

 private:
  std::vector<uint8_t> buf_;
  uint32_t op_count_{0u};
};

}  // namespace nve::canvas
