#include "NveWinHostWindow.hpp"

#if defined(_WIN32)

#include <algorithm>
#include <cmath>
#include <cstring>
#include <fstream>
#include <vector>
#include <windowsx.h>

namespace nve::win {

namespace {

constexpr wchar_t kWindowClassName[] = L"NveWinHostWindowClass";

D2D1_COLOR_F to_d2d_color(uint32_t rgba) noexcept {
  const nve::RgbaColor c = nve::unpack_rgba(rgba);
  return D2D1::ColorF(c.r, c.g, c.b, c.a);
}

void paint_node_recursive(
    ID2D1RenderTarget* rt,
    ID2D1SolidColorBrush* brush,
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer,
    uint32_t node_id,
    float origin_x,
    float origin_y,
    uint32_t depth) {
  if (depth > 512) {
    return;
  }
  const NveWinNodeVisual* node = tree.find(node_id);
  if (node == nullptr) {
    return;
  }

  const float clamped_opacity = std::clamp(node->opacity, 0.0f, 1.0f);
  if (clamped_opacity <= 0.0f) {
    return;
  }

  ID2D1Layer* opacity_layer = nullptr;
  const bool push_layer = (clamped_opacity < 0.999f);
  if (push_layer) {
    if (SUCCEEDED(rt->CreateLayer(nullptr, &opacity_layer)) && opacity_layer != nullptr) {
      D2D1_LAYER_PARAMETERS params = D2D1::LayerParameters(
          D2D1::InfiniteRect(),
          nullptr,
          D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
          D2D1::IdentityMatrix(),
          clamped_opacity,
          nullptr,
          D2D1_LAYER_OPTIONS_NONE);
      rt->PushLayer(params, opacity_layer);
    }
  }

  const float abs_x = origin_x + node->layout.x;
  const float abs_y = origin_y + node->layout.y;
  const float w = std::max(0.0f, node->layout.width);
  const float h = std::max(0.0f, node->layout.height);

  if (w > 0.0f && h > 0.0f) {
    const D2D1_RECT_F rect = D2D1::RectF(abs_x, abs_y, abs_x + w, abs_y + h);
    const float radius = std::clamp(node->border_radius, 0.0f, 0.5f * std::min(w, h));

    const nve::RgbaColor bg = nve::unpack_rgba(node->background_rgba);
    if (bg.a8 > 0) {
      brush->SetColor(D2D1::ColorF(bg.r, bg.g, bg.b, bg.a));
      if (radius > 0.0f) {
        rt->FillRoundedRectangle(D2D1::RoundedRect(rect, radius, radius), brush);
      } else {
        rt->FillRectangle(rect, brush);
      }
    }

    const nve::RgbaColor border = nve::unpack_rgba(node->border_color_rgba);
    if (node->border_width > 0.0f && border.a8 > 0) {
      const float half = 0.5f * node->border_width;
      const D2D1_RECT_F inset =
          D2D1::RectF(rect.left + half, rect.top + half, rect.right - half, rect.bottom - half);
      if (inset.right > inset.left && inset.bottom > inset.top) {
        brush->SetColor(D2D1::ColorF(border.r, border.g, border.b, border.a));
        if (radius > 0.0f) {
          const float inner_r = std::max(0.0f, radius - half);
          rt->DrawRoundedRectangle(
              D2D1::RoundedRect(inset, inner_r, inner_r), brush, node->border_width);
        } else {
          rt->DrawRectangle(inset, brush, node->border_width);
        }
      }
    }

    if (node->is_text_leaf && !node->text.empty()) {
      const nve::RgbaColor fg = nve::unpack_rgba(node->text_color_rgba);
      if (fg.a8 > 0) {
        IDWriteTextLayout* text_layout = nullptr;
        if (measurer.create_text_layout(
                node->text, node->font_size, std::max(1.0f, w), std::max(1.0f, h), &text_layout)) {
          brush->SetColor(D2D1::ColorF(fg.r, fg.g, fg.b, fg.a));
          rt->DrawTextLayout(
              D2D1::Point2F(abs_x, abs_y), text_layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
          text_layout->Release();
        }
      }
    }
  }

  if (!node->children.empty()) {
    std::vector<uint32_t> ordered = node->children;
    std::stable_sort(ordered.begin(), ordered.end(), [&tree](uint32_t a, uint32_t b) {
      const NveWinNodeVisual* va = tree.find(a);
      const NveWinNodeVisual* vb = tree.find(b);
      const int32_t za = (va != nullptr) ? va->z_index : 0;
      const int32_t zb = (vb != nullptr) ? vb->z_index : 0;
      return za < zb;
    });
    for (uint32_t child_id : ordered) {
      paint_node_recursive(rt, brush, tree, measurer, child_id, abs_x, abs_y, depth + 1);
    }
  }

  if (opacity_layer != nullptr) {
    rt->PopLayer();
    opacity_layer->Release();
  }
}

bool write_bgra_bmp_file(
    std::string_view out_path,
    uint32_t width_px,
    uint32_t height_px,
    const uint8_t* top_down_bgra,
    std::string* out_error) {
  if (out_path.empty() || width_px == 0 || height_px == 0 || top_down_bgra == nullptr) {
    if (out_error != nullptr) {
      *out_error = "Invalid BMP export arguments";
    }
    return false;
  }

  const uint32_t row_bytes = width_px * 4u;
  const uint32_t pixel_bytes = row_bytes * height_px;

  BITMAPFILEHEADER file_hdr{};
  file_hdr.bfType = 0x4D42; // "BM"
  file_hdr.bfOffBits = sizeof(BITMAPFILEHEADER) + sizeof(BITMAPINFOHEADER);
  file_hdr.bfSize = file_hdr.bfOffBits + pixel_bytes;

  BITMAPINFOHEADER info_hdr{};
  info_hdr.biSize = sizeof(BITMAPINFOHEADER);
  info_hdr.biWidth = static_cast<LONG>(width_px);
  info_hdr.biHeight = static_cast<LONG>(height_px); // positive = standard bottom-up BMP
  info_hdr.biPlanes = 1;
  info_hdr.biBitCount = 32;
  info_hdr.biCompression = BI_RGB;
  info_hdr.biSizeImage = pixel_bytes;

  std::ofstream out(DirectWriteTextMeasurer::utf8_to_wide(out_path), std::ios::binary | std::ios::trunc);
  if (!out.is_open()) {
    if (out_error != nullptr) {
      *out_error = "Failed to open BMP output file";
    }
    return false;
  }

  out.write(reinterpret_cast<const char*>(&file_hdr), sizeof(file_hdr));
  out.write(reinterpret_cast<const char*>(&info_hdr), sizeof(info_hdr));
  for (uint32_t y = 0; y < height_px; ++y) {
    const uint32_t src_row = (height_px - 1u) - y;
    out.write(
        reinterpret_cast<const char*>(top_down_bgra + static_cast<size_t>(src_row) * row_bytes),
        row_bytes);
  }
  if (!out.good()) {
    if (out_error != nullptr) {
      *out_error = "Failed while writing BMP output file";
    }
    return false;
  }
  return true;
}

} // namespace

HRESULT paint_visual_tree_d2d(
    ID2D1RenderTarget* rt,
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer) {
  if (rt == nullptr) {
    return E_INVALIDARG;
  }

  ID2D1SolidColorBrush* brush = nullptr;
  HRESULT hr = rt->CreateSolidColorBrush(D2D1::ColorF(D2D1::ColorF::White), &brush);
  if (FAILED(hr) || brush == nullptr) {
    return hr;
  }

  rt->BeginDraw();
  rt->SetTransform(D2D1::Matrix3x2F::Identity());

  const nve::RgbaColor root_bg = nve::unpack_rgba(tree.root_background_rgba());
  if (root_bg.a8 > 0) {
    rt->Clear(to_d2d_color(tree.root_background_rgba()));
  } else {
    rt->Clear(D2D1::ColorF(0.06f, 0.09f, 0.16f, 1.0f));
  }

  std::vector<uint32_t> ordered_roots = tree.root_children();
  std::stable_sort(ordered_roots.begin(), ordered_roots.end(), [&tree](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = tree.find(a);
    const NveWinNodeVisual* vb = tree.find(b);
    const int32_t za = (va != nullptr) ? va->z_index : 0;
    const int32_t zb = (vb != nullptr) ? vb->z_index : 0;
    return za < zb;
  });

  for (uint32_t child_id : ordered_roots) {
    paint_node_recursive(rt, brush, tree, measurer, child_id, 0.0f, 0.0f, 0);
  }

  hr = rt->EndDraw();
  brush->Release();
  return hr;
}

bool render_visual_tree_to_bmp(
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer,
    uint32_t width_px,
    uint32_t height_px,
    float scale_factor,
    std::string_view out_bmp_path,
    std::string* out_error) {
  if (width_px == 0 || height_px == 0) {
    if (out_error != nullptr) {
      *out_error = "Screenshot dimensions must be non-zero";
    }
    return false;
  }

  ID2D1Factory* factory = nullptr;
  HRESULT hr = ::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &factory);
  if (FAILED(hr) || factory == nullptr) {
    if (out_error != nullptr) {
      *out_error = "D2D1CreateFactory failed";
    }
    return false;
  }

  const float effective_scale = (scale_factor > 0.0f && std::isfinite(scale_factor)) ? scale_factor : 1.0f;
  const float dpi = effective_scale * 96.0f;

  const D2D1_RENDER_TARGET_PROPERTIES props = D2D1::RenderTargetProperties(
      D2D1_RENDER_TARGET_TYPE_DEFAULT,
      D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED),
      dpi,
      dpi,
      D2D1_RENDER_TARGET_USAGE_NONE,
      D2D1_FEATURE_LEVEL_DEFAULT);

  ID2D1DCRenderTarget* dc_rt = nullptr;
  hr = factory->CreateDCRenderTarget(&props, &dc_rt);
  if (FAILED(hr) || dc_rt == nullptr) {
    factory->Release();
    if (out_error != nullptr) {
      *out_error = "CreateDCRenderTarget failed";
    }
    return false;
  }

  HDC mem_dc = ::CreateCompatibleDC(nullptr);
  if (mem_dc == nullptr) {
    dc_rt->Release();
    factory->Release();
    if (out_error != nullptr) {
      *out_error = "CreateCompatibleDC failed";
    }
    return false;
  }

  BITMAPINFO bmi{};
  bmi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
  bmi.bmiHeader.biWidth = static_cast<LONG>(width_px);
  bmi.bmiHeader.biHeight = -static_cast<LONG>(height_px); // top-down DIB
  bmi.bmiHeader.biPlanes = 1;
  bmi.bmiHeader.biBitCount = 32;
  bmi.bmiHeader.biCompression = BI_RGB;

  void* dib_bits = nullptr;
  HBITMAP dib = ::CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &dib_bits, nullptr, 0);
  if (dib == nullptr || dib_bits == nullptr) {
    ::DeleteDC(mem_dc);
    dc_rt->Release();
    factory->Release();
    if (out_error != nullptr) {
      *out_error = "CreateDIBSection failed";
    }
    return false;
  }

  HGDIOBJ prev_obj = ::SelectObject(mem_dc, dib);
  const RECT bind_rect{0, 0, static_cast<LONG>(width_px), static_cast<LONG>(height_px)};
  hr = dc_rt->BindDC(mem_dc, &bind_rect);

  bool ok = false;
  if (SUCCEEDED(hr)) {
    hr = paint_visual_tree_d2d(dc_rt, tree, measurer);
    if (SUCCEEDED(hr)) {
      ::GdiFlush();
      ok = write_bgra_bmp_file(
          out_bmp_path, width_px, height_px, static_cast<const uint8_t*>(dib_bits), out_error);
    } else if (out_error != nullptr) {
      *out_error = "paint_visual_tree_d2d failed";
    }
  } else if (out_error != nullptr) {
    *out_error = "ID2D1DCRenderTarget::BindDC failed";
  }

  ::SelectObject(mem_dc, prev_obj);
  ::DeleteObject(dib);
  ::DeleteDC(mem_dc);
  dc_rt->Release();
  factory->Release();
  return ok;
}

// -----------------------------------------------------------------------------
// NveWinHostWindow
// -----------------------------------------------------------------------------

NveWinHostWindow::NveWinHostWindow() {
  host_.set_on_visual_tree_changed([this]() {
    if (hwnd_ != nullptr) {
      ::InvalidateRect(hwnd_, nullptr, FALSE);
    }
  });
}

NveWinHostWindow::~NveWinHostWindow() {
  host_.set_on_visual_tree_changed({});
  discard_device_resources();
  if (d2d_factory_ != nullptr) {
    d2d_factory_->Release();
    d2d_factory_ = nullptr;
  }
  if (hwnd_ != nullptr && ::IsWindow(hwnd_)) {
    ::DestroyWindow(hwnd_);
    hwnd_ = nullptr;
  }
}

bool NveWinHostWindow::create(HINSTANCE instance, const WinHostWindowOptions& options) {
  options_ = options;
  if (!ensure_d2d_factory()) {
    return false;
  }

  WNDCLASSEXW wc{};
  wc.cbSize = sizeof(wc);
  wc.style = CS_HREDRAW | CS_VREDRAW;
  wc.lpfnWndProc = &NveWinHostWindow::static_wnd_proc;
  wc.hInstance = instance;
  wc.hCursor = ::LoadCursorW(nullptr, reinterpret_cast<LPCWSTR>(IDC_ARROW));
  wc.lpszClassName = kWindowClassName;
  ::RegisterClassExW(&wc);

  const std::wstring title_w = DirectWriteTextMeasurer::utf8_to_wide(
      options_.title.empty() ? "NVE Windows Host Shell" : options_.title);

  RECT desired{0, 0, options_.width, options_.height};
  const DWORD style = WS_OVERLAPPEDWINDOW;
  ::AdjustWindowRect(&desired, style, FALSE);

  hwnd_ = ::CreateWindowExW(
      0,
      kWindowClassName,
      title_w.c_str(),
      style,
      CW_USEDEFAULT,
      CW_USEDEFAULT,
      desired.right - desired.left,
      desired.bottom - desired.top,
      nullptr,
      nullptr,
      instance,
      this);

  return hwnd_ != nullptr;
}

void NveWinHostWindow::show(int cmd_show) {
  if (hwnd_ != nullptr) {
    ::ShowWindow(hwnd_, cmd_show);
    ::UpdateWindow(hwnd_);
  }
}

int NveWinHostWindow::run_message_loop() {
  MSG msg{};
  while (::GetMessageW(&msg, nullptr, 0, 0) > 0) {
    ::TranslateMessage(&msg);
    ::DispatchMessageW(&msg);
  }
  return static_cast<int>(msg.wParam);
}

float NveWinHostWindow::dpi_scale() const noexcept {
  if (hwnd_ == nullptr) {
    return 1.0f;
  }
  const UINT dpi = ::GetDpiForWindow(hwnd_);
  return (dpi > 0) ? (static_cast<float>(dpi) / 96.0f) : 1.0f;
}

NveSurfaceConfig NveWinHostWindow::current_surface_config() const noexcept {
  const float scale = dpi_scale();
  float dip_w = static_cast<float>(options_.width > 0 ? options_.width : 640);
  float dip_h = static_cast<float>(options_.height > 0 ? options_.height : 420);
  if (hwnd_ != nullptr) {
    RECT rc{};
    if (::GetClientRect(hwnd_, &rc)) {
      const float px_w = static_cast<float>(std::max<LONG>(0, rc.right - rc.left));
      const float px_h = static_cast<float>(std::max<LONG>(0, rc.bottom - rc.top));
      if (px_w > 0.0f && px_h > 0.0f) {
        dip_w = px_w / scale;
        dip_h = px_h / scale;
      }
    }
  }
  return NveSurfaceConfig{
      .surface_id = options_.surface_id,
      .width = dip_w,
      .height = dip_h,
      .scale_factor = scale,
  };
}

bool NveWinHostWindow::attach_interface(NveGetInterfaceFn get_interface) {
  return host_.attach_interface(get_interface, current_surface_config());
}

bool NveWinHostWindow::load_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  return host_.load_dylib(path, current_surface_config(), out_err_code, out_err_msg);
}

bool NveWinHostWindow::reload_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  return host_.reload_dylib(path, out_err_code, out_err_msg);
}

bool NveWinHostWindow::save_screenshot_bmp(
    std::string_view out_bmp_path,
    std::string* out_error) const {
  const NveSurfaceConfig cfg = host_.is_valid() ? host_.surface_config() : current_surface_config();
  const float scale = (cfg.scale_factor > 0.0f) ? cfg.scale_factor : 1.0f;
  const uint32_t px_w = static_cast<uint32_t>(std::max(1.0f, std::round(cfg.width * scale)));
  const uint32_t px_h = static_cast<uint32_t>(std::max(1.0f, std::round(cfg.height * scale)));
  return render_visual_tree_to_bmp(
      host_.visual_tree(), *host_.text_measurer(), px_w, px_h, scale, out_bmp_path, out_error);
}

bool NveWinHostWindow::ensure_d2d_factory() {
  if (d2d_factory_ != nullptr) {
    return true;
  }
  const HRESULT hr = ::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &d2d_factory_);
  return SUCCEEDED(hr) && d2d_factory_ != nullptr;
}

bool NveWinHostWindow::ensure_render_target() {
  if (render_target_ != nullptr) {
    return true;
  }
  if (hwnd_ == nullptr || !ensure_d2d_factory()) {
    return false;
  }

  RECT rc{};
  ::GetClientRect(hwnd_, &rc);
  const D2D1_SIZE_U size = D2D1::SizeU(
      static_cast<UINT32>(std::max<LONG>(1, rc.right - rc.left)),
      static_cast<UINT32>(std::max<LONG>(1, rc.bottom - rc.top)));

  const HRESULT hr = d2d_factory_->CreateHwndRenderTarget(
      D2D1::RenderTargetProperties(),
      D2D1::HwndRenderTargetProperties(hwnd_, size),
      &render_target_);
  if (FAILED(hr) || render_target_ == nullptr) {
    return false;
  }

  const float dpi = dpi_scale() * 96.0f;
  render_target_->SetDpi(dpi, dpi);
  return true;
}

void NveWinHostWindow::discard_device_resources() noexcept {
  if (render_target_ != nullptr) {
    render_target_->Release();
    render_target_ = nullptr;
  }
}

void NveWinHostWindow::on_paint() {
  PAINTSTRUCT ps{};
  ::BeginPaint(hwnd_, &ps);
  if (ensure_render_target()) {
    const HRESULT hr =
        paint_visual_tree_d2d(render_target_, host_.visual_tree(), *host_.text_measurer());
    if (hr == D2DERR_RECREATE_TARGET) {
      discard_device_resources();
      ::InvalidateRect(hwnd_, nullptr, FALSE);
    }
  }
  ::EndPaint(hwnd_, &ps);
}

void NveWinHostWindow::on_resize(UINT width_px, UINT height_px) {
  if (width_px == 0 || height_px == 0) {
    return;
  }
  if (render_target_ != nullptr) {
    (void)render_target_->Resize(D2D1::SizeU(width_px, height_px));
  }
  const float scale = dpi_scale();
  host_.set_surface_size(
      static_cast<float>(width_px) / scale, static_cast<float>(height_px) / scale, scale);
  ::InvalidateRect(hwnd_, nullptr, FALSE);
}

void NveWinHostWindow::on_left_button_up(int x_px, int y_px) {
  const float scale = dpi_scale();
  const float dip_x = static_cast<float>(x_px) / scale;
  const float dip_y = static_cast<float>(y_px) / scale;
  if (host_.dispatch_click_at(dip_x, dip_y) > 0) {
    ::InvalidateRect(hwnd_, nullptr, FALSE);
  }
}

LRESULT CALLBACK NveWinHostWindow::static_wnd_proc(
    HWND hwnd,
    UINT msg,
    WPARAM wparam,
    LPARAM lparam) {
  NveWinHostWindow* self = nullptr;
  if (msg == WM_NCCREATE) {
    const auto* cs = reinterpret_cast<const CREATESTRUCTW*>(lparam);
    self = static_cast<NveWinHostWindow*>(cs->lpCreateParams);
    ::SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    if (self != nullptr) {
      self->hwnd_ = hwnd;
    }
  } else {
    self = reinterpret_cast<NveWinHostWindow*>(::GetWindowLongPtrW(hwnd, GWLP_USERDATA));
  }

  if (self != nullptr) {
    return self->handle_message(msg, wparam, lparam);
  }
  return ::DefWindowProcW(hwnd, msg, wparam, lparam);
}

LRESULT NveWinHostWindow::handle_message(UINT msg, WPARAM wparam, LPARAM lparam) {
  switch (msg) {
    case WM_ERASEBKGND:
      return 1;
    case WM_PAINT:
      on_paint();
      return 0;
    case WM_SIZE:
      on_resize(LOWORD(lparam), HIWORD(lparam));
      return 0;
    case WM_DPICHANGED: {
      const float new_dpi = static_cast<float>(HIWORD(wparam));
      const float new_scale = (new_dpi > 0.0f) ? (new_dpi / 96.0f) : 1.0f;
      if (render_target_ != nullptr) {
        render_target_->SetDpi(new_dpi, new_dpi);
      }
      if (const auto* suggested = reinterpret_cast<const RECT*>(lparam)) {
        ::SetWindowPos(
            hwnd_,
            nullptr,
            suggested->left,
            suggested->top,
            suggested->right - suggested->left,
            suggested->bottom - suggested->top,
            SWP_NOZORDER | SWP_NOACTIVATE);
      }
      RECT client{};
      if (::GetClientRect(hwnd_, &client)) {
        const float dip_w = static_cast<float>(std::max<LONG>(1, client.right - client.left)) / new_scale;
        const float dip_h = static_cast<float>(std::max<LONG>(1, client.bottom - client.top)) / new_scale;
        host_.set_surface_size(dip_w, dip_h, new_scale);
      }
      ::InvalidateRect(hwnd_, nullptr, FALSE);
      return 0;
    }
    case WM_LBUTTONUP:
      on_left_button_up(GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam));
      return 0;
    case WM_DESTROY:
      discard_device_resources();
      hwnd_ = nullptr;
      if (options_.quit_on_destroy) {
        ::PostQuitMessage(0);
      }
      return 0;
    default:
      return ::DefWindowProcW(hwnd_, msg, wparam, lparam);
  }
}

} // namespace nve::win

#endif // defined(_WIN32)
