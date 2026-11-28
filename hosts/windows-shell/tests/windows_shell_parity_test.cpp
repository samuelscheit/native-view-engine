// Portable Slice 4 parity tests for `NveWinHost`: keyboard dispatch (`KeyDown`/`KeyUp`), Tab /
// Shift+Tab focus traversal with positive `tab_index` + modal scoping, Enter / Space activation
// respecting `disabled` / `TextInput`, IME composition events, `set_on_scene_sync`, and ABI v6
// Frame-0 file + bytes cold boot.
#include "NveWinTextOffsets.hpp"
#include "windows_test_helpers.hpp"

#include <nve/host/demo_counter_app.hpp>

#include <cmath>
#include <filesystem>
#include <fstream>
#include <limits>
#include <vector>

namespace {

using namespace nve::win;
using namespace nve::win::test;
using nve::host::demo::CounterApp;
namespace h = nve::host;

void test_keyboard_focus_tab_modal_and_activation() {
  constexpr uint32_t kBtnA = 2u;
  constexpr uint32_t kBtnB = 3u;
  constexpr uint32_t kDisabled = 4u;
  constexpr uint32_t kHidden = 5u;
  constexpr uint32_t kSkipTab = 6u;
  constexpr uint32_t kLink = 7u;
  constexpr uint32_t kInput = 8u;
  constexpr uint32_t kModal = 9u;
  constexpr uint32_t kModalBtn = 10u;

  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);

  const auto make_btn = [&](uint32_t id, int16_t tab_index, bool disabled) {
    w.create(id);
    sized(w, id, 100.0f, 32.0f);
    w.set_events(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD,
                 NVE_INTERACTION_EVENT_SET_PRESS | NVE_INTERACTION_EVENT_SET_FOCUS | NVE_INTERACTION_EVENT_SET_BLUR |
                     NVE_INTERACTION_EVENT_SET_KEY_DOWN | NVE_INTERACTION_EVENT_SET_KEY_UP);
    w.set_bool(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
    w.set(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_TAB_INDEX,
          static_cast<uint32_t>(static_cast<uint16_t>(tab_index)));
    if (disabled) w.set_bool(id, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_DISABLED, true);
  };

  make_btn(kBtnA, 0, false);
  make_btn(kBtnB, 1, false);  // positive tab_index: visited before kBtnA
  make_btn(kDisabled, 0, true);
  make_btn(kHidden, 0, false);
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_HIDDEN);
  make_btn(kSkipTab, -1, false);

  w.create(kLink);
  sized(w, kLink, 100.0f, 24.0f);
  (void)w.set_str(kLink, NVE_TABLE_LINK_DESC, NVE_TABLE_LINK_HREF, "https://nve.dev/docs");

  w.create(kInput);
  sized(w, kInput, 140.0f, 30.0f);
  w.set_events(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_EVENTS_WORD,
               NVE_TEXT_INPUT_EVENT_SET_CHANGE | NVE_TEXT_INPUT_EVENT_SET_COMPOSITION_START |
                   NVE_TEXT_INPUT_EVENT_SET_COMPOSITION_UPDATE | NVE_TEXT_INPUT_EVENT_SET_COMPOSITION_END);
  (void)w.set_str(kInput, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "hi");

  const uint32_t root_kids[] = {kBtnA, kBtnB, kDisabled, kHidden, kSkipTab, kLink, kInput};
  w.set_children(1u, root_kids);

  const auto host_owner = make_host();
  NveWinHost& host = *host_owner;
  size_t scene_syncs = 0u;
  host.set_on_scene_sync([&scene_syncs](const h::RuntimeSceneGuard& g) {
    if (g.raw().rows != nullptr) ++scene_syncs;
  });
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(scene_syncs >= 1u);

  // Tab order: kBtnB (tab_index=1) -> kBtnA (0) -> kLink (0) -> kInput (0) -> wrap to kBtnB.
  // kDisabled, kHidden, and kSkipTab (-1) must all be skipped.
  NVE_CHECK(host.handle_key(true, "Tab", "Tab", 0u, 9u).focused_node == kBtnB);
  NVE_CHECK(host.focused_node() == kBtnB);
  NVE_CHECK(host.handle_key(true, "Tab", "Tab", 0u, 9u).focused_node == kBtnA);
  NVE_CHECK(host.handle_key(true, "Tab", "Tab", 0u, 9u).focused_node == kLink);
  NVE_CHECK(host.handle_key(true, "Tab", "Tab", 0u, 9u).focused_node == kInput);
  NVE_CHECK(host.handle_key(true, "Tab", "Tab", 0u, 9u).focused_node == kBtnB);
  // Shift+Tab wraps backwards.
  NVE_CHECK(host.handle_key(true, "Tab", "Tab", NVE_EVENT_FLAG_MOD_SHIFT, 9u).focused_node == kInput);
  NVE_CHECK(host.handle_key(true, "Tab", "Tab", NVE_EVENT_FLAG_MOD_SHIFT, 9u).focused_node == kLink);

  // Enter on focused link opens href; Space does NOT open link; on button dispatches Press.
  const KeyResult link_space = host.handle_key(true, " ", "Space", 0u, 32u);
  NVE_CHECK(link_space.href.empty());
  const KeyResult link_act = host.handle_key(true, "Enter", "Enter", 0u, 13u);
  NVE_CHECK(link_act.handled && link_act.href == "https://nve.dev/docs");

  // `dispatch_focus(kInput, true)` while `kBtnA` is focused must emit Blur to `kBtnA` first.
  NVE_CHECK(host.set_focused_node(kBtnA) || host.focused_node() == kBtnA);
  app.events.clear();
  (void)host.dispatch_focus(kInput, true);
  NVE_CHECK(host.focused_node() == kInput);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_BLUR, kBtnA) == 1u);

  NVE_CHECK(host.set_focused_node(kBtnA) || host.focused_node() == kBtnA);
  app.events.clear();
  const KeyResult btn_act = host.handle_key(true, " ", "Space", NVE_EVENT_FLAG_MOD_SHIFT, 32u);
  NVE_CHECK(btn_act.handled && btn_act.dispatched);
  const auto* kd = app.last(NVE_INTERACTION_EVENT_KEY_DOWN);
  const auto* pr = app.last(NVE_INTERACTION_EVENT_PRESS);
  NVE_CHECK(kd != nullptr && kd->ev.node == kBtnA && kd->key() == " " && kd->code() == "Space" &&
            (kd->ev.flags & NVE_EVENT_FLAG_MOD_SHIFT) != 0u);
  NVE_CHECK(pr != nullptr && pr->ev.node == kBtnA);

  // KeyUp delivers to focused listener; disabled node refuses focus and key events.
  (void)host.handle_key(false, "a", "KeyA", NVE_EVENT_FLAG_MOD_CTRL, 'A');
  const auto* ku = app.last(NVE_INTERACTION_EVENT_KEY_UP);
  NVE_CHECK(ku != nullptr && ku->ev.node == kBtnA && ku->key() == "a" && ku->code() == "KeyA" &&
            (ku->ev.flags & NVE_EVENT_FLAG_MOD_CTRL) != 0u);
  NVE_CHECK(!host.set_focused_node(kDisabled));

  // TextInput IME composition sequence.
  app.events.clear();
  (void)host.dispatch_composition(kInput, NVE_TEXT_INPUT_EVENT_COMPOSITION_START, "");
  (void)host.dispatch_composition(kInput, NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE, "にほ");
  (void)host.dispatch_composition(kInput, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, "日本");
  const auto* cs = app.last(NVE_TEXT_INPUT_EVENT_COMPOSITION_START);
  const auto* cu = app.last(NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE);
  const auto* ce = app.last(NVE_TEXT_INPUT_EVENT_COMPOSITION_END);
  NVE_CHECK(cs != nullptr && cs->ev.node == kInput);
  NVE_CHECK(cu != nullptr && cu->ev.node == kInput && cu->text == "にほ");
  NVE_CHECK(ce != nullptr && ce->ev.node == kInput && ce->text == "日本");

  // Modal dialog traps Tab traversal inside the modal subtree (including an empty modal).
  w.create(kModal);
  sized(w, kModal, 240.0f, 160.0f);
  w.set_bool(kModal, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_MODAL, true);
  const uint32_t with_empty_modal[] = {kBtnA, kBtnB, kLink, kInput, kModal};
  w.set_children(1u, with_empty_modal);
  flush(host);
  const KeyResult empty_tab = host.handle_key(true, "Tab", "Tab", 0u, 9u);
  NVE_CHECK(empty_tab.handled && host.focused_node() == kBtnA);

  make_btn(kModalBtn, 0, false);
  const uint32_t modal_kids[] = {kModalBtn};
  w.set_children(kModal, modal_kids);
  flush(host);
  NVE_CHECK(host.visual_tree().find(kModal) != nullptr && host.visual_tree().find(kModal)->modal);
  NVE_CHECK(host.step_tab_focus(false) == kModalBtn);
  NVE_CHECK(host.step_tab_focus(false) == kModalBtn);
  NVE_CHECK(host.step_tab_focus(true) == kModalBtn);
}

void test_scroll_into_view_and_key_destroy_safety() {
  constexpr uint32_t kScroll = 2u;
  constexpr uint32_t kTop = 3u;
  constexpr uint32_t kBottom = 4u;
  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kScroll);
  sized(w, kScroll, 200.0f, 100.0f);
  w.set(kScroll, NVE_TABLE_SCROLL_DESC, NVE_TABLE_SCROLL_AXES, NVE_SCROLL_AXES_VERTICAL);
  w.set_events(kScroll, NVE_TABLE_SCROLL_DESC, NVE_TABLE_SCROLL_EVENTS_WORD, NVE_SCROLL_EVENT_SET_SCROLL);
  w.create(kTop);
  sized(w, kTop, 180.0f, 220.0f);
  w.create(kBottom);
  sized(w, kBottom, 180.0f, 40.0f);
  const uint32_t skids[] = {kTop, kBottom};
  w.set_children(kScroll, skids);
  const uint32_t rkids[] = {kScroll};
  w.set_children(1u, rkids);

  auto host_owner = make_host();
  NVE_CHECK(host_owner->attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(host_owner->scroll_into_view(kBottom));
  NVE_CHECK(host_owner->visual_tree().find(kScroll)->scroll_y > 100.0f);
  NVE_CHECK(host_owner->scroll_into_view(kTop));
  NVE_CHECK(host_owner->visual_tree().find(kScroll)->scroll_y < 1.0f);

  // Destroying `NveWinHost` synchronously inside a key-driven change callback must not UAF.
  w.set_bool(kBottom, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  w.set_events(kBottom, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD,
               NVE_INTERACTION_EVENT_SET_KEY_DOWN | NVE_INTERACTION_EVENT_SET_PRESS);
  flush(*host_owner);
  (void)host_owner->set_focused_node(kBottom);
  NVE_CHECK(host_owner->focused_node() == kBottom);
  bool destroyed = false;
  app.on_event = [&](ScriptedApp& a, const h::testing::RecordedEvent& e) {
    if (e.ev.table == NVE_TABLE_INTERACTION_KEY && e.ev.event == NVE_INTERACTION_EVENT_KEY_DOWN) {
      a.w.set(kBottom, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BACKGROUND, h::rgba(10, 20, 30));
    }
  };
  host_owner->set_on_change([&]() {
    if (host_owner != nullptr) {
      destroyed = true;
      host_owner.reset();
    }
  });
  (void)host_owner->handle_key(true, "Enter", "Enter", 0u, 13u);
  NVE_CHECK(destroyed && host_owner == nullptr);
}

void test_frame0_file_and_bytes_boot() {
  const ScratchDir scratch("nve_win_frame0_test");
  const std::filesystem::path f0_path = scratch.path() / "frame0.nve2";

  {
    const auto writer_host = make_host();
    NVE_CHECK(writer_host->attach_interface(&CounterApp::get_interface));
    NVE_CHECK(writer_host->simulate_press(CounterApp::kIncrement));
    NVE_CHECK(node(*writer_host, CounterApp::kCount).text == "Count: 1");
    NVE_CHECK(writer_host->save_frame0_file(f0_path.string(), 42u, 99u));
  }

  // Read envelope bytes before any mismatch-quarantine test deletes the file on disk.
  std::ifstream in(f0_path, std::ios::binary);
  const std::vector<uint8_t> bytes((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
  NVE_CHECK(!bytes.empty());

  // Cold boot from file onto a fresh host with zero app attached.
  {
    const auto cold_host = make_host();
    NVE_CHECK(cold_host->boot_frame0_file(f0_path.string(), 42u, 99u));
    NVE_CHECK(cold_host->view_count() == 5u);
    NVE_CHECK(node(*cold_host, CounterApp::kCount).text == "Count: 1");
  }

  // Cold boot from in-memory bytes + corrupt / build_id-mismatch refusal.
  {
    const auto mem_host = make_host();
    const uint8_t garbage[] = {0x4E, 0x56, 0x00, 0xFF};
    NVE_CHECK(!mem_host->boot_frame0_bytes(garbage, 42u, 99u));
    NVE_CHECK(!mem_host->boot_frame0_bytes(bytes, 41u, 99u));
    NVE_CHECK(mem_host->boot_frame0_bytes(bytes, 42u, 99u));
    NVE_CHECK(mem_host->view_count() == 5u);
    NVE_CHECK(node(*mem_host, CounterApp::kCount).text == "Count: 1");
  }

  // File boot with build_id mismatch refuses and quarantines the stale file.
  {
    const auto mismatch_host = make_host();
    NVE_CHECK(!mismatch_host->boot_frame0_file(f0_path.string(), 41u, 99u));
  }
}

void test_multiline_crlf_utf16_offsets_and_numeric_formatting() {
  // Multiline LF <-> CRLF UTF-16 offset round-trip across lines 1, 2, and 3 (including a 4-byte emoji).
  const std::string text = "a\xF0\x9F\x98\x80\nbc\ndef";  // LF u16: a(0) emoji(1..2) \n(3) b(4) c(5) \n(6) d(7) e(8) f(9)
  // CRLF u16: a(0) emoji(1..2) \r(3) \n(4) b(5) c(6) \r(7) \n(8) d(9) e(10) f(11)
  NVE_CHECK(utf16_offset_for_char_index(text, 0u) == 0u);
  NVE_CHECK(utf16_offset_for_char_index(text, 2u) == 3u);  // after `a😀`, before line-1 `\n`
  NVE_CHECK(utf16_offset_for_char_index(text, 3u) == 4u);  // line 2 start (`b`)
  NVE_CHECK(utf16_offset_for_char_index(text, 6u) == 7u);  // line 3 start (`d`)
  NVE_CHECK(utf16_offset_for_char_index(text, 9u) == 10u); // end of line 3

  NVE_CHECK(crlf_utf16_offset_from_lf(text, 0u) == 0u);
  NVE_CHECK(crlf_utf16_offset_from_lf(text, 3u) == 3u);   // end of line 1 (before `\r\n`)
  NVE_CHECK(crlf_utf16_offset_from_lf(text, 4u) == 5u);   // start of line 2 (`b`)
  NVE_CHECK(crlf_utf16_offset_from_lf(text, 6u) == 7u);   // end of line 2 (before 2nd `\r\n`)
  NVE_CHECK(crlf_utf16_offset_from_lf(text, 7u) == 9u);   // start of line 3 (`d`)
  NVE_CHECK(crlf_utf16_offset_from_lf(text, 10u) == 12u); // end of line 3

  NVE_CHECK(lf_utf16_offset_from_crlf(text, 0u) == 0u);
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 3u) == 3u);   // before line-1 `\r\n`
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 4u) == 3u);   // between `\r` and `\n` snaps before `\n`
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 5u) == 4u);   // line 2 `b`
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 7u) == 6u);   // before line-2 `\r\n`
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 8u) == 6u);   // between 2nd `\r` and `\n` snaps before `\n`
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 9u) == 7u);   // line 3 `d`
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 12u) == 10u); // end of line 3
  NVE_CHECK(lf_utf16_offset_from_crlf(text, 99u) == 10u); // clamped past end

  NVE_CHECK(format_numeric_value(42.5) == "42.5");
  NVE_CHECK(format_numeric_value(std::numeric_limits<double>::quiet_NaN()).empty());
  NVE_CHECK(format_numeric_value(std::numeric_limits<double>::infinity()).empty());
  NVE_CHECK(sanitize_single_line("a\r\nb\nc") == "abc");
}

}  // namespace

void nve_test_win_keyboard_ime_and_frame0() {
  test_keyboard_focus_tab_modal_and_activation();
  test_scroll_into_view_and_key_destroy_safety();
  test_multiline_crlf_utf16_offsets_and_numeric_formatting();
  test_frame0_file_and_bytes_boot();
}
