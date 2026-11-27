#pragma once

#include "NveQtHost.hpp"

#include <nve/host/testing/scripted_app.hpp>

#include <QApplication>
#include <QBuffer>
#include <QByteArray>
#include <QColor>
#include <QCoreApplication>
#include <QEventLoop>
#include <QImage>
#include <QMouseEvent>
#include <QWidget>

#include <cmath>
#include <cstdlib>
#include <iostream>
#include <string>
#include <string_view>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace nve::qt::test {

using host::testing::RecordedEvent;
using host::testing::ScriptedApp;

inline bool approx(double a, double b, double eps = 1.5) { return std::fabs(a - b) <= eps; }

/// Runs queued events (wakes, deferred deletes) until none are left (bounded).
inline void spin(int rounds = 8) {
  for (int i = 0; i < rounds; ++i) {
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    QCoreApplication::processEvents(QEventLoop::AllEvents);
  }
}

/// Renders the `ScriptedApp`'s pending ops and mirrors them into views.
inline void flush(QtHost& host) {
  (void)host.session().dispatch(ScriptedApp::flush_event(host.surface_id()));
  (void)host.pump();
}

inline void text_node(host::Nve2Writer& w, uint32_t id, std::string_view text) {
  w.create(id);
  (void)w.set_str(id, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, text);
}

inline void size_node(host::Nve2Writer& w, uint32_t id, float width, float height) {
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, host::length_points(width));
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, host::length_points(height));
  w.set_f32(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0.0f);
}

inline void subscribe(host::Nve2Writer& w, uint32_t id, std::initializer_list<NveEventKind> kinds) {
  uint32_t mask = 0u;
  for (const NveEventKind k : kinds) mask |= host::event_bit(k);
  w.set(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENT_MASK, mask);
}

/// A left click at widget-local `pos` (press + release, delivered synchronously).
inline void click(QWidget* w, QPointF pos) {
  const QPointF global = w->mapToGlobal(pos);
  QMouseEvent press(QEvent::MouseButtonPress, pos, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
  QMouseEvent release(QEvent::MouseButtonRelease, pos, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
  QApplication::sendEvent(w, &press);
  QApplication::sendEvent(w, &release);
}

/// Pixel of a widget rendered offscreen (device-independent coordinates, dpr 1).
inline QColor pixel(const QImage& img, int x, int y) { return img.pixelColor(x, y); }

inline QImage render(QWidget* w) {
  QImage img(w->size(), QImage::Format_ARGB32_Premultiplied);
  img.fill(Qt::transparent);
  w->render(&img);
  return img;
}

inline bool same_rgb(const QColor& c, int r, int g, int b, int eps = 3) {
  return std::abs(c.red() - r) <= eps && std::abs(c.green() - g) <= eps && std::abs(c.blue() - b) <= eps;
}

/// PNG bytes of a solid `w` x `h` image.
inline std::vector<uint8_t> png(int w, int h, QColor color) {
  QImage img(w, h, QImage::Format_ARGB32);
  img.fill(color);
  QByteArray out;
  QBuffer buf(&out);
  buf.open(QIODevice::WriteOnly);
  img.save(&buf, "PNG");
  return std::vector<uint8_t>(out.begin(), out.end());
}

inline std::string data_url(const std::vector<uint8_t>& bytes) {
  const QByteArray raw(reinterpret_cast<const char*>(bytes.data()), static_cast<qsizetype>(bytes.size()));
  return "data:image/png;base64," + raw.toBase64().toStdString();
}

}  // namespace nve::qt::test
