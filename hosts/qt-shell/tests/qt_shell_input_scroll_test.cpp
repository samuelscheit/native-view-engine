// Qt Widgets host: controlled text inputs (`QLineEdit` / `QPlainTextEdit`: echo, transform,
// reject, app-driven values, IME composition, secure, max length clamping, sanitized text,
// submit, focus / blur), native scroll views (content extents, user scroll -> event + runtime
// offset, programmatic scroll applied once and never echoed, overflow views), hover / pointer
// events, cursors, links (native open vs. Press), and a node removed by its own press handler.
#include "NveQtWidgetInternal.hpp"
#include "qt_test_helpers.hpp"

#include <QDesktopServices>
#include <QEnterEvent>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QPointer>
#include <QScrollArea>
#include <QScrollBar>
#include <QUrl>

#include <cctype>
#include <vector>

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

void type(QWidget* w, Qt::Key key, const QString& text) {
  QKeyEvent ev(QEvent::KeyPress, key, Qt::NoModifier, text);
  QApplication::sendEvent(w, &ev);
}

void test_controlled_text_inputs() {
  constexpr uint32_t kEcho = 2u, kUpper = 3u, kReject = 4u, kNotes = 5u, kSecret = 6u, kHuge = 7u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kEcho);
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "hello");
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_PLACEHOLDER, "Na\x01me");
  w.set(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 12u);
  w.set(kEcho, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(10, 20, 30));
  w.set(kEcho, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_COLOR, h::rgba(1, 2, 3));
  w.set_f32(kEcho, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_WIDTH, 2.0f);
  w.set_f32(kEcho, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, std::nanf(""));
  w.set(kEcho, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_PADDING_LEFT, h::length_points(6.0f));
  w.create(kUpper);
  w.attach(kUpper, NVE_TABLE_TEXT_INPUT_DESC);  // defaults-only row: an empty input
  w.create(kReject);
  (void)w.set_str(kReject, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "fixed");
  w.create(kNotes);
  w.set_bool(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MULTILINE, true);
  (void)w.set_str(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "a\r\nb\tc");
  w.set(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 8u);
  w.create(kSecret);
  w.set_bool(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_SECURE, true);
  w.set_bool(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_READ_ONLY, true);
  (void)w.set_str(kSecret, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "pw\x7f\x01x");
  w.create(kHuge);
  w.set(kHuge, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 65535u);  // the 16-bit field maximum
  const uint32_t kids[] = {kEcho, kUpper, kReject, kNotes, kSecret, kHuge};
  w.set_children(1u, kids);
  // kEcho / kNotes: echo; kUpper: transform to upper case; kReject: ignore edits.
  app.on_event = [](ScriptedApp& a, const RecordedEvent& e) {
    if (e.ev.kind != NVE_EVENT_TEXT_CHANGE || e.ev.node_id == kReject) return;
    std::string v = e.text;
    if (e.ev.node_id == kUpper) {
      for (char& c : v) c = static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
    }
    (void)a.w.set_str(e.ev.node_id, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, v);
  };

  NveQtWidget widget(QtHostConfig{.width = 400.0f, .height = 400.0f});
  QtHost& host = widget.host();
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  widget.show();
  widget.activateWindow();
  auto* echo = dynamic_cast<NveQtLineEdit*>(widget.nativeWidgetForNodeId(kEcho));
  auto* upper = dynamic_cast<NveQtLineEdit*>(widget.nativeWidgetForNodeId(kUpper));
  auto* reject = dynamic_cast<NveQtLineEdit*>(widget.nativeWidgetForNodeId(kReject));
  auto* notes = dynamic_cast<NveQtTextEdit*>(widget.nativeWidgetForNodeId(kNotes));
  auto* secret = dynamic_cast<NveQtLineEdit*>(widget.nativeWidgetForNodeId(kSecret));
  auto* huge = dynamic_cast<NveQtLineEdit*>(widget.nativeWidgetForNodeId(kHuge));
  NVE_CHECK(echo && upper && reject && notes && secret && huge);

  // Attributes, sanitized text, clamped lengths, style sheet with finite numbers only.
  NVE_CHECK(echo->text() == "hello" && echo->placeholderText() == "Name" && echo->maxLength() == 12);
  NVE_CHECK(echo->echoMode() == QLineEdit::Normal && !echo->isReadOnly());
  NVE_CHECK(huge->maxLength() == 32767);  // 65535 clamped to QLineEdit's range
  NVE_CHECK(secret->echoMode() == QLineEdit::Password && secret->isReadOnly() && secret->text() == "pwx");
  NVE_CHECK(notes->toPlainText() == "a\nb c");
  const QString css = echo->styleSheet();
  NVE_CHECK(css.contains("background-color: rgba(10, 20, 30, 255)") && css.contains("border: 2px solid rgba(1, 2, 3, 255)"));
  NVE_CHECK(css.contains("border-radius: 0px") && css.contains("padding: 0px 0px 0px 6px") && !css.contains("nan"));
  NVE_CHECK(widget.accessibilityInfoForNodeId(kEcho)->role == QAccessible::EditableText);
  NVE_CHECK(widget.accessibilityInfoForNodeId(kSecret)->secure && widget.accessibilityInfoForNodeId(kNotes)->multiline);

  // Typing: one TextChange per edit; the app's echo never fights the caret.
  echo->setCursorPosition(5);
  type(echo, Qt::Key_A, QStringLiteral("a"));
  NVE_CHECK(app.count(NVE_EVENT_TEXT_CHANGE, kEcho) == 1u && app.last(NVE_EVENT_TEXT_CHANGE)->text == "helloa");
  NVE_CHECK(echo->text() == "helloa" && echo->cursorPosition() == 6);
  echo->setCursorPosition(0);
  type(echo, Qt::Key_X, QStringLiteral("x"));
  NVE_CHECK(echo->text() == "xhelloa" && echo->cursorPosition() == 1);  // the echo did not move the caret
  NVE_CHECK(app.count(NVE_EVENT_TEXT_CHANGE, kEcho) == 2u);
  // Max length: QLineEdit refuses beyond 12 chars, so no TextChange either.
  for (int i = 0; i < 8; ++i) type(echo, Qt::Key_Z, QStringLiteral("z"));
  NVE_CHECK(echo->text().size() == 12 && app.last(NVE_EVENT_TEXT_CHANGE)->text.size() == 12u);
  // App-driven value replaces the text and is not reported back.
  const size_t edits = app.count(NVE_EVENT_TEXT_CHANGE, kEcho);
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "reset");
  flush(host);
  NVE_CHECK(echo->text() == "reset" && app.count(NVE_EVENT_TEXT_CHANGE, kEcho) == edits);
  // Submit (Return) carries the current text.
  type(echo, Qt::Key_Return, QStringLiteral("\r"));
  NVE_CHECK(app.count(NVE_EVENT_SUBMIT, kEcho) == 1u && app.last(NVE_EVENT_SUBMIT)->text == "reset");

  // Transforming app: the widget adopts the transformed value.
  type(upper, Qt::Key_Q, QStringLiteral("q"));
  NVE_CHECK(app.last(NVE_EVENT_TEXT_CHANGE)->text == "q" && upper->text() == "Q");
  type(upper, Qt::Key_W, QStringLiteral("w"));
  NVE_CHECK(app.last(NVE_EVENT_TEXT_CHANGE)->text == "Qw" && upper->text() == "QW");
  // Rejecting app (value unchanged): the user's text is kept, not fought.
  reject->setCursorPosition(5);
  type(reject, Qt::Key_S, QStringLiteral("s"));
  NVE_CHECK(app.last(NVE_EVENT_TEXT_CHANGE)->text == "fixeds" && reject->text() == "fixeds");

  // IME: composition is neither dispatched nor clobbered by an app update; the commit is.
  spin();  // window activation is asynchronous
  echo->setFocus();
  spin();
  NVE_CHECK(echo->hasFocus() && app.count(NVE_EVENT_FOCUS, kEcho) == 1u);
  echo->setCursorPosition(5);
  const size_t before_ime = app.count(NVE_EVENT_TEXT_CHANGE, kEcho);
  QInputMethodEvent preedit(QStringLiteral("ni"), {});
  QApplication::sendEvent(echo, &preedit);
  NVE_CHECK(echo->composing() && app.count(NVE_EVENT_TEXT_CHANGE, kEcho) == before_ime);
  (void)w.set_str(kEcho, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "app-wins?");
  flush(host);
  NVE_CHECK(echo->text() == "reset");  // composing: untouched
  QInputMethodEvent commit;
  commit.setCommitString(QStringLiteral("你"));
  QApplication::sendEvent(echo, &commit);
  NVE_CHECK(!echo->composing() && app.last(NVE_EVENT_TEXT_CHANGE)->text == "reset你");
  NVE_CHECK(echo->text() == "reset你");

  // Multi-line: edits dispatch, host writes do not, max length is enforced by the host.
  notes->moveCursor(QTextCursor::End);
  notes->insertPlainText(QStringLiteral("\nd"));
  NVE_CHECK(app.last(NVE_EVENT_TEXT_CHANGE)->ev.node_id == kNotes && app.last(NVE_EVENT_TEXT_CHANGE)->text == "a\nb c\nd");
  notes->insertPlainText(QStringLiteral("efgh"));
  NVE_CHECK(notes->toPlainText() == "a\nb c\nde" && app.last(NVE_EVENT_TEXT_CHANGE)->text == "a\nb c\nde");
  const size_t notes_edits = app.count(NVE_EVENT_TEXT_CHANGE, kNotes);
  (void)w.set_str(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "x\ny");
  flush(host);
  NVE_CHECK(notes->toPlainText() == "x\ny" && app.count(NVE_EVENT_TEXT_CHANGE, kNotes) == notes_edits);

  // Focus / blur round-trip.
  reject->setFocus(Qt::MouseFocusReason);
  spin();
  NVE_CHECK(app.count(NVE_EVENT_FOCUS, kReject) == 1u && app.count(NVE_EVENT_BLUR, kEcho) >= 1u);
  reject->clearFocus();
  spin();
  NVE_CHECK(app.count(NVE_EVENT_BLUR, kReject) == 1u);
  widget.hide();
}

void test_scroll_views() {
  constexpr uint32_t kScroll = 2u, kTall = 3u, kOverflow = 4u, kWide = 5u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kScroll);
  w.attach(kScroll, NVE_TABLE_SCROLL_DESC);
  w.set(kScroll, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(100.0f));
  w.set_f32(kScroll, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0.0f);
  subscribe(w, kScroll, {NVE_EVENT_SCROLL});
  w.create(kTall);
  w.set(kTall, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(300.0f));
  w.set_f32(kTall, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0.0f);
  const uint32_t tall[] = {kTall};
  w.set_children(kScroll, tall);
  w.create(kOverflow);  // a plain view with overflow: scroll is a scroll view too
  w.set(kOverflow, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OVERFLOW_X, NVE_OVERFLOW_SCROLL);
  w.set(kOverflow, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(40.0f));
  w.create(kWide);
  size_node(w, kWide, 900.0f, 10.0f);
  const uint32_t wide[] = {kWide};
  w.set_children(kOverflow, wide);
  const uint32_t kids[] = {kScroll, kOverflow};
  w.set_children(1u, kids);

  NveQtWidget widget(QtHostConfig{.width = 320.0f, .height = 240.0f});
  QtHost& host = widget.host();
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  widget.show();
  QWidget* scroll_frame = widget.nativeWidgetForNodeId(kScroll);
  auto* area = scroll_frame->findChild<QScrollArea*>();
  auto* overflow = widget.nativeWidgetForNodeId(kOverflow)->findChild<QScrollArea*>();
  NVE_CHECK(area != nullptr && overflow != nullptr);
  NVE_CHECK(widget.nativeWidgetForNodeId(kTall)->parentWidget() == area->widget());
  NVE_CHECK(area->widget()->height() == 300 && area->geometry() == QRect(0, 0, 320, 100));
  NVE_CHECK(area->horizontalScrollBarPolicy() == Qt::ScrollBarAlwaysOff &&
            area->verticalScrollBarPolicy() == Qt::ScrollBarAsNeeded);
  NVE_CHECK(overflow->horizontalScrollBarPolicy() == Qt::ScrollBarAsNeeded &&
            overflow->verticalScrollBarPolicy() == Qt::ScrollBarAlwaysOff);
  NVE_CHECK(overflow->widget()->width() == 900);

  // User scroll -> Scroll event with offset + content size, and the runtime offset follows.
  QScrollBar* bar = area->verticalScrollBar();
  NVE_CHECK(bar->maximum() >= 199);
  bar->setValue(50);
  NVE_CHECK(app.count(NVE_EVENT_SCROLL, kScroll) == 1u);
  const RecordedEvent* ev = app.last(NVE_EVENT_SCROLL);
  NVE_CHECK(approx(ev->ev.y, 50.0, 0.5) && approx(ev->ev.height, 300.0, 1.0));
  const auto runtime_offset = host.scroll_by(kScroll, 0.0f, 0.0f);
  NVE_CHECK(runtime_offset && approx(runtime_offset->y(), 50.0, 0.5) && approx(runtime_offset->x(), 0.0, 0.01));
  NVE_CHECK(!host.scroll_by(kTall, 0.0f, 1.0f));                       // not a scroll container
  NVE_CHECK(!host.scroll_by(kScroll, std::nanf(""), 0.0f));          // non-finite delta refused
  spin();  // queued wakes from the offset change: no Scroll echo
  NVE_CHECK(app.count(NVE_EVENT_SCROLL, kScroll) == 1u);

  // Programmatic scroll is applied once and not echoed back as a user scroll.
  w.set_f32(kScroll, NVE_TABLE_SCROLL_DESC, NVE_TABLE_SCROLL_REQUESTED_SCROLL_Y, 120.0f);
  flush(host);
  NVE_CHECK(bar->value() == 120 && app.count(NVE_EVENT_SCROLL, kScroll) == 1u);
  NVE_CHECK(approx(host.scroll_by(kScroll, 0.0f, 0.0f)->y(), 120.0, 0.5));
  bar->setValue(10);  // the user scrolls away; an unrelated frame keeps it
  NVE_CHECK(app.count(NVE_EVENT_SCROLL, kScroll) == 2u);
  w.set(kTall, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(9, 9, 9));
  flush(host);
  NVE_CHECK(bar->value() == 10);
  // An out-of-range target clamps to the content.
  w.set_f32(kScroll, NVE_TABLE_SCROLL_DESC, NVE_TABLE_SCROLL_REQUESTED_SCROLL_Y, 5000.0f);
  flush(host);
  NVE_CHECK(bar->value() == bar->maximum() && app.count(NVE_EVENT_SCROLL, kScroll) == 2u);

  // A growing child grows the scroll extent without touching the scroll node.
  w.set(kTall, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(500.0f));
  flush(host);
  NVE_CHECK(area->widget()->height() == 500 && bar->maximum() >= 399);
  // Shrinking clamps the bar without reporting a user scroll.
  w.set(kTall, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(50.0f));
  flush(host);
  NVE_CHECK(area->widget()->height() == 100 && bar->value() == 0 && app.count(NVE_EVENT_SCROLL, kScroll) == 2u);
  widget.hide();
}

void test_pointer_cursor_links_and_removal() {
  constexpr uint32_t kHover = 2u, kLink = 3u, kPressLink = 4u, kSelfRemove = 5u, kText = 6u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kHover);
  size_node(w, kHover, 50.0f, 20.0f);
  subscribe(w, kHover, {NVE_EVENT_POINTER_ENTER, NVE_EVENT_POINTER_LEAVE, NVE_EVENT_POINTER_MOVE});
  w.set(kHover, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_CURSOR, NVE_CURSOR_KIND_GRAB);
  text_node(w, kLink, "docs");
  (void)w.set_str(kLink, NVE_TABLE_LINK_DESC, NVE_TABLE_LINK_HREF, "https://example.com/docs");
  text_node(w, kPressLink, "app link");
  (void)w.set_str(kPressLink, NVE_TABLE_LINK_DESC, NVE_TABLE_LINK_HREF, "https://example.com/never");
  subscribe(w, kPressLink, {NVE_EVENT_PRESS});
  w.create(kSelfRemove);
  size_node(w, kSelfRemove, 30.0f, 30.0f);
  subscribe(w, kSelfRemove, {NVE_EVENT_PRESS});
  text_node(w, kText, "plain");
  w.set(kText, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_CURSOR, NVE_CURSOR_KIND_TEXT);
  const uint32_t kids[] = {kHover, kLink, kPressLink, kSelfRemove, kText};
  w.set_children(1u, kids);
  app.on_event = [](ScriptedApp& a, const RecordedEvent& e) {
    if (e.ev.kind == NVE_EVENT_PRESS && e.ev.node_id == kSelfRemove) {
      a.w.remove(kSelfRemove);
      const uint32_t rest[] = {kHover, kLink, kPressLink, kText};
      a.w.set_children(1u, rest);
    }
  };

  UrlSink sink;
  QDesktopServices::setUrlHandler(QStringLiteral("https"), &sink, "open");
  NveQtWidget widget;
  QtHost& host = widget.host();
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  widget.show();
  QWidget* hover = widget.nativeWidgetForNodeId(kHover);
  NVE_CHECK(hover->cursor().shape() == Qt::OpenHandCursor && hover->hasMouseTracking());
  NVE_CHECK(widget.nativeWidgetForNodeId(kText)->cursor().shape() == Qt::IBeamCursor);
  NVE_CHECK(widget.nativeWidgetForNodeId(kLink)->cursor().shape() == Qt::PointingHandCursor);

  // Enter / move / leave with node-local points; a repeated enter is deduplicated.
  QEnterEvent enter(QPointF(3, 4), QPointF(3, 4), hover->mapToGlobal(QPointF(3, 4)));
  QApplication::sendEvent(hover, &enter);
  QApplication::sendEvent(hover, &enter);
  QMouseEvent move(QEvent::MouseMove, QPointF(7, 8), hover->mapToGlobal(QPointF(7, 8)), Qt::NoButton, Qt::NoButton,
                   Qt::NoModifier);
  QApplication::sendEvent(hover, &move);
  QEvent leave(QEvent::Leave);
  QApplication::sendEvent(hover, &leave);
  NVE_CHECK(app.count(NVE_EVENT_POINTER_ENTER, kHover) == 1u && app.count(NVE_EVENT_POINTER_LEAVE, kHover) == 1u);
  NVE_CHECK(app.count(NVE_EVENT_POINTER_MOVE, kHover) == 1u);
  const RecordedEvent* mv = app.last(NVE_EVENT_POINTER_MOVE);
  NVE_CHECK(mv->ev.x == 7.0f && mv->ev.y == 8.0f && app.last(NVE_EVENT_POINTER_ENTER)->ev.x == 3.0f);
  // Unsubscribed nodes send no pointer events.
  QApplication::sendEvent(widget.nativeWidgetForNodeId(kText), &enter);
  NVE_CHECK(app.count(NVE_EVENT_POINTER_ENTER) == 1u);

  // A link without a Press subscription opens natively; with one, the app gets Press instead.
  click(widget.nativeWidgetForNodeId(kLink), QPointF(2, 2));
  NVE_CHECK(sink.opened.size() == 1u && sink.opened[0] == QUrl("https://example.com/docs"));
  NVE_CHECK(widget.accessibilityInfoForNodeId(kLink)->role == QAccessible::Link);
  click(widget.nativeWidgetForNodeId(kPressLink), QPointF(2, 2));
  NVE_CHECK(sink.opened.size() == 1u && app.count(NVE_EVENT_PRESS, kPressLink) == 1u);
  QDesktopServices::unsetUrlHandler(QStringLiteral("https"));

  // A press whose handler removes the pressed node: the widget dies later, safely.
  QPointer<QWidget> doomed = widget.nativeWidgetForNodeId(kSelfRemove);
  click(doomed, QPointF(5, 5));
  NVE_CHECK(widget.nativeWidgetForNodeId(kSelfRemove) == nullptr && widget.viewCount() == 4u);
  spin();
  NVE_CHECK(doomed.isNull());
  widget.hide();
}

}  // namespace

void nve_test_qt_input_and_scroll() {
  test_controlled_text_inputs();
  test_scroll_views();
  test_pointer_cursor_links_and_removal();
}

#include "qt_shell_input_scroll_test.moc"
