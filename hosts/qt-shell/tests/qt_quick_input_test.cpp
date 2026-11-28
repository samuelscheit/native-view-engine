// Qt Quick host input: picking (z-index, clips, hidden nodes, scroll offsets), press / release /
// link activation, hover enter / move / leave with node-local points, cursors, self-removing
// press targets, wheel scrolling with chaining and programmatic scroll applied once, and
// controlled text inputs (typing, echo / transform / reject, caret keys, max length, secure,
// read-only, submit vs. newline, IME composition, Tab traversal, focus / blur).
#include "qt_quick_helpers.hpp"

#include <QDesktopServices>
#include <QInputMethodEvent>
#include <QUrl>

#include <cctype>

namespace {

using namespace nve::qt;
using namespace nve::qt::test;
namespace h = nve::host;

class UrlSink : public QObject {
  Q_OBJECT
 public:
  std::vector<QUrl> opened;
 public slots:
  void open(const QUrl& url) { opened.push_back(url); }
};

void test_pointer_hover_links_and_picking() {
  constexpr uint32_t kHover = 2u, kLink = 3u, kPressLink = 4u, kSelfRemove = 5u, kUnder = 6u, kOver = 7u,
                     kHidden = 8u, kClip = 9u, kOutside = 10u, kText = 11u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_DIRECTION, NVE_FLEX_DIRECTION_ROW);
  w.set(1u, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_ALIGN_ITEMS, NVE_ALIGN_ITEMS_FLEX_START);
  w.create(kHover);
  size_node(w, kHover, 50.0f, 20.0f);
  subscribe(w, kHover,
            {NVE_INTERACTION_EVENT_POINTER_ENTER, NVE_INTERACTION_EVENT_POINTER_LEAVE,
             NVE_INTERACTION_EVENT_POINTER_MOVE, NVE_INTERACTION_EVENT_POINTER_DOWN,
             NVE_INTERACTION_EVENT_POINTER_UP});
  w.set(kHover, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_CURSOR, NVE_CURSOR_KIND_GRAB);
  text_node(w, kLink, "docs");
  (void)w.set_str(kLink, NVE_TABLE_LINK_DESC, NVE_TABLE_LINK_HREF, "https://example.com/docs");
  text_node(w, kPressLink, "app");
  (void)w.set_str(kPressLink, NVE_TABLE_LINK_DESC, NVE_TABLE_LINK_HREF, "https://example.com/never");
  subscribe(w, kPressLink, {NVE_INTERACTION_EVENT_PRESS});
  w.create(kSelfRemove);
  size_node(w, kSelfRemove, 30.0f, 30.0f);
  subscribe(w, kSelfRemove, {NVE_INTERACTION_EVENT_PRESS});
  // kOver (z 1) overlaps kUnder via a negative margin: it takes the press.
  w.create(kUnder);
  size_node(w, kUnder, 40.0f, 40.0f);
  subscribe(w, kUnder, {NVE_INTERACTION_EVENT_PRESS});
  w.create(kOver);
  size_node(w, kOver, 40.0f, 40.0f);
  w.set(kOver, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_MARGIN_LEFT, h::length_points(-20.0f));
  w.set(kOver, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_Z_INDEX, 1u);
  subscribe(w, kOver, {NVE_INTERACTION_EVENT_PRESS});
  w.create(kHidden);  // invisible nodes take no hits
  size_node(w, kHidden, 30.0f, 30.0f);
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_HIDDEN);
  subscribe(w, kHidden, {NVE_INTERACTION_EVENT_PRESS});
  w.create(kClip);  // its child sticks out of the clip: not hittable outside
  size_node(w, kClip, 30.0f, 30.0f);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_HIDDEN);
  w.set(kClip, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_Y, NVE_OVERFLOW_HIDDEN);
  w.create(kOutside);
  size_node(w, kOutside, 30.0f, 120.0f);
  subscribe(w, kOutside, {NVE_INTERACTION_EVENT_PRESS});
  const uint32_t clipped[] = {kOutside};
  w.set_children(kClip, clipped);
  text_node(w, kText, "plain");
  w.set(kText, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_CURSOR, NVE_CURSOR_KIND_TEXT);
  const uint32_t kids[] = {kHover, kLink, kPressLink, kSelfRemove, kUnder, kOver, kHidden, kClip, kText};
  w.set_children(1u, kids);
  app.on_event = [](ScriptedApp& a, const RecordedEvent& e) {
    if (h::is_interaction_event(e.ev, NVE_INTERACTION_EVENT_PRESS) && e.ev.node == kSelfRemove) {
      a.w.remove(kSelfRemove);
      const uint32_t rest[] = {kHover, kLink, kPressLink, kUnder, kOver, kHidden, kClip, kText};
      a.w.set_children(1u, rest);
    }
  };

  UrlSink sink;
  QDesktopServices::setUrlHandler(QStringLiteral("https"), &sink, "open");
  QuickHarness q(QSize(600, 200));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));

  // Picking: z-index, hidden, clipped, empty space.
  const QRectF under = item.nodeRect(kUnder), over = item.nodeRect(kOver);
  NVE_CHECK(over.left() < under.right());
  const QPointF overlap((over.left() + under.right()) / 2.0, 10.0);
  NVE_CHECK(item.hitTestNode(overlap) == kOver);
  NVE_CHECK(item.hitTestNode(QPointF(under.left() + 2.0, 10.0)) == kUnder);
  NVE_CHECK(item.hitTestNode(q.center(kHidden)) != kHidden);
  const QRectF clip = item.nodeRect(kClip);
  NVE_CHECK(item.hitTestNode(QPointF(clip.center().x(), 10.0)) == kOutside);
  NVE_CHECK(item.hitTestNode(QPointF(clip.center().x(), 80.0)) != kOutside);  // beyond the clip
  NVE_CHECK(item.hitTestNode(QPointF(590.0, 190.0)) == 0u);  // empty space: no node
  NVE_CHECK(item.hitTestNode(QPointF(item.nodeRect(kLink).center()), true) == kLink);  // links are interactive

  q.click(overlap);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kOver) == 1u &&
            app.count(NVE_INTERACTION_EVENT_PRESS, kUnder) == 0u);
  q.click(q.center(kHidden));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kHidden) == 0u);
  // Press on one node, release on another: no activation.
  q.press(q.center(kUnder) - QPointF(10, 0));
  q.release(QPointF(590.0, 190.0));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kUnder) == 0u);
  // Pointer down / up carry node-local points.
  const QRectF hover = item.nodeRect(kHover);
  q.click(hover.topLeft() + QPointF(5, 6));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_POINTER_DOWN, kHover) == 1u &&
            app.count(NVE_INTERACTION_EVENT_POINTER_UP, kHover) == 1u);
  NVE_CHECK(h::event_payload_f32(app.last(NVE_INTERACTION_EVENT_POINTER_DOWN)->ev, 0u) == 5.0f &&
            h::event_payload_f32(app.last(NVE_INTERACTION_EVENT_POINTER_UP)->ev, 1u) == 6.0f);

  // Hover: enter once, move with local points, leave when moving off; cursors follow.
  q.move(hover.topLeft() + QPointF(3, 4));
  q.move(hover.topLeft() + QPointF(7, 8));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_POINTER_ENTER, kHover) == 1u &&
            app.count(NVE_INTERACTION_EVENT_POINTER_MOVE, kHover) >= 2u);
  NVE_CHECK(h::event_payload_f32(app.last(NVE_INTERACTION_EVENT_POINTER_ENTER)->ev, 0u) == 3.0f &&
            h::event_payload_f32(app.last(NVE_INTERACTION_EVENT_POINTER_MOVE)->ev, 1u) == 8.0f);
  NVE_CHECK(item.cursor().shape() == Qt::OpenHandCursor);
  q.move(q.center(kLink));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_POINTER_LEAVE, kHover) == 1u &&
            item.cursor().shape() == Qt::PointingHandCursor);
  q.move(q.center(kText));
  NVE_CHECK(item.cursor().shape() == Qt::IBeamCursor);
  q.move(QPointF(590.0, 190.0));
  NVE_CHECK(item.cursor().shape() == Qt::ArrowCursor);
  q.move(hover.center());
  QEvent leave(QEvent::Leave);
  QCoreApplication::sendEvent(&q.window, &leave);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_POINTER_ENTER, kHover) == 2u &&
            app.count(NVE_INTERACTION_EVENT_POINTER_LEAVE, kHover) == 2u);

  // Links: native open without a Press subscription; Press instead with one.
  q.click(q.center(kLink));
  NVE_CHECK(sink.opened.size() == 1u && sink.opened[0] == QUrl("https://example.com/docs"));
  q.click(q.center(kPressLink));
  NVE_CHECK(sink.opened.size() == 1u && app.count(NVE_INTERACTION_EVENT_PRESS, kPressLink) == 1u);
  QDesktopServices::unsetUrlHandler(QStringLiteral("https"));

  // A press handler removing its own node: the record dies, nothing dangles.
  q.click(q.center(kSelfRemove));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kSelfRemove) == 1u && item.nodeRect(kSelfRemove).isNull());
  NVE_CHECK(item.nodeCount() == 9u);
  (void)q.grab();
  NVE_CHECK(q.sg(kSelfRemove) == nullptr);
}

void test_wheel_and_programmatic_scroll() {
  constexpr uint32_t kOuter = 2u, kInner = 3u, kInnerContent = 4u, kMarker = 5u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kOuter);  // 200x100 scroll view holding a 200x60 inner scroller and a marker at y 250
  w.attach(kOuter, NVE_TABLE_SCROLL_DESC);
  size_node(w, kOuter, 200.0f, 100.0f);
  subscribe(w, kOuter, {NVE_SCROLL_EVENT_SCROLL});
  w.create(kInner);
  w.attach(kInner, NVE_TABLE_SCROLL_DESC);
  size_node(w, kInner, 200.0f, 60.0f);
  subscribe(w, kInner, {NVE_SCROLL_EVENT_SCROLL});
  w.create(kInnerContent);
  size_node(w, kInnerContent, 200.0f, 100.0f);
  const uint32_t inner[] = {kInnerContent};
  w.set_children(kInner, inner);
  w.create(kMarker);
  size_node(w, kMarker, 200.0f, 50.0f);
  w.set(kMarker, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_MARGIN_TOP, h::length_points(190.0f));
  w.set(kMarker, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 0, 255));
  const uint32_t outer[] = {kInner, kMarker};
  w.set_children(kOuter, outer);
  const uint32_t kids[] = {kOuter};
  w.set_children(1u, kids);

  QuickHarness q(QSize(200, 200));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(!same_rgb(q.at(100, 60), 0, 0, 255));  // the marker (y 250) is scrolled out

  // Wheel over the inner scroller: it scrolls first (to its 40px end), then the outer one.
  q.wheel(QPointF(100, 30), QPoint(0, -30));
  NVE_CHECK(approx(item.scrollOffsetForNode(kInner).y(), 30.0, 0.01) && item.scrollOffsetForNode(kOuter).isNull());
  NVE_CHECK(app.count(NVE_SCROLL_EVENT_SCROLL, kInner) == 1u &&
            approx(h::event_payload_f32(app.last(NVE_SCROLL_EVENT_SCROLL)->ev, 3u), 100.0));
  q.wheel(QPointF(100, 30), QPoint(0, -30));
  NVE_CHECK(approx(item.scrollOffsetForNode(kInner).y(), 40.0, 0.01));  // clamped
  q.wheel(QPointF(100, 30), QPoint(0, -30));                         // inner at its end: chains out
  NVE_CHECK(approx(item.scrollOffsetForNode(kOuter).y(), 30.0, 0.01) &&
            app.count(NVE_SCROLL_EVENT_SCROLL, kOuter) == 1u);
  // Angle deltas (one notch = 48 px) over the outer view, clamped to content 300 - viewport 100.
  q.wheel(QPointF(100, 90), QPoint(), QPoint(0, -600));
  NVE_CHECK(approx(item.scrollOffsetForNode(kOuter).y(), 200.0, 0.01));
  NVE_CHECK(approx(item.nodeRect(kMarker).y(), 50.0, 0.5));  // content y 250, shifted by the offset
  NVE_CHECK(same_rgb(q.at(100, 75), 0, 0, 255));              // painted at its scrolled place
  NVE_CHECK(!same_rgb(q.at(100, 120), 0, 0, 255));            // clipped by the outer viewport
  // The runtime agrees, and picking honours the offset.
  NVE_CHECK(approx(item.host().scroll_by(kOuter, 0.0f, 0.0f)->y(), 200.0, 0.01));
  NVE_CHECK(item.hitTestNode(QPointF(100, 75)) == kMarker);

  // Programmatic scroll: applied once, clamped, never echoed as a Scroll event.
  const size_t outer_events = app.count(NVE_SCROLL_EVENT_SCROLL, kOuter);
  w.set_f32(kOuter, NVE_TABLE_SCROLL_DESC, NVE_TABLE_SCROLL_REQUESTED_SCROLL_Y, 10.0f);
  flush(item.host());
  NVE_CHECK(approx(item.scrollOffsetForNode(kOuter).y(), 10.0, 0.01));
  q.wheel(QPointF(100, 90), QPoint(0, -20));  // the user scrolls away; unrelated frames keep it
  w.set(kMarker, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 255, 0));
  flush(item.host());
  NVE_CHECK(approx(item.scrollOffsetForNode(kOuter).y(), 30.0, 0.01));
  w.set_f32(kOuter, NVE_TABLE_SCROLL_DESC, NVE_TABLE_SCROLL_REQUESTED_SCROLL_Y, 5000.0f);
  flush(item.host());
  NVE_CHECK(approx(item.scrollOffsetForNode(kOuter).y(), 200.0, 0.5));
  NVE_CHECK(app.count(NVE_SCROLL_EVENT_SCROLL, kOuter) == outer_events + 1u);  // only the user's wheel
  // Wheel over non-scrollable space (below the 100px viewport) scrolls nothing.
  const size_t all_scrolls = app.count(NVE_SCROLL_EVENT_SCROLL);
  q.wheel(QPointF(100, 150), QPoint(0, 30));
  NVE_CHECK(app.count(NVE_SCROLL_EVENT_SCROLL) == all_scrolls &&
            approx(item.scrollOffsetForNode(kOuter).y(), 200.0, 0.5));
}

void test_text_inputs_keys_ime_and_focus() {
  constexpr uint32_t kEcho = 2u, kUpper = 3u, kReject = 4u, kNotes = 5u, kSecret = 6u, kButton = 7u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kEcho);
  size_node(w, kEcho, 200.0f, 24.0f);
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "hello");
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_PLACEHOLDER, "Na\x01me");
  w.set(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 8u);
  w.create(kUpper);
  size_node(w, kUpper, 200.0f, 24.0f);
  w.attach(kUpper, NVE_TABLE_TEXT_INPUT_DESC);
  w.create(kReject);
  size_node(w, kReject, 200.0f, 24.0f);
  (void)w.set_str(kReject, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "fixed");
  w.create(kNotes);
  size_node(w, kNotes, 200.0f, 48.0f);
  w.set_bool(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MULTILINE, true);
  (void)w.set_str(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "a\r\nb\tc");
  w.create(kSecret);
  size_node(w, kSecret, 200.0f, 24.0f);
  w.set_bool(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_SECURE, true);
  w.set_bool(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_READ_ONLY, true);
  (void)w.set_str(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "pw\x7fx");
  w.create(kButton);
  size_node(w, kButton, 60.0f, 24.0f);
  w.set_bool(kButton, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  subscribe(w, kButton, {NVE_INTERACTION_EVENT_PRESS, NVE_INTERACTION_EVENT_FOCUS, NVE_INTERACTION_EVENT_BLUR});
  const uint32_t kids[] = {kEcho, kUpper, kReject, kNotes, kSecret, kButton};
  w.set_children(1u, kids);
  app.on_event = [](ScriptedApp& a, const RecordedEvent& e) {
    if (!h::is_text_input_event(e.ev, NVE_TEXT_INPUT_EVENT_CHANGE) || e.ev.node == kReject) return;
    std::string v = e.text;
    if (e.ev.node == kUpper) {
      for (char& c : v) c = static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
    }
    (void)a.w.set_str(e.ev.node, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, v);
  };

  QuickHarness q(QSize(300, 240));
  ProbeItem& item = *q.item;
  NVE_CHECK(item.host().attach_interface(&ScriptedApp::get_interface));
  q.window.requestActivate();
  spin();
  NVE_CHECK(item.nodeText(kEcho) == "hello" && item.nodeText(kNotes) == "a\nb c" && item.nodeText(kSecret) == "pwx");
  NVE_CHECK(item.accessibilityInfoForNodeId(kEcho)->role == QAccessible::EditableText);
  NVE_CHECK(item.accessibilityInfoForNodeId(kEcho)->placeholder == "Name");

  // Click focuses (caret at the end); typing echoes; the app's echo never fights the caret.
  q.click(q.center(kEcho));
  NVE_CHECK(item.focusedNodeId() == kEcho && item.hasActiveFocus());
  NVE_CHECK(item.inputMethodQuery(Qt::ImEnabled).toBool() && item.inputMethodQuery(Qt::ImCursorPosition).toInt() == 5);
  q.type(QStringLiteral("a"));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kEcho) == 1u &&
            app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "helloa");
  q.key(Qt::Key_Home);
  q.type(QStringLiteral("x"));
  NVE_CHECK(item.nodeText(kEcho) == "xhelloa" && item.inputMethodQuery(Qt::ImCursorPosition).toInt() == 1);
  q.key(Qt::Key_Delete);  // removes 'h'
  q.key(Qt::Key_End);
  q.key(Qt::Key_Backspace);
  NVE_CHECK(item.nodeText(kEcho) == "xello" && app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "xello");
  q.key(Qt::Key_Left);
  q.key(Qt::Key_Right);
  q.key(Qt::Key_Right);  // clamped at the end
  NVE_CHECK(item.inputMethodQuery(Qt::ImCursorPosition).toInt() == 5);
  // Max length 8: extra characters are cut, the app sees at most 8.
  q.type(QStringLiteral("123456"));
  NVE_CHECK(item.nodeText(kEcho) == "xello123" && app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "xello123");
  // Control characters typed are dropped; an app value replaces the text without an echo.
  const size_t edits = app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kEcho);
  q.key(Qt::Key_Escape, QString(QChar(0x1b)));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kEcho) == edits);
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "reset");
  flush(item.host());
  NVE_CHECK(item.nodeText(kEcho) == "reset" && app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kEcho) == edits);
  q.key(Qt::Key_Return, QStringLiteral("\r"));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_SUBMIT, kEcho) == 1u &&
            app.last(NVE_TEXT_INPUT_EVENT_SUBMIT)->text == "reset");

  // IME: composition is not dispatched nor clobbered by app updates; the commit is.
  QInputMethodEvent preedit(QStringLiteral("ni"), {});
  QCoreApplication::sendEvent(&item, &preedit);
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kEcho) == edits);
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "app?");
  flush(item.host());
  NVE_CHECK(item.nodeText(kEcho) == "reset");
  QInputMethodEvent commit;
  commit.setCommitString(QStringLiteral("你"));
  QCoreApplication::sendEvent(&item, &commit);
  NVE_CHECK(item.nodeText(kEcho) == "reset你" && app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "reset你");

  // Transforming and rejecting apps.
  q.click(q.center(kUpper));
  q.type(QStringLiteral("qw"));
  NVE_CHECK(item.nodeText(kUpper) == "QW" && app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "Qw");
  q.click(q.center(kReject));
  q.type(QStringLiteral("s"));  // rejected: the app's value is shown again (no echo of the revert)
  const size_t rejected = app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kReject);
  NVE_CHECK(item.nodeText(kReject) == "fixed" && app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "fixeds");
  q.type(QStringLiteral("t"));
  NVE_CHECK(item.nodeText(kReject) == "fixed" && app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "fixedt");
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kReject) == rejected + 1u);

  // Multi-line: Return inserts a newline, no submit.
  q.click(q.center(kNotes));
  q.key(Qt::Key_Return, QStringLiteral("\r"));
  q.type(QStringLiteral("d"));
  NVE_CHECK(item.nodeText(kNotes) == "a\nb c\nd" && app.count(NVE_TEXT_INPUT_EVENT_SUBMIT, kNotes) == 0u);
  NVE_CHECK(item.inputMethodQuery(Qt::ImHints).toInt() & Qt::ImhMultiLine);

  // Secure + read-only: focusable, hidden-text hints, typing ignored.
  q.click(q.center(kSecret));
  NVE_CHECK(item.focusedNodeId() == kSecret && !item.inputMethodQuery(Qt::ImEnabled).toBool());
  NVE_CHECK(item.inputMethodQuery(Qt::ImHints).toInt() & Qt::ImhHiddenText);
  q.type(QStringLiteral("zz"));
  NVE_CHECK(item.nodeText(kSecret) == "pwx" && app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kSecret) == 0u);

  // Tab traversal in tree order, wrapping; focus / blur events; Space activates the button.
  q.key(Qt::Key_Tab);
  NVE_CHECK(item.focusedNodeId() == kButton && app.count(NVE_INTERACTION_EVENT_FOCUS, kButton) == 1u);
  q.key(Qt::Key_Space, QStringLiteral(" "));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kButton) == 1u);
  q.key(Qt::Key_Tab);
  NVE_CHECK(item.focusedNodeId() == kEcho && app.count(NVE_INTERACTION_EVENT_BLUR, kButton) == 1u);
  q.key(Qt::Key_Backtab);
  NVE_CHECK(item.focusedNodeId() == kButton);
  // Clicking empty space clears focus; losing item focus blurs too.
  q.click(QPointF(290, 230));
  NVE_CHECK(item.focusedNodeId() == 0u && app.count(NVE_INTERACTION_EVENT_BLUR, kButton) == 2u);
  q.click(q.center(kButton));
  NVE_CHECK(item.focusedNodeId() == kButton);
  item.setFocus(false);
  spin();
  NVE_CHECK(item.focusedNodeId() == 0u && app.count(NVE_INTERACTION_EVENT_BLUR, kButton) == 3u);
  // Text inputs are painted: each edit re-rasterized only its own node.
  (void)q.grab();
  const uint64_t text_rasters = item.sceneGraphStats().textRasterizations;
  q.click(q.center(kEcho));
  q.type(QStringLiteral("!"));
  (void)q.grab();
  NVE_CHECK(item.sceneGraphStats().textRasterizations == text_rasters + 1u);
}

}  // namespace

void nve_test_qt_quick_input() {
  test_pointer_hover_links_and_picking();
  test_wheel_and_programmatic_scroll();
  test_text_inputs_keys_ime_and_focus();
}

#include "qt_quick_input_test.moc"
