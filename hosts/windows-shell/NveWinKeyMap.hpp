// Win32 virtual-key (`VK_*`) + modifier mapping to W3C `KeyboardEvent.key` / `code` + NVE flags.
#pragma once

#if !defined(_WIN32)
#error "NveWinKeyMap.hpp is Windows-only"
#endif

#include <nve/host/host_session.hpp>

#include <windows.h>

#include <cstdint>
#include <string>

namespace nve::win {

struct MappedWinKey {
  std::string key;
  std::string code;
  uint32_t flags = 0u;
  uint32_t key_code = 0u;
};

[[nodiscard]] inline uint32_t win32_modifier_flags(LPARAM lparam, bool pressed) noexcept {
  uint32_t flags = 0u;
  if ((::GetKeyState(VK_SHIFT) & 0x8000) != 0) flags |= NVE_EVENT_FLAG_MOD_SHIFT;
  if ((::GetKeyState(VK_CONTROL) & 0x8000) != 0) flags |= NVE_EVENT_FLAG_MOD_CTRL;
  if ((::GetKeyState(VK_MENU) & 0x8000) != 0) flags |= NVE_EVENT_FLAG_MOD_ALT;
  if (((::GetKeyState(VK_LWIN) | ::GetKeyState(VK_RWIN)) & 0x8000) != 0) flags |= NVE_EVENT_FLAG_MOD_META;
  if (pressed && ((static_cast<uintptr_t>(lparam) >> 30u) & 1u) != 0u) flags |= NVE_EVENT_FLAG_MOD_REPEAT;
  return flags;
}

[[nodiscard]] inline MappedWinKey map_win32_key(WPARAM wparam, LPARAM lparam, bool pressed) {
  MappedWinKey out;
  const auto vk = static_cast<UINT>(wparam);
  out.flags = win32_modifier_flags(lparam, pressed);
  out.key_code = static_cast<uint32_t>(vk);

  const bool extended = ((static_cast<uintptr_t>(lparam) >> 24u) & 1u) != 0u;
  switch (vk) {
    case VK_TAB: out.key = "Tab"; out.code = "Tab"; return out;
    case VK_RETURN: out.key = "Enter"; out.code = extended ? "NumpadEnter" : "Enter"; return out;
    case VK_ESCAPE: out.key = "Escape"; out.code = "Escape"; return out;
    case VK_BACK: out.key = "Backspace"; out.code = "Backspace"; return out;
    case VK_DELETE: out.key = "Delete"; out.code = "Delete"; return out;
    case VK_INSERT: out.key = "Insert"; out.code = "Insert"; return out;
    case VK_SPACE: out.key = " "; out.code = "Space"; return out;
    case VK_LEFT: out.key = "ArrowLeft"; out.code = "ArrowLeft"; return out;
    case VK_RIGHT: out.key = "ArrowRight"; out.code = "ArrowRight"; return out;
    case VK_UP: out.key = "ArrowUp"; out.code = "ArrowUp"; return out;
    case VK_DOWN: out.key = "ArrowDown"; out.code = "ArrowDown"; return out;
    case VK_HOME: out.key = "Home"; out.code = "Home"; return out;
    case VK_END: out.key = "End"; out.code = "End"; return out;
    case VK_PRIOR: out.key = "PageUp"; out.code = "PageUp"; return out;
    case VK_NEXT: out.key = "PageDown"; out.code = "PageDown"; return out;
    case VK_SHIFT:
    case VK_LSHIFT:
    case VK_RSHIFT:
      out.key = "Shift";
      out.code = vk == VK_RSHIFT ? "ShiftRight" : "ShiftLeft";
      return out;
    case VK_CONTROL:
    case VK_LCONTROL:
    case VK_RCONTROL:
      out.key = "Control";
      out.code = (vk == VK_RCONTROL || extended) ? "ControlRight" : "ControlLeft";
      return out;
    case VK_MENU:
    case VK_LMENU:
    case VK_RMENU:
      out.key = "Alt";
      out.code = (vk == VK_RMENU || extended) ? "AltRight" : "AltLeft";
      return out;
    case VK_LWIN: out.key = "Meta"; out.code = "MetaLeft"; return out;
    case VK_RWIN: out.key = "Meta"; out.code = "MetaRight"; return out;
    default: break;
  }
  if (vk >= VK_F1 && vk <= VK_F12) {
    const std::string fn = "F" + std::to_string(vk - VK_F1 + 1u);
    out.key = fn;
    out.code = fn;
    return out;
  }
  if (vk >= 'A' && vk <= 'Z') {
    const bool shift = (out.flags & NVE_EVENT_FLAG_MOD_SHIFT) != 0u;
    const bool caps = (::GetKeyState(VK_CAPITAL) & 1) != 0;
    const char ch = static_cast<char>((shift != caps) ? vk : (vk - 'A' + 'a'));
    out.key.assign(1u, ch);
    out.code = std::string("Key") + static_cast<char>(vk);
    return out;
  }
  if (vk >= '0' && vk <= '9') {
    out.code = std::string("Digit") + static_cast<char>(vk);
  }
  BYTE kb[256]{};
  if (::GetKeyboardState(kb)) {
    wchar_t buf[8]{};
    const UINT sc = static_cast<UINT>((static_cast<uintptr_t>(lparam) >> 16u) & 0xFFu);
    const int n = ::ToUnicode(vk, sc, kb, buf, 8, 0x4u);
    if (n > 0) {
      const int utf8_len = ::WideCharToMultiByte(CP_UTF8, 0, buf, n, nullptr, 0, nullptr, nullptr);
      if (utf8_len > 0) {
        out.key.resize(static_cast<size_t>(utf8_len));
        (void)::WideCharToMultiByte(CP_UTF8, 0, buf, n, out.key.data(), utf8_len, nullptr, nullptr);
      }
    }
  }
  if (out.key.empty() && vk >= '0' && vk <= '9') out.key.assign(1u, static_cast<char>(vk));
  if (out.key.empty()) out.key = "Unidentified";
  return out;
}

}  // namespace nve::win
