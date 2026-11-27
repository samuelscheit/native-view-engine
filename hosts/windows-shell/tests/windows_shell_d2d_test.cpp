// Windows-only suite: DirectWrite measurement, Direct2D `NVE_DRAW1` pixels, WIC image decoding,
// headless BMP screenshots, and a live (hidden) host window: native EDIT overlays (controlled
// echo, submit), queued runtime wakes, and the window manager with real HWNDs.
#include "windows_test_helpers.hpp"

#include "NveWinD2DInternal.hpp"
#include "NveWinWindowManager.hpp"

#include <nve/canvas/draw_list_encoder.hpp>
#include <nve/host/core_resources.hpp>
#include <nve/host/demo_counter_app.hpp>

#include <wincodec.h>
#include <wincrypt.h>

#include <filesystem>
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

/// Offscreen premultiplied-BGRA WIC bitmap with a Direct2D target over it.
struct Offscreen {
  IWICImagingFactory* wic = nullptr;
  IWICBitmap* bitmap = nullptr;
  ID2D1Factory* factory = nullptr;
  ID2D1RenderTarget* rt = nullptr;
  ID2D1SolidColorBrush* brush = nullptr;
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
    d2d_internal::release_bitmaps(rt);
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
  DirectWriteTextMeasurer m;
  NVE_CHECK(m.valid());
  const NveIntrinsicSize regular = m.measure("Hello NVE", 16.0f, 400u, 0.0f);
  const NveIntrinsicSize big = m.measure("Hello NVE", 32.0f, 400u, 0.0f);
  const NveIntrinsicSize bold = m.measure("Hello NVE", 16.0f, 800u, 0.0f);
  const NveIntrinsicSize wrapped = m.measure("Hello NVE wrapping text", 16.0f, 400u, 40.0f);
  NVE_CHECK(regular.width > 0.0f && regular.height > 0.0f);
  NVE_CHECK(big.width > regular.width * 1.5f && big.height > regular.height);
  NVE_CHECK(bold.width >= regular.width);
  NVE_CHECK(wrapped.width <= 40.0f && wrapped.height > regular.height * 1.5f);
  NVE_CHECK(m.measure("", 16.0f, 400u, 0.0f).width == 0.0f);
  float w = 0.0f, ht = 0.0f;
  const std::string_view text = "\xC3\xA9t\xC3\xA9";  // UTF-8 through the C trampoline
  DirectWriteTextMeasurer::measure_trampoline(&m, NveStrSlice{reinterpret_cast<const uint8_t*>(text.data()), text.size()},
                                              16.0f, 400u, 0.0f, &w, &ht);
  NVE_CHECK(w > 0.0f && ht > 0.0f);
  TextPaint clamped;
  clamped.max_lines = 1u;
  IDWriteTextLayout* layout = m.create_layout("one two three four five six", clamped, 40.0f, 0.0f);
  NVE_CHECK(layout != nullptr);
  DWRITE_TEXT_METRICS metrics{};
  NVE_CHECK(SUCCEEDED(layout->GetMetrics(&metrics)) && layout->GetMaxHeight() < regular.height * 1.5f);
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
  // Corrupt payload: rejected before painting; unbalanced clips are unwound.
  std::vector<uint8_t> bad = bytes;
  bad.pop_back();
  NVE_CHECK(render_draw_list_to_d2d(s.rt, s.brush, s.factory, D2D1::RectF(0, 0, 40, 20), bad).error == DrawListError::Truncated);
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
}

void test_wic_images() {
  Offscreen s(4u, 4u);
  WinImageSource url;
  url.url = std::string(kPng1x1);
  url.key = url.url;
  ID2D1Bitmap* bmp = d2d_internal::bitmap_for(s.rt, url);
  NVE_CHECK(bmp != nullptr && bmp->GetSize().width == 1.0f);
  NVE_CHECK(d2d_internal::bitmap_for(s.rt, url) == bmp);  // cached
  WinImageSource broken;
  broken.url = "data:image/png;base64,AAAA";
  broken.key = broken.url;
  NVE_CHECK(d2d_internal::bitmap_for(s.rt, broken) == nullptr);
  WinImageSource remote;
  remote.url = remote.key = "https://example.com/a.png";
  NVE_CHECK(d2d_internal::bitmap_for(s.rt, remote) == nullptr);  // the shell does not fetch
  // The same PNG as a file URL and as inline bytes.
  const std::string b64(kPng1x1.substr(kPng1x1.find(',') + 1u));
  DWORD len = 0u;
  NVE_CHECK(::CryptStringToBinaryA(b64.c_str(), 0, CRYPT_STRING_BASE64, nullptr, &len, nullptr, nullptr));
  std::vector<uint8_t> png(len);
  NVE_CHECK(::CryptStringToBinaryA(b64.c_str(), 0, CRYPT_STRING_BASE64, png.data(), &len, nullptr, nullptr));
  const auto path = std::filesystem::temp_directory_path() / "nve win test image.png";
  std::ofstream(path, std::ios::binary).write(reinterpret_cast<const char*>(png.data()), static_cast<std::streamsize>(png.size()));
  WinImageSource file;
  file.url = "file:///" + path.generic_string();
  for (size_t at = file.url.find(' '); at != std::string::npos; at = file.url.find(' ')) file.url.replace(at, 1u, "%20");
  file.key = file.url;
  NVE_CHECK(d2d_internal::bitmap_for(s.rt, file) != nullptr);
  const WinImageSource bytes = WinImageSource::from(h::ImageSource{{}, png});
  NVE_CHECK(d2d_internal::bitmap_for(s.rt, bytes) != nullptr);
  std::filesystem::remove(path);
}

void test_screenshot_bmp() {
  NveWinHost host(WinHostConfig{.width = 320.0f, .height = 200.0f});
  NVE_CHECK(host.attach_interface(&h::demo::CounterApp::get_interface));
  const std::string out = (std::filesystem::temp_directory_path() / "nve_win_counter.bmp").string();
  std::string error;
  NVE_CHECK(render_visual_tree_to_bmp(host.visual_tree(), host.text_measurer(), 640u, 400u, 2.0f, out, &error));
  std::ifstream in(out, std::ios::binary);
  const std::vector<uint8_t> file{std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>()};
  NVE_CHECK(file.size() == 54u + 640u * 400u * 4u && file[0] == 'B' && file[1] == 'M');
  // Bottom-up rows: the bottom-right pixel is the column background rgb(248, 250, 252).
  const uint8_t* px = file.data() + 54u + 4u * 639u;
  NVE_CHECK(px[0] == 252u && px[1] == 250u && px[2] == 248u);
  NVE_CHECK(!render_visual_tree_to_bmp(host.visual_tree(), host.text_measurer(), 0u, 10u, 1.0f, out, &error));
  std::filesystem::remove(out);
}

void test_window_inputs_and_wakes() {
  constexpr uint32_t kInput = 2u;
  ScriptedApp app;
  app.w.create(1u);
  app.w.create(kInput);
  (void)app.w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "hi");
  events(app.w, kInput, {NVE_EVENT_TEXT_CHANGE, NVE_EVENT_SUBMIT, NVE_EVENT_FOCUS, NVE_EVENT_BLUR});
  sized(app.w, kInput, 160.0f, 24.0f);
  const uint32_t kids[] = {kInput};
  app.w.set_children(1u, kids);
  app.on_event = [](ScriptedApp& a, const h::testing::RecordedEvent& e) {
    if (e.ev.kind != NVE_EVENT_TEXT_CHANGE) return;
    std::string upper = e.text;
    for (char& c : upper) c = static_cast<char>(c >= 'a' && c <= 'z' ? c - 32 : c);
    (void)a.w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, upper);
  };

  NveWinHostWindow window(WinHostWindowOptions{.title = "test", .width = 320, .height = 200, .quit_on_destroy = false});
  NVE_CHECK(window.create(::GetModuleHandleW(nullptr)));
  NVE_CHECK(window.host().attach_interface(&ScriptedApp::get_interface));
  pump_messages();
  HWND edit = window.text_inputs().control_for_node(kInput);
  NVE_CHECK(edit != nullptr && WinTextInputs::control_text(edit) == "hi");
  ::SetWindowTextW(edit, L"typed");  // EN_CHANGE -> TextChange -> app echo
  pump_messages();
  NVE_CHECK(app.last(NVE_EVENT_TEXT_CHANGE)->text == "typed" && app.count(NVE_EVENT_TEXT_CHANGE) == 1u);
  NVE_CHECK(WinTextInputs::control_text(edit) == "TYPED" && node(window.host(), kInput).input.value == "TYPED");
  ::SendMessageW(edit, WM_KEYDOWN, VK_RETURN, 0);
  NVE_CHECK(app.last(NVE_EVENT_SUBMIT)->text == "TYPED");
  // A runtime-side change (state the app renders on its own) arrives through a queued wake.
  (void)app.w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "from app");
  (void)window.host().session().dispatch(ScriptedApp::flush_event(window.surface_id()));
  pump_messages();
  NVE_CHECK(WinTextInputs::control_text(edit) == "from app");
  // Removing the node destroys its control.
  app.w.remove(kInput);
  app.w.set_children(1u, std::span<const uint32_t>());
  (void)window.host().session().dispatch(ScriptedApp::flush_event(window.surface_id()));
  (void)window.host().pump();
  pump_messages();
  NVE_CHECK(window.text_inputs().size() == 0u && !::IsWindow(edit));
  window.close();
  pump_messages();
}

void test_window_manager_windows() {
  NveWinWindowManager mgr;
  NveWinHostWindow* a = mgr.open_window(::GetModuleHandleW(nullptr), {.surface_id = 1u, .title = "A"}, false);
  NveWinHostWindow* b = mgr.open_window(::GetModuleHandleW(nullptr), {.surface_id = 2u, .title = "B", .parent_surface_id = 1u, .modal = true}, false);
  NVE_CHECK(a != nullptr && b != nullptr && mgr.window_for_surface(2u) == b && &a->host() == mgr.host_for_surface(1u));
  NVE_CHECK(a->host().attach_interface(&h::demo::CounterApp::get_interface));
  ::SendMessageW(b->hwnd(), WM_CLOSE, 0, 0);  // user close: the window manager drops it from WM_DESTROY
  pump_messages();
  NVE_CHECK(mgr.window_count() == 1u && mgr.window_for_surface(2u) == nullptr);
  NVE_CHECK(mgr.request_close(1u) && mgr.window_count() == 0u && mgr.quit_posted());
  MSG quit{};
  (void)::PeekMessageW(&quit, nullptr, WM_QUIT, WM_QUIT, PM_REMOVE);  // swallow the manager's WM_QUIT
}

}  // namespace

void nve_test_win_direct2d() {
  test_directwrite_measurer();
  test_draw_list_pixels();
  test_wic_images();
  test_screenshot_bmp();
  test_window_inputs_and_wakes();
  test_window_manager_windows();
}
