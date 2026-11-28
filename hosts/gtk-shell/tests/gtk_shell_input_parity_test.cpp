// GTK host Slice 4 parity tests: keyboard dispatch (GDK keyvals/modifiers, Tab/Shift+Tab focus
// traversal, Enter/Space activation respecting disabled/read_only), GtkIMContext IME preedit
// lifecycle (CompositionStart/Update/End, ControlledText guard while composing, focus-leave abort),
// GtkAccessible parity (AriaRelations, live_region, modal, focus sync), and ABI v6 Frame-0.
#include "gtk_test_helpers.hpp"

#include <gdk/gdkkeysyms.h>

#include <nve/host/core_resources.hpp>

#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

namespace {

using namespace nve::gtk;
using namespace nve::gtk::test;
namespace h = nve::host;

void test_keyboard_focus_activation_and_readonly() {
  constexpr uint32_t kCol = 2u, kBtn1 = 3u, kDisabled = 4u, kReadOnly = 5u, kBtn2 = 6u;
  ScriptedApp app;
  auto& w = app.w;
  w.create(1u);
  w.create(kCol);
  w.set_events(kCol, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD,
               NVE_INTERACTION_EVENT_SET_KEY_DOWN | NVE_INTERACTION_EVENT_SET_KEY_UP);

  w.create(kBtn1);
  w.set_bool(kBtn1, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  w.set_events(kBtn1, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD,
               NVE_INTERACTION_EVENT_SET_PRESS | NVE_INTERACTION_EVENT_SET_FOCUS | NVE_INTERACTION_EVENT_SET_BLUR |
                   NVE_INTERACTION_EVENT_SET_KEY_DOWN | NVE_INTERACTION_EVENT_SET_KEY_UP);

  w.create(kDisabled);
  w.set_bool(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  w.set_bool(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_DISABLED, true);
  w.set_events(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD,
               NVE_INTERACTION_EVENT_SET_PRESS | NVE_INTERACTION_EVENT_SET_KEY_DOWN);

  w.create(kReadOnly);
  (void)w.set_str(kReadOnly, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "locked");
  w.set_bool(kReadOnly, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_READ_ONLY, true);
  w.set_events(kReadOnly, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD,
               NVE_INTERACTION_EVENT_SET_FOCUS | NVE_INTERACTION_EVENT_SET_BLUR | NVE_INTERACTION_EVENT_SET_KEY_DOWN);

  w.create(kBtn2);
  w.set_bool(kBtn2, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  w.set_events(kBtn2, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENTS_WORD,
               NVE_INTERACTION_EVENT_SET_PRESS | NVE_INTERACTION_EVENT_SET_FOCUS | NVE_INTERACTION_EVENT_SET_BLUR);

  const uint32_t kids[] = {kBtn1, kDisabled, kReadOnly, kBtn2};
  w.set_children(kCol, kids);
  const uint32_t root_kids[] = {kCol};
  w.set_children(1u, root_kids);

  NveGtkHost host;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));

  // `read_only` is decoupled from `disabled`: sensitive & focusable, but non-editable.
  GtkWidget* ro_widget = host.widget_for_node_id(kReadOnly);
  NVE_CHECK(GTK_IS_ENTRY(ro_widget));
  NVE_CHECK(gtk_widget_get_sensitive(ro_widget));
  NVE_CHECK(!gtk_editable_get_editable(GTK_EDITABLE(ro_widget)));
  NVE_CHECK(!host.accessibility_info_for_node_id(kReadOnly)->disabled);
  NVE_CHECK(host.accessibility_info_for_node_id(kReadOnly)->read_only);
  NVE_CHECK(!host.set_focused_node(kDisabled));

  // Tab traversal skips disabled nodes and wraps cleanly forward & backward.
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kBtn1);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_FOCUS, kBtn1) == 1u);

  // Enter and Space activate focused pressable button + dispatch KeyDown/KeyUp with modifiers.
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Return, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kBtn1) == 1u);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_KEY_DOWN, kBtn1) >= 1u);
  (void)host.handle_gdk_key(false, GDK_KEY_Return, 0u, static_cast<GdkModifierType>(0));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_KEY_UP, kBtn1) == 1u);

  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_space, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_PRESS, kBtn1) == 2u);

  // Tab skips kDisabled -> focuses kReadOnly -> focuses kBtn2 -> Shift+Tab returns to kReadOnly.
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kReadOnly);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_BLUR, kBtn1) == 1u);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_FOCUS, kReadOnly) == 1u);

  // Ctrl+C on read_only dispatches KeyDown with MOD_CTRL and does not mutate text.
  (void)host.handle_gdk_key(true, GDK_KEY_c, 0u, GDK_CONTROL_MASK);
  const auto* last_kd = app.last(NVE_INTERACTION_EVENT_KEY_DOWN);
  NVE_CHECK(last_kd != nullptr && last_kd->ev.node == kReadOnly && last_kd->key() == "c");
  NVE_CHECK((last_kd->ev.flags & NVE_EVENT_FLAG_MOD_CTRL) != 0u);

  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kBtn2);
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_ISO_Left_Tab, 0u, GDK_SHIFT_MASK));
  NVE_CHECK(host.focused_node_id() == kReadOnly);
}

void test_ime_preedit_and_controlled_text_guard() {
  constexpr uint32_t kEntry = 2u;
  ScriptedApp app;
  auto& w = app.w;
  w.create(1u);
  w.create(kEntry);
  (void)w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "hi");
  w.set_events(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_EVENTS_WORD,
               NVE_TEXT_INPUT_EVENT_SET_CHANGE | NVE_TEXT_INPUT_EVENT_SET_COMPOSITION_START |
                   NVE_TEXT_INPUT_EVENT_SET_COMPOSITION_UPDATE | NVE_TEXT_INPUT_EVENT_SET_COMPOSITION_END);
  const uint32_t kids[] = {kEntry};
  w.set_children(1u, kids);

  app.on_event = [](ScriptedApp& a, const h::testing::RecordedEvent& e) {
    if (h::is_text_input_event(e.ev, NVE_TEXT_INPUT_EVENT_CHANGE)) {
      (void)a.w.set_str(e.ev.node, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, e.text);
    }
  };

  NveGtkHost host;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  GtkWidget* entry = host.widget_for_node_id(kEntry);
  GtkIMContext* im = host.im_context_for_node_id(kEntry);
  NVE_CHECK(im != nullptr);
  NVE_CHECK(host.set_focused_node(kEntry));
  gtk_editable_set_position(GTK_EDITABLE(entry), -1);

  // Start IME composition via real GtkIMContext signals.
  g_signal_emit_by_name(im, "preedit-start");
  NVE_CHECK(host.is_composing_node(kEntry));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_COMPOSITION_START, kEntry) == 1u);

  // While composing, app-side value updates MUST NOT clobber the editable widget.
  (void)app.w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "stale-remote");
  flush(host);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "hi");

  // Commit ends composition and flushes TextChange cleanly.
  g_signal_emit_by_name(im, "commit", "漢");
  NVE_CHECK(!host.is_composing_node(kEntry));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_COMPOSITION_END, kEntry) == 1u);
  NVE_CHECK(app.last(NVE_TEXT_INPUT_EVENT_COMPOSITION_END)->text == "漢");
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kEntry) == 1u);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "hi漢");

  // Second composition (IBus/Wayland order: `preedit-end` fires right before `commit`) must guard
  // against mid-handoff app clobbering AND emit `COMPOSITION_END(committed)` strictly before `CHANGE`.
  g_signal_emit_by_name(im, "preedit-start");
  g_signal_emit_by_name(im, "preedit-changed");
  NVE_CHECK(host.is_composing_node(kEntry));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE, kEntry) >= 1u);
  const size_t ev_before_ibus = app.events.size();
  g_signal_emit_by_name(im, "preedit-end");
  // App update arriving between `preedit-end` and `commit` must NOT clobber the widget.
  (void)app.w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "mid-handoff-stale");
  flush(host);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "hi漢");
  g_signal_emit_by_name(im, "commit", "字");
  while (g_main_context_iteration(nullptr, FALSE)) {}
  NVE_CHECK(!host.is_composing_node(kEntry));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_COMPOSITION_END, kEntry) == 2u);
  NVE_CHECK(app.last(NVE_TEXT_INPUT_EVENT_COMPOSITION_END)->text == "字");
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_CHANGE, kEntry) == 2u);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "hi漢字");
  size_t end_idx = app.events.size(), change_idx = app.events.size();
  for (size_t i = ev_before_ibus; i < app.events.size(); ++i) {
    if (h::is_text_input_event(app.events[i].ev, NVE_TEXT_INPUT_EVENT_COMPOSITION_END)) end_idx = i;
    if (h::is_text_input_event(app.events[i].ev, NVE_TEXT_INPUT_EVENT_CHANGE)) change_idx = i;
  }
  NVE_CHECK(end_idx < change_idx && change_idx < app.events.size());

  // Third composition cancelled via `preedit-end` (no `commit`) emits empty `COMPOSITION_END`.
  g_signal_emit_by_name(im, "preedit-start");
  g_signal_emit_by_name(im, "preedit-end");
  while (g_main_context_iteration(nullptr, FALSE)) {}
  NVE_CHECK(!host.is_composing_node(kEntry));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_COMPOSITION_END, kEntry) == 3u);
  NVE_CHECK(app.last(NVE_TEXT_INPUT_EVENT_COMPOSITION_END)->text.empty());

  // Fourth composition aborted by blur emits `COMPOSITION_END` and never leaves composing stuck.
  g_signal_emit_by_name(im, "preedit-start");
  NVE_CHECK(host.is_composing_node(kEntry));
  NVE_CHECK(host.set_focused_node(0u));
  NVE_CHECK(!host.is_composing_node(kEntry));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_COMPOSITION_END, kEntry) == 4u);

  // Disabling a focused/composing input clears focus + composing and flushes `COMPOSITION_END` + `BLUR`.
  NVE_CHECK(host.set_focused_node(kEntry));
  g_signal_emit_by_name(im, "preedit-start");
  NVE_CHECK(host.is_composing_node(kEntry));
  const size_t blurs_before = app.count(NVE_INTERACTION_EVENT_BLUR, kEntry);
  w.set_bool(kEntry, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_DISABLED, true);
  flush(host);
  NVE_CHECK(host.focused_node_id() == 0u && !host.is_composing_node(kEntry));
  NVE_CHECK(app.count(NVE_TEXT_INPUT_EVENT_COMPOSITION_END, kEntry) == 5u);
  NVE_CHECK(app.count(NVE_INTERACTION_EVENT_BLUR, kEntry) == blurs_before + 1u);
}

void test_tab_index_order_and_modal_trap() {
  constexpr uint32_t kBtnA = 2u, kBtnB = 3u, kSkip = 4u, kModal = 5u, kModalBtn = 6u;
  ScriptedApp app;
  auto& w = app.w;
  w.create(1u);
  w.create(kBtnA);
  w.set_bool(kBtnA, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  w.set(kBtnA, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_TAB_INDEX, 0u);
  w.create(kBtnB);
  w.set_bool(kBtnB, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  w.set(kBtnB, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_TAB_INDEX, 1u);
  w.create(kSkip);
  w.set_bool(kSkip, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  w.set(kSkip, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_TAB_INDEX,
        static_cast<uint32_t>(static_cast<uint16_t>(static_cast<int16_t>(-1))));
  const uint32_t kids[] = {kBtnA, kBtnB, kSkip};
  w.set_children(1u, kids);

  NveGtkHost host;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kBtnB);  // positive tab_index first
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kBtnA);  // negative tab_index skipped
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kBtnB);

  // Empty modal traps & consumes Tab without escaping to background buttons.
  w.create(kModal);
  w.set_bool(kModal, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_MODAL, true);
  const uint32_t with_empty_modal[] = {kBtnA, kBtnB, kSkip, kModal};
  w.set_children(1u, with_empty_modal);
  flush(host);
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kBtnB);

  // Populated modal scopes Tab strictly inside the modal subtree.
  w.create(kModalBtn);
  w.set_bool(kModalBtn, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_FOCUSABLE, true);
  const uint32_t mkids[] = {kModalBtn};
  w.set_children(kModal, mkids);
  flush(host);
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kModalBtn);
  NVE_CHECK(host.handle_gdk_key(true, GDK_KEY_Tab, 0u, static_cast<GdkModifierType>(0)));
  NVE_CHECK(host.focused_node_id() == kModalBtn);
}

void test_a11y_relations_live_modal_and_frame0() {
  constexpr uint32_t kDialog = 2u, kTitle = 3u, kDesc = 4u, kList = 5u, kOpt1 = 6u, kLive = 7u;
  ScriptedApp app;
  auto& w = app.w;
  w.create(1u);
  w.create(kDialog);
  w.set(kDialog, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_DIALOG);
  w.set_bool(kDialog, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_MODAL, true);
  w.define_resource(90u, h::kAriaRelationsKind,
                    h::encode_aria_relations(h::AriaRelations{"opts-list", "dlg-title", "dlg-desc", ""}));
  w.set(kDialog, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_RELATIONS, 90u);

  text_node(w, kTitle, "Confirm");
  (void)w.set_str(kTitle, NVE_TABLE_DEBUG_ID_DESC, NVE_TABLE_DEBUG_ID_ID, "dlg-title");

  text_node(w, kDesc, "Are you sure?");
  (void)w.set_str(kDesc, NVE_TABLE_DEBUG_ID_DESC, NVE_TABLE_DEBUG_ID_ID, "dlg-desc");

  w.create(kList);
  (void)w.set_str(kList, NVE_TABLE_DEBUG_ID_DESC, NVE_TABLE_DEBUG_ID_ID, "opts-list");
  w.set(kList, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_LIST_BOX);
  w.define_resource(91u, h::kAriaRelationsKind, h::encode_aria_relations(h::AriaRelations{"", "", "", "opt-1"}));
  w.set(kList, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_RELATIONS, 91u);

  text_node(w, kOpt1, "Yes");
  (void)w.set_str(kOpt1, NVE_TABLE_DEBUG_ID_DESC, NVE_TABLE_DEBUG_ID_ID, "opt-1");
  w.set(kOpt1, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_OPTION);

  text_node(w, kLive, "Ready");
  w.set(kLive, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LIVE, NVE_ARIA_LIVE_POLITE);

  const uint32_t list_kids[] = {kOpt1};
  w.set_children(kList, list_kids);
  const uint32_t dlg_kids[] = {kTitle, kDesc, kList, kLive};
  w.set_children(kDialog, dlg_kids);
  const uint32_t root_kids[] = {kDialog};
  w.set_children(1u, root_kids);

  NveGtkHost host;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));

  const auto dlg_sem = host.accessibility_info_for_node_id(kDialog);
  NVE_CHECK(dlg_sem && dlg_sem->modal);
  NVE_CHECK(dlg_sem->labelled_by == "dlg-title" && dlg_sem->described_by == "dlg-desc" &&
            dlg_sem->controls == "opts-list");
  NVE_CHECK((host.accessible_relation_targets(kDialog, GTK_ACCESSIBLE_RELATION_LABELLED_BY) ==
             std::vector<GtkWidget*>{host.widget_for_node_id(kTitle)}));
  NVE_CHECK((host.accessible_relation_targets(kDialog, GTK_ACCESSIBLE_RELATION_DESCRIBED_BY) ==
             std::vector<GtkWidget*>{host.widget_for_node_id(kDesc)}));
  NVE_CHECK((host.accessible_relation_targets(kDialog, GTK_ACCESSIBLE_RELATION_CONTROLS) ==
             std::vector<GtkWidget*>{host.widget_for_node_id(kList)}));
  NVE_CHECK((host.accessible_relation_targets(kList, GTK_ACCESSIBLE_RELATION_ACTIVE_DESCENDANT) ==
             std::vector<GtkWidget*>{host.widget_for_node_id(kOpt1)}));

  // Live region announces initial non-empty message and subsequent updates.
  NVE_CHECK(host.live_announcement_count() == 1u && host.last_live_announcement() == "Ready");
  (void)app.w.set_str(kLive, NVE_TABLE_TEXT_DESC, NVE_TABLE_TEXT_TEXT, "Saved!");
  flush(host);
  NVE_CHECK(host.live_announcement_count() == 2u && host.last_live_announcement() == "Saved!");

  // ABI v6 Frame-0 file save + instant boot (file & bytes) + mismatch/corrupt refusal.
  const ScratchDir scratch("nve-gtk-frame0");
  const std::string f0_path = scratch.file("frame0.nve2");
  NVE_CHECK(host.save_frame0_file(f0_path, 42u, 99u));
  NVE_CHECK(std::filesystem::exists(f0_path));

  std::ifstream in(f0_path, std::ios::binary);
  std::vector<uint8_t> raw((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
  NVE_CHECK(!raw.empty());

  NveGtkHost cold_file;
  NVE_CHECK(cold_file.boot_frame0_file(f0_path, 42u, 99u));
  NVE_CHECK(cold_file.view_count() == host.view_count());
  NVE_CHECK(label_text(cold_file.widget_for_node_id(kLive)) == "Saved!");
  NVE_CHECK(cold_file.accessibility_info_for_node_id(kDialog)->modal);

  NveGtkHost cold_bytes;
  const uint8_t corrupt[] = {0x4E, 0x56, 0x00, 0xFF};
  NVE_CHECK(!cold_bytes.boot_frame0_bytes(corrupt, 42u, 99u));
  NVE_CHECK(!cold_bytes.boot_frame0_bytes(raw, 41u, 99u));
  NVE_CHECK(cold_bytes.boot_frame0_bytes(raw, 42u, 99u));
  NVE_CHECK(cold_bytes.view_count() == host.view_count());
  NVE_CHECK(label_text(cold_bytes.widget_for_node_id(kTitle)) == "Confirm");

  NveGtkHost mismatch_file;
  NVE_CHECK(!mismatch_file.boot_frame0_file(f0_path, 41u, 99u));
}

}  // namespace

void nve_test_gtk_input_parity() {
  test_keyboard_focus_activation_and_readonly();
  test_ime_preedit_and_controlled_text_guard();
  test_tab_index_order_and_modal_trap();
  test_a11y_relations_live_modal_and_frame0();
}
