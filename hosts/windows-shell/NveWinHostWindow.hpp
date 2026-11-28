// Win32 + Direct2D / DirectWrite host window and the painting API (Windows only).
#pragma once

#if !defined(_WIN32)
#error "NveWinHostWindow.hpp is Windows-only"
#endif

#include "NveWinHost.hpp"
#include "NveWinTextInputs.hpp"

#include <nve/canvas/draw_list.hpp>

#include <d2d1.h>
#include <windows.h>

#include <atomic>
#include <cstdint>
#include <functional>
#include <list>
#include <memory>
#include <mutex>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

struct IWICImagingFactory;
struct NveA11yWinHandle;
struct NveA11yWinAction;

namespace nve::win {

/// Runtime wake (any thread) -> `NveWinHost::pump()` on the UI thread.
inline constexpr UINT WM_APP_WAKE = WM_APP + 1u;
/// Re-place the native `EDIT` overlays after the visual tree changed (coalesced, never inline).
inline constexpr UINT WM_APP_SYNC_INPUTS = WM_APP + 2u;
/// UIA assistive-technology action marshaled from a COM worker thread onto the UI thread.
inline constexpr UINT WM_APP_A11Y_ACTION = WM_APP + 3u;

struct WinHostWindowOptions {
  std::string title{"NVE Windows Host Shell"};
  int width = 640;  // client DIPs
  int height = 420;
  uint32_t surface_id = 1u;
  bool quit_on_destroy = true;
};

/// Device resources of ONE render target, owned next to it (window member, screenshot local) and
/// released deterministically with it: WIC-decoded bitmaps (bounded LRU, own WIC factory) and
/// per-node DirectWrite layouts (rebuilt when text / style / box change, dropped with the node).
/// UI-thread only.
class D2DPaintCache {
 public:
  D2DPaintCache() = default;
  ~D2DPaintCache() { release(); }
  D2DPaintCache(const D2DPaintCache&) = delete;
  D2DPaintCache& operator=(const D2DPaintCache&) = delete;

  /// Bitmap of `src` on `rt` (decoded on a miss; `nullptr` when it does not decode or is not a
  /// local / inline image). Borrowed until `release()` or eviction.
  ID2D1Bitmap* bitmap(ID2D1RenderTarget* rt, const WinImageSource& src);
  /// Layout of node `node`'s `text` in a `width` x `height` box. Borrowed until the next call
  /// for the node or `prune`.
  IDWriteTextLayout* layout(const DirectWriteTextMeasurer& m, uint32_t node, std::string_view text,
                            const TextPaint& style, float width, float height);
  /// Drops layouts of nodes `tree` no longer has.
  void prune(const NveWinVisualTree& tree);
  /// Drops everything (the target was lost or is being destroyed).
  void release() noexcept;

 private:
  struct Bitmap {
    ID2D1Bitmap* bitmap = nullptr;
    std::list<std::string>::iterator at;
  };
  struct Layout {
    IDWriteTextLayout* layout = nullptr;
    std::string text;
    TextPaint style;
    float width = 0.0f;
    float height = 0.0f;
  };
  ID2D1Bitmap* decode(ID2D1RenderTarget* rt, const WinImageSource& src);

  ID2D1RenderTarget* target_ = nullptr;  // the bitmaps' owner
  IWICImagingFactory* wic_ = nullptr;
  bool com_ = false;  // this cache's `CoInitializeEx` succeeded (S_OK / S_FALSE): balanced in `release`
  std::unordered_map<std::string, Bitmap> bitmaps_;
  std::list<std::string> lru_;  // most recent first
  std::unordered_map<uint32_t, Layout> layouts_;
};

/// Paints all 8 flat `nve.res.draw_list` opcodes into `bounds` (validated first; clip stack
/// balanced even for corrupt payloads or unbalanced clips). `factory` builds polylines / stroke
/// styles (nullable).
canvas::DrawListDecodeResult render_draw_list_to_d2d(ID2D1RenderTarget* rt, ID2D1SolidColorBrush* brush,
                                                     ID2D1Factory* factory, const D2D1_RECT_F& bounds,
                                                     std::span<const uint8_t> bytes) noexcept;

/// Paints `tree` (one `BeginDraw` / `EndDraw`) onto any render target: the window's
/// `ID2D1HwndRenderTarget` or an offscreen `ID2D1DCRenderTarget`. `factory` created `rt`; `cache`
/// belongs to `rt`. With `dirty` (DIPs), only that area is cleared and repainted.
HRESULT paint_visual_tree_d2d(ID2D1RenderTarget* rt, ID2D1Factory* factory, const NveWinVisualTree& tree,
                              const DirectWriteTextMeasurer& text, D2DPaintCache& cache,
                              const Rect* dirty = nullptr);

/// Headless: paints `tree` into a 32-bit DIB and writes `out_bmp_path` (no window needed).
bool render_visual_tree_to_bmp(const NveWinVisualTree& tree, const DirectWriteTextMeasurer& text, uint32_t width_px,
                               uint32_t height_px, float scale_factor, std::string_view out_bmp_path,
                               std::string* out_error = nullptr);

/// System cursor (`IDC_*`) of an `NVE_CURSOR_KIND_*`.
[[nodiscard]] LPCWSTR system_cursor(uint8_t kind) noexcept;

/// Per-monitor-DPI-aware top-level window showing one `NveWinHost` surface: Direct2D painting,
/// press / link / wheel / hover / cursor input, native `EDIT` text inputs, queued runtime wakes.
class NveWinHostWindow {
 public:
  explicit NveWinHostWindow(WinHostWindowOptions options = {});
  ~NveWinHostWindow();
  NveWinHostWindow(const NveWinHostWindow&) = delete;
  NveWinHostWindow& operator=(const NveWinHostWindow&) = delete;

  /// Creates the runtime surface and the window (hidden until `show`), sized to its client
  /// area; `error` gets the reason on failure. `host()` is valid only after a successful create.
  bool create(HINSTANCE instance, std::string* error = nullptr);
  void show(int cmd_show = SW_SHOWDEFAULT);
  /// Runs the thread's message loop until `WM_QUIT`.
  static int run_message_loop();

  /// Renders the current tree to a `.bmp` (client size x DPI scale).
  bool save_screenshot_bmp(std::string_view out_bmp_path, std::string* out_error = nullptr) const;

  void set_title(std::string_view title);
  /// Moves the window to screen `(x, y)` with a client area of `width` x `height` DIPs.
  void set_bounds(int x, int y, int width, int height);
  void focus();
  void close();
  /// `WM_CLOSE`: return `false` to veto.
  void set_on_close_request(std::function<bool(uint32_t)> callback) { on_close_request_ = std::move(callback); }
  void set_on_destroy(std::function<void(uint32_t)> callback) { on_destroy_ = std::move(callback); }

  [[nodiscard]] uint32_t surface_id() const noexcept { return options_.surface_id; }
  [[nodiscard]] HWND hwnd() const noexcept { return hwnd_; }
  [[nodiscard]] float dpi_scale() const noexcept;
  [[nodiscard]] NveWinHost& host() noexcept { return *host_; }
  [[nodiscard]] const NveWinHost& host() const noexcept { return *host_; }
  [[nodiscard]] const WinTextInputs& text_inputs() const noexcept { return inputs_; }
  /// Called by the C-ABI UIA action callback (may run on a COM worker thread).
  void enqueue_a11y_action(const NveA11yWinAction& action);

 private:
  struct QueuedA11yAction {
    uint32_t node_id = 0u;
    uint32_t kind = 0u;
    float x = 0.0f;
    float y = 0.0f;
    double numeric_value = 0.0;
    size_t anchor_char = 0u;
    size_t focus_char = 0u;
    std::string text;
  };

  static LRESULT CALLBACK static_wnd_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);
  LRESULT handle_message(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

  bool ensure_render_target();
  void discard_render_target() noexcept;
  void on_paint();
  void sync_surface_size();
  void on_wheel(WPARAM wparam, LPARAM lparam, bool horizontal);
  void on_key(WPARAM wparam, LPARAM lparam, bool pressed);
  void sync_native_focus(uint32_t focused_node);
  void on_change();
  void schedule_input_sync();
  void sync_a11y_with_guard(const host::RuntimeSceneGuard& guard);
  void sync_a11y_now();
  void execute_a11y_action(const QueuedA11yAction& action);
  void drain_a11y_actions();
  void destroy_a11y() noexcept;
  void open_link(const std::string& href);
  [[nodiscard]] float dip(int device_px) const noexcept { return static_cast<float>(device_px) / dpi_scale(); }
  [[nodiscard]] RECT outer_rect(int width_dips, int height_dips) const noexcept;

  WinHostWindowOptions options_;
  std::unique_ptr<NveWinHost> host_;
  WinTextInputs inputs_;
  D2DPaintCache cache_;
  HWND hwnd_ = nullptr;
  std::atomic<HWND> wake_hwnd_{nullptr};  // read by wake posters on any thread; null once destroyed
  DWORD ui_thread_id_ = 0u;
  NveA11yWinHandle* a11y_ = nullptr;
  std::mutex a11y_mu_;
  std::vector<QueuedA11yAction> a11y_actions_;
  ID2D1Factory* d2d_factory_ = nullptr;
  ID2D1HwndRenderTarget* render_target_ = nullptr;
  bool tracking_leave_ = false;
  bool input_sync_posted_ = false;
  std::function<bool(uint32_t)> on_close_request_;
  std::function<void(uint32_t)> on_destroy_;
};

}  // namespace nve::win
