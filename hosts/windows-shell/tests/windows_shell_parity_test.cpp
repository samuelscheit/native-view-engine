// Portable Slice 4 parity tests for `NveWinHost`: keyboard dispatch (`KeyDown`/`KeyUp`), Tab /
// Shift+Tab focus traversal with positive `tab_index` + modal scoping, Enter / Space activation
// respecting `disabled` / `TextInput`, IME composition events, `set_on_scene_sync`, and ABI v6
// Frame-0 file + bytes cold boot.
#include "windows_test_helpers.hpp"

#include <nve/host/demo_counter_app.hpp>

#include <filesystem>
#include <fstream>
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

  // Enter / Space on focused link opens href; on button dispatches Press; on TextInput does not press.
  const KeyResult link_act = host.handle_key(true, "Enter", "Enter", 0u, 13u);
  NVE_CHECK(link_act.handled && link_act.href == "https://nve.dev/docs");

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

  // Modal dialog traps Tab traversal inside the modal subtree.
  w.create(kModal);
  sized(w, kModal, 240.0f, 160.0f);
  w.set_bool(kModal, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_MODAL, true);
  make_btn(kModalBtn, 0, false);
  const uint32_t modal_kids[] = {kModalBtn};
  w.set_children(kModal, modal_kids);
  const uint32_t with_modal[] = {kBtnA, kBtnB, kLink, kInput, kModal};
  w.set_children(1u, with_modal);
  flush(host);
  NVE_CHECK(host.visual_tree().find(kModal) != nullptr && host.visual_tree().find(kModal)->modal);
  NVE_CHECK(host.step_tab_focus(false) == kModalBtn);
  NVE_CHECK(host.step_tab_focus(false) == kModalBtn);
  NVE_CHECK(host.step_tab_focus(true) == kModalBtn);
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

}  // namespace

void nve_test_win_keyboard_ime_and_frame0() {
  test_keyboard_focus_tab_modal_and_activation();
  test_frame0_file_and_bytes_boot();
}
