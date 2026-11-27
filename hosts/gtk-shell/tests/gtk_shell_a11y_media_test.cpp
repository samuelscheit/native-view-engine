// GTK host: accessibility (roles, construct-only role changes, states, properties), controlled
// text inputs (entry / multiline / secure / search, IME echo, submit, focus), images (data URL,
// inline bytes, file URL, invalid, LRU cache, fit), CSS (visual, shadows, inherited typography),
// visibility, cursors, and removal.
#include "gtk_test_helpers.hpp"

#include <nve/host/core_resources.hpp>

#include <filesystem>
#include <fstream>
#include <vector>

namespace {

using namespace nve::gtk;
using namespace nve::gtk::test;
namespace h = nve::host;

constexpr uint32_t kColumn = 2u, kHeading = 3u, kCheck = 4u, kDisabled = 5u, kEntry = 6u, kSearch = 7u,
                   kNotes = 8u, kImgUrl = 9u, kImgBytes = 10u, kImgFile = 11u, kImgBad = 12u, kHidden = 13u;

std::vector<uint8_t> png(int w, int hgt, uint32_t rgba) {
  std::vector<uint8_t> px(static_cast<size_t>(w * hgt * 4));
  for (size_t i = 0; i < px.size(); i += 4) {
    for (size_t c = 0; c < 4; ++c) px[i + c] = static_cast<uint8_t>(rgba >> (8u * c));
  }
  GBytes* raw = g_bytes_new(px.data(), px.size());
  GdkTexture* tex = gdk_memory_texture_new(w, hgt, GDK_MEMORY_R8G8B8A8, raw, static_cast<gsize>(w) * 4u);
  GBytes* encoded = gdk_texture_save_to_png_bytes(tex);
  gsize len = 0;
  const auto* data = static_cast<const uint8_t*>(g_bytes_get_data(encoded, &len));
  std::vector<uint8_t> out(data, data + len);
  g_bytes_unref(encoded);
  g_object_unref(tex);
  g_bytes_unref(raw);
  return out;
}

std::string data_url(const std::vector<uint8_t>& bytes) {
  gchar* b64 = g_base64_encode(bytes.data(), bytes.size());
  std::string url = std::string("data:image/png;base64,") + b64;
  g_free(b64);
  return url;
}

void a11y(h::Nve2Writer& w, uint32_t id, uint32_t role, std::string_view label) {
  w.set(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, role);
  if (!label.empty()) (void)w.set_str(id, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LABEL, label);
}

void image(h::Nve2Writer& w, uint32_t id, uint32_t res, std::span<const uint8_t> src, std::string_view alt) {
  w.create(id);
  w.define_resource(res, h::kImageSrcKind, src);
  w.set(id, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, res);
  if (!alt.empty()) (void)w.set_str(id, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_ALT, alt);
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_WIDTH, h::length_points(32.0f));
  w.set(id, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(16.0f));
}

GtkAccessibleRole role_of(GtkWidget* w) { return gtk_accessible_get_accessible_role(GTK_ACCESSIBLE(w)); }

void test_a11y_inputs_images_and_styles() {
  const ScratchDir scratch("nve-gtk-a11y-media");
  const std::string tmp = scratch.file("image.png");
  const std::vector<uint8_t> red = png(4, 3, h::rgba(255, 0, 0));
  { std::ofstream(tmp, std::ios::binary).write(reinterpret_cast<const char*>(red.data()), static_cast<std::streamsize>(red.size())); }

  ScriptedApp app;
  h::Nve2Writer& w = app.w;
  w.create(1u);
  w.create(kColumn);
  // Inherited typography from the column.
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET,
        NVE_TYPOGRAPHY_SET_FONT_STYLE | NVE_TYPOGRAPHY_SET_FONT_FAMILY | NVE_TYPOGRAPHY_SET_TEXT_DECORATION);
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_STYLE, NVE_FONT_STYLE_ITALIC);
  (void)w.set_str(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY, "Inter\";}");
  w.set(kColumn, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION, NVE_TEXT_DECORATION_UNDERLINE);

  text_node(w, kHeading, "Settings");
  a11y(w, kHeading, NVE_ARIA_ROLE_HEADING, "");
  w.set(kHeading, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_LEVEL, 2u);
  w.set(kHeading, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_MAX_LINES | NVE_TYPOGRAPHY_SET_TEXT_ALIGN);
  w.set(kHeading, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_MAX_LINES, 1u);
  w.set(kHeading, NVE_TABLE_TYPOGRAPHY_DESC, NVE_TABLE_TYPOGRAPHY_TEXT_ALIGN, NVE_TEXT_ALIGN_CENTER);

  w.create(kCheck);
  a11y(w, kCheck, NVE_ARIA_ROLE_CHECKBOX, "Agree");
  (void)w.set_str(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_HINT, "Accept the terms");
  w.set(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_CHECKED, NVE_ARIA_TRI_STATE_TRUE);
  w.set(kCheck, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENT_MASK, h::event_bit(NVE_EVENT_PRESS));
  w.set(kCheck, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_CURSOR, NVE_CURSOR_KIND_POINTER);
  w.set(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_COLOR, h::rgba(1, 2, 3));
  w.set_f32(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_WIDTH, 2.0f);
  w.set_f32(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BORDER_RADIUS, 6.0f);
  w.set_f32(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_OPACITY, 0.5f);
  const h::BoxShadow shadow[] = {{1.0f, 2.0f, 3.0f, 0.0f, h::rgba(0, 0, 0, 128), false}};
  w.define_resource(50u, h::kBoxShadowsKind, h::encode_box_shadows(shadow));
  w.set(kCheck, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_BOX_SHADOWS, 50u);
  w.set(kCheck, NVE_TABLE_LAYOUT_DESC, NVE_TABLE_LAYOUT_HEIGHT, h::length_points(20.0f));

  w.create(kDisabled);
  a11y(w, kDisabled, NVE_ARIA_ROLE_BUTTON, "Disabled");
  w.set(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_EVENT_MASK, h::event_bit(NVE_EVENT_PRESS));
  w.set_bool(kDisabled, NVE_TABLE_INTERACTION_DESC, NVE_TABLE_INTERACTION_DISABLED, true);

  w.create(kEntry);
  (void)w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "hello");
  (void)w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_PLACEHOLDER, "Name");
  w.set_bool(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_SECURE, true);
  w.set(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 12u);
  w.create(kSearch);
  w.set(kSearch, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_INPUT_KIND, NVE_INPUT_KIND_SEARCH);
  w.create(kNotes);
  w.set_bool(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MULTILINE, true);
  (void)w.set_str(kNotes, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "a\nb");

  const std::string url = data_url(png(2, 2, h::rgba(0, 255, 0)));
  const std::string file_url = "file://" + tmp;
  image(w, kImgUrl, 60u, h::encode_image_url(url), "Logo");
  w.set(kImgUrl, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_FIT, NVE_IMAGE_FIT_CONTAIN);
  image(w, kImgBytes, 61u, h::encode_image_bytes(red), "");
  image(w, kImgFile, 62u, h::encode_image_url(file_url), "");
  image(w, kImgBad, 63u, h::encode_image_url("data:image/png;base64,AAAA"), "");
  text_node(w, kHidden, "hidden");
  w.set(kHidden, NVE_TABLE_VISUAL_DESC, NVE_TABLE_VISUAL_VISIBILITY, NVE_VISIBILITY_HIDDEN);
  w.set_bool(kHidden, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_HIDDEN, true);

  const uint32_t kids[] = {kHeading, kCheck, kDisabled, kEntry, kSearch, kNotes, kImgUrl, kImgBytes, kImgFile, kImgBad, kHidden};
  w.set_children(kColumn, kids);
  const uint32_t root_kids[] = {kColumn};
  w.set_children(1u, root_kids);

  // Controlled inputs: echo every TextChange back as the new value.
  app.on_event = [](ScriptedApp& a, const nve::host::testing::RecordedEvent& e) {
    if (e.ev.kind == NVE_EVENT_TEXT_CHANGE) (void)a.w.set_str(e.ev.node_id, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, e.text);
  };

  NveGtkHost host;
  NVE_CHECK(host.attach_interface(&ScriptedApp::get_interface));
  NVE_CHECK(host.view_count() == 12u);

  // --- Accessibility.
  GtkWidget* heading = host.widget_for_node_id(kHeading);
  NVE_CHECK(role_of(heading) == GTK_ACCESSIBLE_ROLE_HEADING);
  auto sem = host.accessibility_info_for_node_id(kHeading);
  NVE_CHECK(sem && sem->level == 2u && sem->label == "Settings");
  GtkWidget* check = host.widget_for_node_id(kCheck);
  NVE_CHECK(role_of(check) == GTK_ACCESSIBLE_ROLE_CHECKBOX);
  sem = host.accessibility_info_for_node_id(kCheck);
  NVE_CHECK(sem && sem->label == "Agree" && sem->hint == "Accept the terms" && sem->checked == NVE_ARIA_TRI_STATE_TRUE);
  NVE_CHECK(gtk_widget_get_sensitive(check) && !gtk_widget_get_sensitive(host.widget_for_node_id(kDisabled)));
  NVE_CHECK(host.accessibility_info_for_node_id(kDisabled)->disabled);
  NVE_CHECK(host.accessibility_info_for_node_id(kHidden)->hidden);
  NVE_CHECK(!gtk_widget_get_visible(host.widget_for_node_id(kHidden)));
  NVE_CHECK(gtk_widget_get_cursor(check) != nullptr &&
            std::string(gdk_cursor_get_name(gtk_widget_get_cursor(check))) == "pointer");
  NVE_CHECK(approx(gtk_widget_get_opacity(check), 0.5, 0.01));
  NVE_CHECK(!host.accessibility_info_for_node_id(999u));

  // --- Styles: visual, shadow resource, inherited + escaped typography, label layout props.
  const std::string& css = host.active_css();
  NVE_CHECK(css.find("border: 2px solid rgba(1, 2, 3, 1)") != std::string::npos);
  NVE_CHECK(css.find("border-radius: 6px") != std::string::npos);
  NVE_CHECK(css.find("box-shadow: 1px 2px 3px 0px rgba(0, 0, 0, 0.502)") != std::string::npos);
  NVE_CHECK(css.find("font-style: italic") != std::string::npos);
  NVE_CHECK(css.find("font-family: \"Inter\"") != std::string::npos);  // quotes / braces stripped
  NVE_CHECK(css.find("text-decoration-line: underline") != std::string::npos);
  NVE_CHECK(gtk_label_get_lines(GTK_LABEL(heading)) == 1 && gtk_label_get_ellipsize(GTK_LABEL(heading)) == PANGO_ELLIPSIZE_END);
  NVE_CHECK(approx(gtk_label_get_xalign(GTK_LABEL(heading)), 0.5, 0.01));

  // --- Text inputs.
  GtkWidget* entry = host.widget_for_node_id(kEntry);
  NVE_CHECK(GTK_IS_ENTRY(entry) && role_of(entry) == GTK_ACCESSIBLE_ROLE_TEXT_BOX);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "hello");
  NVE_CHECK(!gtk_entry_get_visibility(GTK_ENTRY(entry)) && gtk_entry_get_max_length(GTK_ENTRY(entry)) == 12);
  NVE_CHECK(std::string(gtk_entry_get_placeholder_text(GTK_ENTRY(entry))) == "Name");
  NVE_CHECK(role_of(host.widget_for_node_id(kSearch)) == GTK_ACCESSIBLE_ROLE_SEARCH_BOX);
  GtkWidget* notes = host.widget_for_node_id(kNotes);
  NVE_CHECK(GTK_IS_TEXT_VIEW(notes) && host.accessibility_info_for_node_id(kNotes)->multiline);

  // Replacing text is a native delete + insert: two TextChanges ("" then "Ada"), echoed stably.
  gtk_editable_set_text(GTK_EDITABLE(entry), "Ada");
  NVE_CHECK(app.count(NVE_EVENT_TEXT_CHANGE, kEntry) == 2u && app.last(NVE_EVENT_TEXT_CHANGE)->text == "Ada");
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "Ada");  // echo does not fight the user
  int at = 0;
  gtk_editable_insert_text(GTK_EDITABLE(entry), "!", 1, &at);  // typing one char: one event
  NVE_CHECK(app.count(NVE_EVENT_TEXT_CHANGE, kEntry) == 3u && app.last(NVE_EVENT_TEXT_CHANGE)->text == "!Ada");
  (void)app.w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "reset");  // app-driven value
  flush(host);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "reset");
  NVE_CHECK(app.count(NVE_EVENT_TEXT_CHANGE, kEntry) == 3u);  // programmatic sets never echo upstream
  // Shrinking the limit truncates natively: a host write, not a user edit.
  app.w.set(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 3u);
  flush(host);
  NVE_CHECK(std::string(gtk_editable_get_text(GTK_EDITABLE(entry))) == "res");
  NVE_CHECK(app.count(NVE_EVENT_TEXT_CHANGE, kEntry) == 3u);
  // Limits beyond GTK's range clamp to it instead of wrapping to a negative (unlimited) int.
  app.w.set(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_MAX_LENGTH, 0xFFFFFFFFu);
  flush(host);
  NVE_CHECK(gtk_entry_get_max_length(GTK_ENTRY(entry)) == GTK_ENTRY_BUFFER_MAX_SIZE);
  (void)app.w.set_str(kEntry, NVE_TABLE_TEXT_INPUT_DESC, NVE_TABLE_TEXT_INPUT_VALUE, "reset!");
  flush(host);
  g_signal_emit_by_name(entry, "activate");
  NVE_CHECK(app.count(NVE_EVENT_SUBMIT, kEntry) == 1u && app.last(NVE_EVENT_SUBMIT)->text == "reset!");
  NVE_CHECK(host.dispatch_focus(kEntry, true) || app.count(NVE_EVENT_FOCUS, kEntry) == 1u);
  (void)host.dispatch_focus(kEntry, false);
  NVE_CHECK(app.count(NVE_EVENT_FOCUS, kEntry) == 1u && app.count(NVE_EVENT_BLUR, kEntry) == 1u);
  gtk_text_buffer_set_text(gtk_text_view_get_buffer(GTK_TEXT_VIEW(notes)), "line1\nline2", -1);
  NVE_CHECK(app.last(NVE_EVENT_TEXT_CHANGE)->text == "line1\nline2" && app.last(NVE_EVENT_TEXT_CHANGE)->ev.node_id == kNotes);

  // --- Images.
  const auto paintable = [&](uint32_t id) { return gtk_picture_get_paintable(GTK_PICTURE(host.widget_for_node_id(id))); };
  NVE_CHECK(paintable(kImgUrl) != nullptr && gdk_paintable_get_intrinsic_width(paintable(kImgUrl)) == 2);
  NVE_CHECK(paintable(kImgBytes) != nullptr && gdk_paintable_get_intrinsic_width(paintable(kImgBytes)) == 4);
  NVE_CHECK(paintable(kImgFile) != nullptr && gdk_paintable_get_intrinsic_height(paintable(kImgFile)) == 3);
  NVE_CHECK(paintable(kImgBad) == nullptr);
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(host.widget_for_node_id(kImgUrl))) == GTK_CONTENT_FIT_CONTAIN);
  NVE_CHECK(gtk_picture_get_content_fit(GTK_PICTURE(host.widget_for_node_id(kImgBytes))) == GTK_CONTENT_FIT_COVER);
  NVE_CHECK(host.accessibility_info_for_node_id(kImgUrl)->label == "Logo");
  NVE_CHECK(host.texture_cache().size() == 3u && host.texture_cache().misses() == 4u);
  // The same bytes under another resource id hit the cache.
  app.w.define_resource(64u, h::kImageSrcKind, h::encode_image_bytes(red));
  app.w.set(kImgBad, NVE_TABLE_IMAGE_DESC, NVE_TABLE_IMAGE_SRC, 64u);
  flush(host);
  NVE_CHECK(paintable(kImgBad) == paintable(kImgBytes) && host.texture_cache().hits() == 1u);

  // --- Construct-only role change recreates the widget in place (same parent slot).
  GtkWidget* column = host.widget_for_node_id(kColumn);
  g_object_add_weak_pointer(G_OBJECT(check), reinterpret_cast<gpointer*>(&check));
  app.w.set(kCheck, NVE_TABLE_ACCESSIBILITY_DESC, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_SWITCH);
  flush(host);
  GtkWidget* sw = host.widget_for_node_id(kCheck);
  NVE_CHECK(check == nullptr);  // the old widget was destroyed (its address may be reused)
  NVE_CHECK(role_of(sw) == GTK_ACCESSIBLE_ROLE_SWITCH);
  NVE_CHECK(gtk_widget_get_parent(sw) == column && gtk_widget_get_prev_sibling(sw) == heading);

  // --- Removal drops the widget and its CSS rule.
  const std::string cls = "-node-" + std::to_string(kImgFile) + " {";
  NVE_CHECK(host.active_css().find(cls) != std::string::npos);
  app.w.remove(kImgFile);
  const uint32_t fewer[] = {kHeading, kCheck, kDisabled, kEntry, kSearch, kNotes, kImgUrl, kImgBytes, kImgBad, kHidden};
  app.w.set_children(kColumn, fewer);
  flush(host);
  NVE_CHECK(host.widget_for_node_id(kImgFile) == nullptr && host.view_count() == 11u);
  NVE_CHECK(host.active_css().find(cls) == std::string::npos);
}

}  // namespace

void nve_test_gtk_a11y_and_media() { test_a11y_inputs_images_and_styles(); }
