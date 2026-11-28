// Windows-only suite: DirectWrite measurement, Direct2D pixels (flat draw lists, rounded clips,
// borders, shadows, text), the per-target paint cache (WIC images: data / file / bytes; UNC and
// remote hosts refused), headless BMP screenshots, and a live (hidden) host window: native EDIT
// overlays (controlled echo, rejecting app, submit, removal), queued wakes, window manager HWNDs.
#include "windows_test_helpers.hpp"

#include "NveWinD2DInternal.hpp"
#include "NveWinWindowManager.hpp"

#include <nve/canvas/draw_list_encoder.hpp>
#include <nve/host/core_resources.hpp>
#include <nve/host/demo_counter_app.hpp>

#include <wincodec.h>
#include <wincrypt.h>

#include <cmath>
#include <filesystem>
#include <limits>
#include <fstream>
#include <iterator>
#include <vector>

namespace {

using namespace nve::win;
using namespace nve::win::test;
using namespace nve::canvas;
namespace h = nve::host;

constexpr std::string_view kPng1x1 =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+X2dIAAAAASUVORK5CYII=";

void pump_messages() {
  MSG msg{};
  for (int i = 0; i < 200 && ::PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE); ++i) {
    ::TranslateMessage(&msg);
    ::DispatchMessageW(&msg);
  }
}

/// Offscreen premultiplied-BGRA WIC bitmap with a Direct2D target (and its paint cache) over it.
struct Offscreen {
  IWICImagingFactory* wic = nullptr;
  IWICBitmap* bitmap = nullptr;
  ID2D1Factory* factory = nullptr;
  ID2D1RenderTarget* rt = nullptr;
  ID2D1SolidColorBrush* brush = nullptr;
  D2DPaintCache cache;
  UINT w, h;

  Offscreen(UINT width, UINT height) : w(width), h(height) {
    (void)::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    NVE_CHECK(SUCCEEDED(::CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&wic))));
    NVE_CHECK(SUCCEEDED(wic->CreateBitmap(w, h, GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad, &bitmap)));
    NVE_CHECK(SUCCEEDED(::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &factory)));
    NVE_CHECK(SUCCEEDED(factory->CreateWicBitmapRenderTarget(bitmap, D2D1::RenderTargetProperties(), &rt)));
    NVE_CHECK(SUCCEEDED(rt->CreateSolidColorBrush(D2D1::ColorF(0.0f, 0.0f, 0.0f), &brush)));
  }
  ~Offscreen() {
    cache.release();
    for (IUnknown* u : {static_cast<IUnknown*>(brush), static_cast<IUnknown*>(rt), static_cast<IUnknown*>(factory),
                        static_cast<IUnknown*>(bitmap), static_cast<IUnknown*>(wic)}) {
      u->Release();
    }
  }
  /// BGRA pixel at `(x, y)` as 0xAARRGGBB.
  uint32_t pixel(UINT x, UINT y) const {
    uint32_t out = 0u;
    const WICRect rect{static_cast<INT>(x), static_cast<INT>(y), 1, 1};
    NVE_CHECK(SUCCEEDED(bitmap->CopyPixels(&rect, 4u, 4u, reinterpret_cast<BYTE*>(&out))));
    return out;
  }
};

void test_directwrite_measurer() {
  using nve::host::str_slice;
  using nve::host::text_style;
  constexpr float kInf = std::numeric_limits<float>::infinity();
  DirectWriteTextMeasurer m;
  NVE_CHECK(m.valid());
  const NveIntrinsicSize regular = m.measure("Hello NVE", text_style(16.0f), kInf);
  const NveIntrinsicSize big = m.measure("Hello NVE", text_style(32.0f), kInf);
  const NveIntrinsicSize bold = m.measure("Hello NVE", text_style(16.0f, 800u), kInf);
  const NveIntrinsicSize wrapped = m.measure("Hello NVE wrapping text", text_style(16.0f), 40.0f);
  NVE_CHECK(regular.width > 0.0f && regular.height > 0.0f);
  NVE_CHECK(big.width > regular.width * 1.5f && big.height > regular.height);
  NVE_CHECK(bold.width >= regular.width);
  NVE_CHECK(wrapped.width <= 40.0f && wrapped.height > regular.height * 1.5f);
  NVE_CHECK(m.measure("", text_style(16.0f), kInf).width == 0.0f);
  NVE_CHECK(m.measure("x", text_style(1.0e30f), 1.0e30f).width > 0.0f);  // huge sizes clamp, never overflow
  float w = 0.0f, ht = 0.0f;
  const std::string_view text = "\xC3\xA9t\xC3\xA9";  // UTF-8 through the C trampoline
  const NveRuntimeTextStyle base = text_style(16.0f);
  DirectWriteTextMeasurer::measure_trampoline(&m, str_slice(text), &base, kInf, &w, &ht);
  NVE_CHECK(w > 0.0f && ht > 0.0f);

  // The node's typography reaches DirectWrite exactly as the painter applies it.
  constexpr std::string_view kNarrow = "iiiiiiiiiiiiiiii";
  NveRuntimeTextStyle mono = base;
  mono.font_family = str_slice("NoSuchFontNve, monospace");  // first installed: Consolas
  NVE_CHECK(m.measure(kNarrow, mono, kInf).width > m.measure(kNarrow, base, kInf).width * 1.5f);
  NveRuntimeTextStyle spaced = base;
  spaced.letter_spacing = 3.0f;
  NVE_CHECK(m.measure(kNarrow, spaced, kInf).width > m.measure(kNarrow, base, kInf).width + 30.0f);
  const std::string_view sentence = "Hello wrapping text that is long";
  const NveIntrinsicSize one_line = m.measure(sentence, base, kInf);
  NveRuntimeTextStyle nowrap = base;
  nowrap.white_space = NVE_WHITE_SPACE_NO_WRAP;
  NVE_CHECK(m.measure(sentence, nowrap, 60.0f).height == one_line.height);
  const NveIntrinsicSize min_content = m.measure(sentence, base, 0.0f);
  NVE_CHECK(min_content.width > 0.0f && min_content.width < one_line.width && min_content.height > one_line.height);
  // CSS Text 3 min-content parity across `Word`, `BreakWord`, `Anywhere`, `BreakAll`, and `None`.
  constexpr std::string_view kPhrase = "considerably long";
  const float full_w = m.measure(kPhrase, base, kInf).width;
  const float word_w = m.measure("considerably", base, kInf).width;
  const float word_h = m.measure("considerably", base, kInf).height;
  const NveIntrinsicSize mc_word = m.measure(kPhrase, base, 0.0f);
  NveRuntimeTextStyle break_word = base;
  break_word.overflow_wrap = NVE_OVERFLOW_WRAP_BREAK_WORD;
  const NveIntrinsicSize mc_break_word = m.measure(kPhrase, break_word, 0.0f);
  NveRuntimeTextStyle word_break_word = base;
  word_break_word.word_break = NVE_WORD_BREAK_BREAK_WORD;
  const NveIntrinsicSize mc_wb_word = m.measure(kPhrase, word_break_word, 0.0f);
  NveRuntimeTextStyle anywhere = base;
  anywhere.overflow_wrap = NVE_OVERFLOW_WRAP_ANYWHERE;
  const NveIntrinsicSize mc_anywhere = m.measure(kPhrase, anywhere, 0.0f);
  NveRuntimeTextStyle break_all = base;
  break_all.word_break = NVE_WORD_BREAK_BREAK_ALL;
  const NveIntrinsicSize mc_break_all = m.measure(kPhrase, break_all, 0.0f);
  NveRuntimeTextStyle pre = base;
  pre.white_space = NVE_WHITE_SPACE_PRE;
  NVE_CHECK(std::fabs(mc_word.width - word_w) <= 1.0f);
  NVE_CHECK(std::fabs(mc_break_word.width - word_w) <= 1.0f);
  // At a positive narrow width, `overflow-wrap: break-word` emergency-breaks onto multiple lines
  // while `normal` (`Word`) keeps the single over-long word on one line.
  const NveIntrinsicSize narrow_normal = m.measure("considerably", base, 30.0f);
  const NveIntrinsicSize narrow_break_word = m.measure("considerably", break_word, 30.0f);
  NVE_CHECK(narrow_normal.height <= word_h + 1.0f);
  NVE_CHECK(narrow_break_word.width <= 30.0f && narrow_break_word.height > word_h * 1.5f);
  NVE_CHECK(mc_wb_word.width > 0.0f && mc_wb_word.width < word_w);
  NVE_CHECK(mc_anywhere.width > 0.0f && mc_anywhere.width < word_w);
  NVE_CHECK(mc_break_all.width > 0.0f && mc_break_all.width < word_w);
  NVE_CHECK(std::fabs(m.measure(kPhrase, nowrap, 0.0f).width - full_w) <= 1.0f);
  NVE_CHECK(std::fabs(m.measure(kPhrase, pre, 0.0f).width - full_w) <= 1.0f);
  NveRuntimeTextStyle tall = base;
  tall.line_height = 250.0f;
  tall.line_height_unit = NVE_LINE_HEIGHT_PERCENT;  // 40pt lines
  NVE_CHECK(std::fabs(m.measure("a\nb", tall, kInf).height - 80.0f) <= 1.0f);
  tall.max_lines = 1u;
  NVE_CHECK(std::fabs(m.measure(sentence, tall, 60.0f).height - 40.0f) <= 1.0f);
  TextPaint clamped;
  clamped.max_lines = 1u;
  IDWriteTextLayout* layout = m.create_layout("one two three four five six", clamped, 40.0f, 0.0f);
  NVE_CHECK(layout != nullptr && layout->GetMaxHeight() < regular.height * 1.5f);
  layout->Release();
}

void test_draw_list_pixels() {
  const DrawPoint line[] = {{0.0f, 18.0f}, {39.0f, 18.0f}};
  DrawListEncoder e;
  e.clear(DrawColorRgba::rgb(255, 255, 255))
      .fill_rect(0.0f, 0.0f, 10.0f, 10.0f, DrawColorRgba::rgb(255, 0, 0))
      .clip_rect(20.0f, 0.0f, 10.0f, 10.0f)
      .fill_rect(15.0f, 0.0f, 20.0f, 10.0f, DrawColorRgba::rgb(0, 0, 255))
      .restore_clip()
      .fill_circle(35.0f, 5.0f, 3.0f, DrawColorRgba::rgb(0, 255, 0))
      .stroke_polyline(line, 2.0f, DrawColorRgba::rgb(0, 0, 0));
  const std::vector<uint8_t> bytes = e.finish();
  Offscreen s(40u, 20u);
  s.rt->BeginDraw();
  s.rt->Clear(D2D1::ColorF(0.0f, 0.0f, 0.0f, 0.0f));
  const DrawListDecodeResult res = render_draw_list_to_d2d(s.rt, s.brush, s.factory, D2D1::RectF(0, 0, 40, 20), bytes);
  std::vector<uint8_t> truncated = bytes;
  truncated.resize(truncated.size() - 4u);
  NVE_CHECK(render_draw_list_to_d2d(s.rt, s.brush, s.factory, D2D1::RectF(0, 0, 40, 20), truncated).error ==
            DrawListError::Truncated);
  std::vector<uint8_t> unaligned = bytes;
  unaligned.pop_back();
  NVE_CHECK(render_draw_list_to_d2d(s.rt, s.brush, s.factory, D2D1::RectF(0, 0, 40, 20), unaligned).error ==
            DrawListError::Unaligned);
  DrawListEncoder unbalanced;
  unbalanced.clip_rect(0.0f, 0.0f, 1.0f, 1.0f);
  NVE_CHECK(render_draw_list_to_d2d(s.rt, s.brush, s.factory, D2D1::RectF(0, 0, 40, 20), unbalanced.finish()).ok());
  NVE_CHECK(SUCCEEDED(s.rt->EndDraw()));  // a leaked clip would fail EndDraw
  NVE_CHECK(res.ok() && res.ops_decoded == 7u);
  NVE_CHECK(s.pixel(5, 5) == 0xFFFF0000u);    // red rect
  NVE_CHECK(s.pixel(17, 5) == 0xFFFFFFFFu);   // blue clipped away
  NVE_CHECK(s.pixel(25, 5) == 0xFF0000FFu);   // blue inside the clip
  NVE_CHECK(s.pixel(35, 5) == 0xFF00FF00u);   // circle centre
  NVE_CHECK(s.pixel(20, 18) == 0xFF000000u);  // polyline

  // 66 nested clips + 2 restores: overflow absorbs clips #65/#66 so clip #64 stays active.
  DrawListEncoder deep;
  deep.clear(DrawColorRgba::rgb(0, 0, 0));
  for (uint32_t i = 0u; i < kMaxClipDepth - 1u; ++i) deep.clip_rect(0.0f, 0.0f, 40.0f, 20.0f);
  deep.clip_rect(0.0f, 0.0f, 10.0f, 10.0f)
      .clip_rect(0.0f, 0.0f, 2.0f, 2.0f)
      .clip_rect(0.0f, 0.0f, 1.0f, 1.0f)
      .restore_clip()
      .restore_clip()
      .fill_rect(0.0f, 0.0f, 40.0f, 20.0f, DrawColorRgba::rgb(255, 0, 0))
      .restore_clip()
      .fill_rect(20.0f, 5.0f, 10.0f, 10.0f, DrawColorRgba::rgb(0, 0, 255));
  s.rt->BeginDraw();
  NVE_CHECK(render_draw_list_to_d2d(s.rt, s.brush, s.factory, D2D1::RectF(0, 0, 40, 20), deep.finish()).ok());
  NVE_CHECK(SUCCEEDED(s.rt->EndDraw()));
  NVE_CHECK(s.pixel(5, 5) == 0xFFFF0000u);
  NVE_CHECK(s.pixel(35, 5) == 0xFF000000u);
  NVE_CHECK(s.pixel(25, 10) == 0xFF0000FFu);
}

void place(h::Nve2Writer& w, uint32_t id, float x, float y, float width, float height) {
  w.create(id);
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_POSITION, NVE_POSITION_ABSOLUTE);
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_INSET_LEFT, h::length_points(x));
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_INSET_TOP, h::length_points(y));
  sized(w, id, width, height);
}

/// Scene pixels through the real painter: rounded background clip, border, hard shadow, text.
void test_scene_pixels() {
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.set(1u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 255, 255));
  place(w, 2u, 10.0f, 10.0f, 40.0f, 40.0f);  // red disc (radius = half the size)
  w.set(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(255, 0, 0));
  w.set_f32(2u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 20.0f);
  place(w, 3u, 60.0f, 10.0f, 40.0f, 40.0f);  // 4pt blue border, no fill
  w.set_f32(3u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_WIDTH, 4.0f);
  w.set(3u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_COLOR, h::rgba(0, 0, 255));
  place(w, 4u, 110.0f, 10.0f, 30.0f, 30.0f);  // green box with a hard black shadow 10pt below
  w.set(4u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(0, 255, 0));
  const h::BoxShadow shadow{0.0f, 10.0f, 0.0f, 0.0f, h::rgba(0, 0, 0), false};
  w.define_resource(50u, h::kBoxShadowsKind, h::encode_box_shadows(std::span(&shadow, 1u)));
  w.set(4u, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 50u);
  place(w, 5u, 10.0f, 60.0f, 120.0f, 30.0f);  // text
  (void)w.set_str(5u, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "Hello NVE");
  const uint32_t kids[] = {2u, 3u, 4u, 5u};
  w.set_children(1u, kids);
  const auto host_owner = make_host(WinHostConfig{.width = 200.0f, .height = 100.0f});
  NveWinHost& host = *host_owner;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  Offscreen s(200u, 100u);
  NVE_CHECK(SUCCEEDED(paint_visual_tree_d2d(s.rt, s.factory, host.visual_tree(), host.text_measurer(), s.cache)));
  NVE_CHECK(s.pixel(30, 30) == 0xFFFF0000u && s.pixel(12, 12) == 0xFFFFFFFFu);  // rounded: corner stays white
  NVE_CHECK(s.pixel(61, 30) == 0xFF0000FFu && s.pixel(80, 30) == 0xFFFFFFFFu);  // border only
  NVE_CHECK(s.pixel(125, 25) == 0xFF00FF00u && s.pixel(125, 45) == 0xFF000000u && s.pixel(125, 55) == 0xFFFFFFFFu);
  int ink = 0;  // DirectWrite drew the label
  for (UINT y = 60u; y < 90u; ++y) {
    for (UINT x = 10u; x < 130u; ++x) ink += (s.pixel(x, y) & 0xFFu) < 0x80u ? 1 : 0;
  }
  NVE_CHECK(ink > 40);
  // A dirty-rect repaint leaves everything outside it untouched.
  const Rect dirty{0.0f, 0.0f, 55.0f, 55.0f};
  NVE_CHECK(SUCCEEDED(paint_visual_tree_d2d(s.rt, s.factory, host.visual_tree(), host.text_measurer(), s.cache, &dirty)));
  NVE_CHECK(s.pixel(30, 30) == 0xFFFF0000u && s.pixel(61, 30) == 0xFF0000FFu);
}

void test_paint_cache_images() {
  Offscreen s(4u, 4u);
  WinImageSource url;
  url.url = url.key = std::string(kPng1x1);
  ID2D1Bitmap* bmp = s.cache.bitmap(s.rt, url);
  NVE_CHECK(bmp != nullptr && bmp->GetSize().width == 1.0f && s.cache.bitmap(s.rt, url) == bmp);  // cached
  WinImageSource broken;
  broken.url = broken.key = "data:image/png;base64,AAAA";
  NVE_CHECK(s.cache.bitmap(s.rt, broken) == nullptr);
  // Remote, UNC and remote-host file URLs are never opened (no fetch, no NTLM handshake).
  for (const char* refused : {"https://example.com/a.png", "\\\\localhost\\c$\\a.png", "//localhost/c$/a.png",
                              "file://server/share/a.png", "file:////server/share/a.png"}) {
    WinImageSource r;
    r.url = r.key = refused;
    NVE_CHECK(s.cache.bitmap(s.rt, r) == nullptr);
  }
  // The same PNG as a local file URL (percent-encoded) and as inline bytes.
  const std::string b64(kPng1x1.substr(kPng1x1.find(',') + 1u));
  DWORD len = 0u;
  NVE_CHECK(::CryptStringToBinaryA(b64.c_str(), 0, CRYPT_STRING_BASE64, nullptr, &len, nullptr, nullptr));
  std::vector<uint8_t> png(len);
  NVE_CHECK(::CryptStringToBinaryA(b64.c_str(), 0, CRYPT_STRING_BASE64, png.data(), &len, nullptr, nullptr));
  const ScratchDir scratch("nve_win_d2d_images");
  const std::string path = scratch.file("test image.png");
  std::ofstream(path, std::ios::binary).write(reinterpret_cast<const char*>(png.data()), static_cast<std::streamsize>(png.size()));
  WinImageSource file;
  file.url = "file:///" + std::filesystem::path(path).generic_string();
  for (size_t at = file.url.find(' '); at != std::string::npos; at = file.url.find(' ')) file.url.replace(at, 1u, "%20");
  file.key = file.url;
  NVE_CHECK(s.cache.bitmap(s.rt, file) != nullptr);
  NVE_CHECK(s.cache.bitmap(s.rt, WinImageSource::from(h::ImageSource::from_encoded(png))) != nullptr);
  s.cache.release();
  NVE_CHECK(s.cache.bitmap(s.rt, url) != nullptr);  // re-decoded after a release
}

void test_screenshot_bmp() {
  const auto host_owner = make_host(WinHostConfig{.width = 320.0f, .height = 200.0f});
  NveWinHost& host = *host_owner;
  NVE_CHECK(host.attach_interface(&h::demo::CounterApp::get_interface));
  const ScratchDir scratch("nve_win_bmp");
  const std::string out = scratch.file("counter.bmp");
  std::string error;
  NVE_CHECK(render_visual_tree_to_bmp(host.visual_tree(), host.text_measurer(), 640u, 400u, 2.0f, out, &error));
  std::ifstream in(out, std::ios::binary);
  const std::vector<uint8_t> file{std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>()};
  NVE_CHECK(file.size() == 54u + 640u * 400u * 4u && file[0] == 'B' && file[1] == 'M');
  const uint8_t* px = file.data() + 54u + 4u * 639u;  // bottom-up rows: bottom-right pixel
  NVE_CHECK(px[0] == 252u && px[1] == 250u && px[2] == 248u);  // column background rgb(248, 250, 252)
  NVE_CHECK(!render_visual_tree_to_bmp(host.visual_tree(), host.text_measurer(), 0u, 10u, 1.0f, out, &error));
}

void test_window_inputs_and_wakes() {
  constexpr uint32_t kInput = 2u, kNumber = 3u;
  ScriptedApp app;
  app.w.create(1u);
  app.w.create(kInput);
  (void)app.w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "hi");
  events(app.w, kInput, {NVE_TEXT_INPUT_EVENT_CHANGE, NVE_TEXT_INPUT_EVENT_SUBMIT});
  events(app.w, kInput, {NVE_INTERACTION_EVENT_FOCUS, NVE_INTERACTION_EVENT_BLUR});
  sized(app.w, kInput, 160.0f, 24.0f);
  app.w.create(kNumber);  // keeps digits only: rejects other edits without republishing
  (void)app.w.set_str(kNumber, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "1");
  events(app.w, kNumber, {NVE_TEXT_INPUT_EVENT_CHANGE});
  sized(app.w, kNumber, 160.0f, 24.0f);
  const uint32_t kids[] = {kInput, kNumber};
  app.w.set_children(1u, kids);
  std::string number = "1";
  app.on_event = [&number](ScriptedApp& a, const h::testing::RecordedEvent& e) {
    if (!h::is_text_input_event(e.ev, NVE_TEXT_INPUT_EVENT_CHANGE)) return;
    if (e.ev.node == kInput) {
      std::string upper = e.text;
      for (char& c : upper) c = static_cast<char>(c >= 'a' && c <= 'z' ? c - 32 : c);
      (void)a.w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, upper);
      return;
    }
    std::string digits;
    for (const char c : e.text) {
      if (c >= '0' && c <= '9') digits.push_back(c);
    }
    if (digits == number) return;  // rejected
    number = digits;
    (void)a.w.set_str(kNumber, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, number);
  };

  NveWinHostWindow window(WinHostWindowOptions{.title = "test", .width = 320, .height = 200, .quit_on_destroy = false});
  std::string error;
  NVE_CHECK(window.create(::GetModuleHandleW(nullptr), &error));
  NVE_CHECK(window.host().attach_interface(&ScriptedApp::get_interface));
  pump_messages();
  HWND edit = window.text_inputs().control_for_node(kInput);
  HWND digits = window.text_inputs().control_for_node(kNumber);
  NVE_CHECK(edit != nullptr && WinTextInputs::control_text(edit) == "hi" && WinTextInputs::control_text(digits) == "1");
  ::SetWindowTextW(edit, L"typed");  // EN_CHANGE -> TextChange -> app echo
  pump_messages();
  NVE_CHECK(app.last(NVE_TEXT_INPUT_EVENT_CHANGE)->text == "typed" &&
            app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kInput) == 1u);
  NVE_CHECK(WinTextInputs::control_text(edit) == "TYPED" && node(window.host(), kInput).input.value == "TYPED");
  ::SendMessageW(edit, WM_KEYDOWN, VK_RETURN, 0);
  NVE_CHECK(app.last(NVE_TEXT_INPUT_EVENT_SUBMIT)->text == "TYPED");
  // A rejected edit reverts at once; an accepted one stays; write-backs never echo upstream.
  ::SetWindowTextW(digits, L"1a");
  NVE_CHECK(WinTextInputs::control_text(digits) == "1" && app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kNumber) == 1u);
  ::SetWindowTextW(digits, L"12");
  pump_messages();
  NVE_CHECK(WinTextInputs::control_text(digits) == "12" && number == "12" &&
            app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kNumber) == 2u);
  // An app-side change arrives through a queued wake.
  (void)app.w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "from app");
  (void)window.host().session().dispatch(ScriptedApp::flush_event(window.surface_id()));
  pump_messages();
  NVE_CHECK(WinTextInputs::control_text(edit) == "from app");
  // Removing the node destroys its control (its EN_KILLFOCUS is not user input).
  ::SetFocus(edit);
  const size_t blurs = app.count(NVE_INTERACTION_EVENT_BLUR, kInput);
  app.w.remove(kInput);
  const uint32_t rest[] = {kNumber};
  app.w.set_children(1u, rest);
  (void)window.host().session().dispatch(ScriptedApp::flush_event(window.surface_id()));
  (void)window.host().pump();
  pump_messages();
  NVE_CHECK(window.text_inputs().size() == 1u && !::IsWindow(edit) &&
            app.count(NVE_INTERACTION_EVENT_BLUR, kInput) == blurs);
  window.close();
  pump_messages();
}

void test_window_manager_windows() {
  NveWinWindowManager mgr;
  NveWinHostWindow* a = mgr.open_window(::GetModuleHandleW(nullptr), {.surface_id = 1u, .title = "A"}, false);
  NveWinHostWindow* b = mgr.open_window(::GetModuleHandleW(nullptr), {.surface_id = 2u, .title = "B", .parent_surface_id = 1u, .modal = true}, false);
  NVE_CHECK(a != nullptr && b != nullptr && mgr.window_for_surface(2u) == b && &a->host() == mgr.host_for_surface(1u));
  NVE_CHECK(a->host().attach_interface(&h::demo::CounterApp::get_interface));
  // set_bounds takes the client size in DIPs: the client area matches it at any DPI.
  a->set_bounds(10, 10, 300, 200);
  RECT client{};
  NVE_CHECK(::GetClientRect(a->hwnd(), &client));
  NVE_CHECK(std::abs(static_cast<float>(client.right) / a->dpi_scale() - 300.0f) <= 1.0f);
  ::SendMessageW(b->hwnd(), WM_CLOSE, 0, 0);  // user close: the window manager drops it from WM_DESTROY
  pump_messages();
  NVE_CHECK(mgr.window_count() == 1u && mgr.window_for_surface(2u) == nullptr);
  NVE_CHECK(mgr.request_close(1u) && mgr.window_count() == 0u && mgr.quit_posted());
  MSG quit{};
  (void)::PeekMessageW(&quit, nullptr, WM_QUIT, WM_QUIT, PM_REMOVE);  // swallow the manager's WM_QUIT
}

/// Minimal C ABI app with one text input (node 2) that emits an outbound custom event on
/// `NVE_TEXT_INPUT_EVENT_CHANGE`, so the host's event sink can destroy the `NveWinHostWindow` mid-`EN_CHANGE`.
struct CloseOnEditApp {
  static constexpr uint32_t kInput = 2u;
  h::Nve2Writer w;
  h::OwnedEvents queued;
  h::OwnedEvents drained;
  uint32_t surface_id = 1u;

  static NveAppHandle create(uint32_t sid) {
    auto* app = new CloseOnEditApp();
    app->surface_id = sid;
    app->w.create(1u);
    app->w.create(kInput);
    (void)app->w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "open");
    events(app->w, kInput, {NVE_TEXT_INPUT_EVENT_CHANGE});
    sized(app->w, kInput, 160.0f, 24.0f);
    const uint32_t kids[] = {kInput};
    app->w.set_children(1u, kids);
    return app;
  }
  static void destroy(NveAppHandle handle) { delete static_cast<CloseOnEditApp*>(handle); }
  static NveBufferSlice render(NveAppHandle handle, uint64_t frame_id) {
    auto* app = static_cast<CloseOnEditApp*>(handle);
    if (!app->w.pending()) return NveBufferSlice{nullptr, 0u};
    const std::span<const uint8_t> frame = app->w.finish(frame_id);
    return NveBufferSlice{frame.data(), frame.size()};
  }
  static uint32_t dispatch_events(NveAppHandle handle, NveEventBatch batch) {
    auto* app = static_cast<CloseOnEditApp*>(handle);
    for (const NveEvent& e : h::batch_events(batch)) {
      if (e.table == NVE_TABLE_TEXT_INPUT_KEY && e.event == NVE_TEXT_INPUT_EVENT_CHANGE && e.node == kInput) {
        (void)app->queued.push_custom(app->surface_id, kInput, 0u, "close", {});
      }
    }
    return 0u;
  }
  static uint32_t set_props(NveAppHandle, const NveMapEntry*, uint32_t) { return 0u; }
  static NveEventBatch drain_emitted_events(NveAppHandle handle) {
    auto* app = static_cast<CloseOnEditApp*>(handle);
    app->drained = std::move(app->queued);
    app->queued.clear();
    return app->drained.batch();
  }
  static const void* get_interface(uint32_t iface) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    static const NveComponentApi kComp{&set_props, &drain_emitted_events};
    if (iface == NVE_IFACE_SURFACE) return &kSurface;
    if (iface == NVE_IFACE_COMPONENT) return &kComp;
    return nullptr;
  }
};

/// Destroying the owning `NveWinHostWindow` (and its `WinTextInputs`) synchronously inside an
/// `EN_CHANGE` dispatch must not use-after-free `WinTextInputs` when `dispatch_controlled_edit`
/// queries the post-dispatch answer.
void test_destroy_window_during_en_change_dispatch() {
  auto window = std::make_unique<NveWinHostWindow>(
      WinHostWindowOptions{.title = "uaf-guard", .width = 320, .height = 200, .quit_on_destroy = false});
  std::string error;
  NVE_CHECK(window->create(::GetModuleHandleW(nullptr), &error));
  bool destroyed_in_callback = false;
  window->host().set_event_callback([&](NveEventBatch batch) {
    if (!h::batch_events(batch).empty() && window != nullptr) {
      destroyed_in_callback = true;
      window.reset();
    }
  });
  NVE_CHECK(window->host().attach_interface(&CloseOnEditApp::get_interface));
  pump_messages();
  HWND edit = window->text_inputs().control_for_node(CloseOnEditApp::kInput);
  NVE_CHECK(edit != nullptr && ::IsWindow(edit));
  // Raises EN_CHANGE synchronously -> event sink destroys `window` mid-dispatch.
  ::SetWindowTextW(edit, L"close");
  pump_messages();
  NVE_CHECK(destroyed_in_callback && window == nullptr && !::IsWindow(edit));
}

}  // namespace

void nve_test_win_direct2d() {
  test_directwrite_measurer();
  test_draw_list_pixels();
  test_scene_pixels();
  test_paint_cache_images();
  test_screenshot_bmp();
  test_window_inputs_and_wakes();
  test_destroy_window_during_en_change_dispatch();
  test_window_manager_windows();
}
