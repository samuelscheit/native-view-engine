#include "jni_test_recorder.h"
#include "nve_scene_abi.h"

#include <jni.h>

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

extern "C" {
JNIEXPORT jlong JNICALL Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
    JNIEnv* env,
    jclass clazz,
    jint surface_id,
    jfloat width,
    jfloat height,
    jfloat scale_factor);

JNIEXPORT void JNICALL Java_dev_nve_android_NveMainActivity_nativeDestroySurface(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jboolean JNICALL Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jstring so_path);

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeDispatchPress(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jint node_id,
    jfloat x,
    jfloat y);

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jint node_id,
    jstring name,
    jstring payload);

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeDispatchScroll(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jint node_id,
    jfloat scroll_x,
    jfloat scroll_y);

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jlongArray out_meta);

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeGetImageResizeMode(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jint node_id);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetChildrenSlabBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetRemovedIdsBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetMetadataBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);
}

namespace {

struct FakeJString {
  std::string value;
};

struct FakeLongArray {
  std::vector<jlong> data;
};

struct FakeDirectBuffer {
  void* address{nullptr};
  jlong capacity{0};
  int ref_count{1};
};

struct ParsedMetaRecord {
  uint32_t node_id{0};
  uint32_t flags{0};
  std::string label;
  std::string role;
  std::string hint;
  std::string state_desc;
  std::string placeholder;
  std::string test_id;
  std::vector<uint8_t> draw_list;
};

uint16_t read_u16_le(const uint8_t* p) {
  return static_cast<uint16_t>(p[0]) | (static_cast<uint16_t>(p[1]) << 8);
}

uint32_t read_u32_le(const uint8_t* p) {
  return static_cast<uint32_t>(p[0]) |
         (static_cast<uint32_t>(p[1]) << 8) |
         (static_cast<uint32_t>(p[2]) << 16) |
         (static_cast<uint32_t>(p[3]) << 24);
}

uint64_t read_u64_le(const uint8_t* p) {
  uint64_t v = 0;
  for (size_t i = 0; i < 8; ++i) {
    v |= static_cast<uint64_t>(p[i]) << (i * 8);
  }
  return v;
}

std::unordered_map<uint32_t, ParsedMetaRecord> parse_metadata_slab(
    const FakeDirectBuffer* buf,
    uint64_t expected_frame_id,
    uint32_t expected_used_bytes) {
  NVE_CHECK(buf != nullptr && buf->address != nullptr);
  NVE_CHECK(buf->capacity >= 32);
  const auto* data = reinterpret_cast<const uint8_t*>(buf->address);
  NVE_CHECK(std::memcmp(data, "NVE_MET1", 8) == 0);
  NVE_CHECK(read_u16_le(data + 8) == 1u);
  const uint32_t record_count = read_u32_le(data + 12);
  const uint64_t frame_id = read_u64_le(data + 16);
  const uint32_t total_bytes = read_u32_le(data + 24);
  NVE_CHECK(frame_id == expected_frame_id);
  NVE_CHECK(total_bytes == expected_used_bytes);
  NVE_CHECK(static_cast<jlong>(total_bytes) <= buf->capacity);

  std::unordered_map<uint32_t, ParsedMetaRecord> out;
  size_t pos = 32;
  for (uint32_t i = 0; i < record_count; ++i) {
    NVE_CHECK(total_bytes - pos >= 28);
    ParsedMetaRecord rec{};
    rec.node_id = read_u32_le(data + pos + 0);
    rec.flags = read_u32_le(data + pos + 4);
    const uint16_t label_len = read_u16_le(data + pos + 8);
    const uint16_t role_len = read_u16_le(data + pos + 10);
    const uint16_t hint_len = read_u16_le(data + pos + 12);
    const uint16_t state_len = read_u16_le(data + pos + 14);
    const uint16_t placeholder_len = read_u16_le(data + pos + 16);
    const uint16_t test_id_len = read_u16_le(data + pos + 18);
    const uint32_t draw_len = read_u32_le(data + pos + 20);
    pos += 28;

    auto read_str = [&](size_t len) -> std::string {
      NVE_CHECK(pos + len <= total_bytes);
      std::string s(reinterpret_cast<const char*>(data + pos), len);
      pos += len;
      return s;
    };
    rec.label = read_str(label_len);
    rec.role = read_str(role_len);
    rec.hint = read_str(hint_len);
    rec.state_desc = read_str(state_len);
    rec.placeholder = read_str(placeholder_len);
    rec.test_id = read_str(test_id_len);
    NVE_CHECK(pos + draw_len <= total_bytes);
    rec.draw_list.assign(data + pos, data + pos + draw_len);
    pos += draw_len;

    out[rec.node_id] = std::move(rec);
  }
  NVE_CHECK(pos == total_bytes);
  NVE_CHECK(out.size() == record_count);
  return out;
}

int g_active_utf_borrows = 0;
int g_direct_buffer_allocations = 0;
std::unordered_set<FakeDirectBuffer*> g_live_global_refs;
std::vector<FakeDirectBuffer*> g_all_direct_buffers;

const char* JNICALL fake_get_string_utf_chars(JNIEnv*, jstring str, jboolean* is_copy) {
  if (is_copy) *is_copy = JNI_FALSE;
  g_active_utf_borrows += 1;
  return reinterpret_cast<const FakeJString*>(str)->value.c_str();
}

void JNICALL fake_release_string_utf_chars(JNIEnv*, jstring, const char*) {
  g_active_utf_borrows -= 1;
}

jsize JNICALL fake_get_array_length(JNIEnv*, jarray arr) {
  return static_cast<jsize>(reinterpret_cast<const FakeLongArray*>(arr)->data.size());
}

void JNICALL fake_set_long_array_region(
    JNIEnv*,
    jlongArray arr,
    jsize start,
    jsize len,
    const jlong* buf) {
  auto* target = reinterpret_cast<FakeLongArray*>(arr);
  for (jsize i = 0; i < len; ++i) {
    target->data[static_cast<size_t>(start + i)] = buf[i];
  }
}

jobject JNICALL fake_new_direct_byte_buffer(JNIEnv*, void* address, jlong capacity) {
  auto* buf = new FakeDirectBuffer{address, capacity, 1};
  g_direct_buffer_allocations += 1;
  g_all_direct_buffers.push_back(buf);
  return reinterpret_cast<jobject>(buf);
}

jobject JNICALL fake_new_global_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    buf->ref_count += 1;
    g_live_global_refs.insert(buf);
  }
  return obj;
}

void JNICALL fake_delete_global_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    NVE_CHECK(g_live_global_refs.erase(buf) == 1u);
    buf->ref_count -= 1;
  }
}

jobject JNICALL fake_new_local_ref(JNIEnv*, jobject obj) {
  return obj;
}

void JNICALL fake_delete_local_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    buf->ref_count -= 1;
  }
}

void* JNICALL fake_get_direct_buffer_address(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->address;
}

jlong JNICALL fake_get_direct_buffer_capacity(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->capacity;
}

JNIEnv_ make_fake_env(JNINativeInterface_& table) {
  table = JNINativeInterface_{};
  table.GetStringUTFChars = &fake_get_string_utf_chars;
  table.ReleaseStringUTFChars = &fake_release_string_utf_chars;
  table.GetArrayLength = &fake_get_array_length;
  table.SetLongArrayRegion = &fake_set_long_array_region;
  table.NewDirectByteBuffer = &fake_new_direct_byte_buffer;
  table.NewGlobalRef = &fake_new_global_ref;
  table.DeleteGlobalRef = &fake_delete_global_ref;
  table.NewLocalRef = &fake_new_local_ref;
  table.DeleteLocalRef = &fake_delete_local_ref;
  table.GetDirectBufferAddress = &fake_get_direct_buffer_address;
  table.GetDirectBufferCapacity = &fake_get_direct_buffer_capacity;
  JNIEnv_ env{};
  env.functions = &table;
  return env;
}

void cleanup_fake_buffers() {
  for (auto* b : g_all_direct_buffers) {
    delete b;
  }
  g_all_direct_buffers.clear();
}

void test_direct_scene_view_cached_buffers_metadata_draw1_and_events(
    const std::string& mock_scene_so) {
  JNINativeInterface_ table{};
  JNIEnv_ env = make_fake_env(table);
  g_direct_buffer_allocations = 0;

  const jlong handle = Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
      &env, nullptr, 1, 412.0f, 915.0f, 2.0f);
  NVE_CHECK(handle != 0);

  FakeJString so_str{mock_scene_so};
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&so_str)) == JNI_TRUE);
  NVE_CHECK(g_active_utf_borrows == 0);

  // Frame 1: 8 nodes (root=1, text=2, pressable=3, banner=4, image=5, scroll=6, textInput=7, canvas=8).
  FakeLongArray meta{std::vector<jlong>(8, 0)};
  const jint count1 = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(count1 == 8);
  NVE_CHECK(static_cast<uint64_t>(meta.data[0]) == NVE_SCENE_SCHEMA_HASH);
  NVE_CHECK(meta.data[1] == 1); // frame_id
  NVE_CHECK(meta.data[2] == 8); // node_count
  NVE_CHECK(meta.data[3] == 8); // dirty_count
  NVE_CHECK(meta.data[4] == 0); // removed_count
  NVE_CHECK(meta.data[7] > 32); // batched NVE_MET1 metadata bytes

  // All 6 DirectByteBuffers allocated once on Frame 1 -> epoch mask == 0x3F, 6 live GlobalRefs.
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(&env, nullptr, handle) == 0x3F);
  NVE_CHECK(g_direct_buffer_allocations == 6);
  NVE_CHECK(g_live_global_refs.size() == 6u);

  auto* nodes_buf1 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle));
  auto* text_buf1 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(&env, nullptr, handle));
  auto* child_buf1 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetChildrenSlabBuffer(&env, nullptr, handle));
  auto* dirty_buf1 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(&env, nullptr, handle));
  auto* removed_buf1 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetRemovedIdsBuffer(&env, nullptr, handle));
  auto* meta_buf1 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetMetadataBuffer(&env, nullptr, handle));

  NVE_CHECK(nodes_buf1->capacity == 8 * static_cast<jlong>(sizeof(NveRenderNode)));
  NVE_CHECK(dirty_buf1->capacity == 8 * static_cast<jlong>(sizeof(uint32_t)));
  NVE_CHECK(child_buf1->capacity == 7 * static_cast<jlong>(sizeof(uint32_t)));
  NVE_CHECK(removed_buf1->capacity == 0);

  // Verify batched dirty-node NVE_MET1 metadata slab packs ONLY nodes with metadata (3, 5, 7);
  // node 8's valid NVE_DRAW1 drawList is already delivered zero-copy via `NveSceneView::text_slab`
  // by FabricHost/ScenePublisher, so NVE_MET1 intentionally avoids duplicating it!
  const auto meta1 = parse_metadata_slab(meta_buf1, 1u, static_cast<uint32_t>(meta.data[7]));
  NVE_CHECK(meta1.size() == 3u);
  NVE_CHECK(
      meta1.count(1) == 0 && meta1.count(2) == 0 && meta1.count(4) == 0 && meta1.count(6) == 0 &&
      meta1.count(8) == 0);

  const auto& m3 = meta1.at(3);
  NVE_CHECK(m3.label == "Increment counter");
  NVE_CHECK(m3.role == "button");
  NVE_CHECK(m3.hint == "Adds one");
  NVE_CHECK((m3.flags & (1u << 0)) == 0u); // not disabled

  const auto& m5 = meta1.at(5);
  NVE_CHECK(((m5.flags >> 8) & 0x3u) == 1u); // resizeMode="contain"

  const auto& m7 = meta1.at(7);
  NVE_CHECK(m7.placeholder == "Enter secret");
  NVE_CHECK((m7.flags & (1u << 1)) != 0u); // secureTextEntry == true
  NVE_CHECK(m7.label == "Secret input");

  const auto* nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf1->address);
  const auto* text_slab = reinterpret_cast<const char*>(text_buf1->address);
  const auto* children_slab = reinterpret_cast<const uint32_t*>(child_buf1->address);

  bool saw_root = false;
  bool saw_label = false;
  bool saw_pressable = false;
  bool saw_image = false;
  bool saw_scroll = false;
  bool saw_input = false;
  bool saw_canvas = false;
  for (int i = 0; i < 8; ++i) {
    if (nodes[i].node_id == 1) {
      saw_root = true;
      NVE_CHECK(nodes[i].child_count == 7);
      NVE_CHECK(children_slab[nodes[i].child_offset + 0] == 2);
      NVE_CHECK(children_slab[nodes[i].child_offset + 6] == 8);
    } else if (nodes[i].node_id == 2) {
      saw_label = true;
      NVE_CHECK((nodes[i].flags & NVE_NODE_LEAF_TEXT) != 0);
      NVE_CHECK(std::string_view(text_slab + nodes[i].text_offset, nodes[i].text_len) == "Count: 0 🚀");
    } else if (nodes[i].node_id == 3) {
      saw_pressable = true;
      NVE_CHECK((nodes[i].flags & NVE_NODE_INTERACTIVE) != 0);
    } else if (nodes[i].node_id == 5) {
      saw_image = true;
      NVE_CHECK(nodes[i].kind == NVE_NODE_KIND_IMAGE);
      const std::string_view img_src(text_slab + nodes[i].text_offset, nodes[i].text_len);
      NVE_CHECK(img_src.rfind("data:image/png;base64,", 0) == 0);
      NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeGetImageResizeMode(&env, nullptr, handle, 5) == 1);
    } else if (nodes[i].node_id == 6) {
      saw_scroll = true;
      NVE_CHECK(nodes[i].kind == NVE_NODE_KIND_SCROLL_VIEW);
    } else if (nodes[i].node_id == 7) {
      saw_input = true;
      NVE_CHECK(nodes[i].kind == NVE_NODE_KIND_TEXT_INPUT);
    } else if (nodes[i].node_id == 8) {
      saw_canvas = true;
      NVE_CHECK(nodes[i].kind == NVE_NODE_KIND_SKIA_CANVAS);
      NVE_CHECK(nodes[i].text_len > 15u);
      const auto* dl = reinterpret_cast<const uint8_t*>(text_slab + nodes[i].text_offset);
      NVE_CHECK(std::memcmp(dl, "NVE_DRAW1", 9) == 0);
      NVE_CHECK(read_u32_le(dl + 11) == 8u); // all 8 opcodes zero-copy in text_slab!
    }
  }
  NVE_CHECK(saw_root && saw_label && saw_pressable && saw_image && saw_scroll && saw_input && saw_canvas);

  // Calling nativeRenderFrame again with zero changes MUST reuse all 6 cached GlobalRefs (mask == 0).
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
          &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta)) == 8);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(&env, nullptr, handle) == 0);
  NVE_CHECK(g_direct_buffer_allocations == 6);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle) ==
      reinterpret_cast<jobject>(nodes_buf1));
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeGetMetadataBuffer(&env, nullptr, handle) ==
      reinterpret_cast<jobject>(meta_buf1));

  // Press 1 -> updates ONLY node 2 text ("Count: 1 🚀"), zero metadata changes -> record_count == 0,
  // and metadata DirectByteBuffer GlobalRef reused with ZERO new allocations!
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeDispatchPress(&env, nullptr, handle, 3, 10.0f, 10.0f) ==
      1);
  const jint count2 = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(count2 == 8);
  NVE_CHECK(meta.data[3] == 1); // dirty_count == 1 (O(changed)!)
  NVE_CHECK(meta.data[4] == 0); // removed_count == 0
  NVE_CHECK(meta.data[7] == 32); // header-only NVE_MET1 (0 changed metadata records!)
  const jint mask2 = Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(&env, nullptr, handle);
  NVE_CHECK((mask2 & (1 << 0)) == 0); // nodes unchanged
  NVE_CHECK((mask2 & (1 << 1)) != 0); // text_slab grew
  NVE_CHECK((mask2 & (1 << 2)) == 0); // children_slab unchanged
  NVE_CHECK((mask2 & (1 << 3)) != 0); // dirty_indices changed len (8 -> 1)
  NVE_CHECK((mask2 & (1 << 4)) == 0); // removed_ids still empty
  NVE_CHECK((mask2 & (1 << 5)) == 0); // metadata slab GlobalRef reused!
  NVE_CHECK(g_live_global_refs.size() == 6u);
  const auto meta2 = parse_metadata_slab(meta_buf1, static_cast<uint64_t>(meta.data[1]), 32u);
  NVE_CHECK(meta2.empty());

  auto* nodes_buf2 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle));
  auto* text_buf2 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(&env, nullptr, handle));
  auto* dirty_buf2 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(&env, nullptr, handle));
  NVE_CHECK(nodes_buf2 == nodes_buf1);
  NVE_CHECK(text_buf2 != text_buf1);
  nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf2->address);
  text_slab = reinterpret_cast<const char*>(text_buf2->address);
  const uint32_t dirty_idx = reinterpret_cast<const uint32_t*>(dirty_buf2->address)[0];
  NVE_CHECK(nodes[dirty_idx].node_id == 2);
  NVE_CHECK(
      std::string_view(text_slab + nodes[dirty_idx].text_offset, nodes[dirty_idx].text_len) ==
      "Count: 1 🚀");

  // Press 2 -> removes node 4 and updates node 1 children + node 2 text.
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeDispatchPress(&env, nullptr, handle, 3, 10.0f, 10.0f) ==
      1);
  const jint count3 = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(count3 == 7);
  NVE_CHECK(meta.data[4] == 1); // removed_count == 1
  auto* removed_buf3 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetRemovedIdsBuffer(&env, nullptr, handle));
  NVE_CHECK(removed_buf3->capacity == static_cast<jlong>(sizeof(uint32_t)));
  NVE_CHECK(reinterpret_cast<const uint32_t*>(removed_buf3->address)[0] == 4);

  // Scroll event on ScrollView node 6 dispatches zero-copy EventKind::Scroll with exact (x, y).
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeDispatchScroll(&env, nullptr, handle, 6, 18.0f, 142.0f) ==
      1);
  const jint count4 = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(count4 == 7);
  NVE_CHECK(meta.data[3] == 1);
  auto* nodes_buf4 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle));
  auto* text_buf4 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(&env, nullptr, handle));
  auto* dirty_buf4 = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(&env, nullptr, handle));
  nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf4->address);
  text_slab = reinterpret_cast<const char*>(text_buf4->address);
  const uint32_t scroll_dirty_idx = reinterpret_cast<const uint32_t*>(dirty_buf4->address)[0];
  NVE_CHECK(nodes[scroll_dirty_idx].node_id == 2);
  NVE_CHECK(
      std::string_view(
          text_slab + nodes[scroll_dirty_idx].text_offset, nodes[scroll_dirty_idx].text_len) ==
      "Scroll: 18,142");

  // Custom "submit" event on TextInput node 7 -> mutates accessibility & TextInput raw props ONLY
  // on nodes 3 & 7 (NOT unchanged nodes 5 & 8!) with ZERO DirectByteBuffer reallocations for metadata!
  const int allocs_before_submit = g_direct_buffer_allocations;
  FakeJString submit_name{"submit"};
  FakeJString submit_payload{"hunter2"};
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
          &env,
          nullptr,
          handle,
          7,
          reinterpret_cast<jstring>(&submit_name),
          reinterpret_cast<jstring>(&submit_payload)) == 1);
  NVE_CHECK(g_active_utf_borrows == 0);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
          &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta)) == 7);
  const jint mask_submit =
      Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(&env, nullptr, handle);
  NVE_CHECK((mask_submit & (1 << 5)) == 0); // high-water-mark metadata GlobalRef reused!
  const auto meta_submit = parse_metadata_slab(
      meta_buf1, static_cast<uint64_t>(meta.data[1]), static_cast<uint32_t>(meta.data[7]));
  NVE_CHECK(meta_submit.size() == 2u); // ONLY dirty nodes 3 & 7!
  NVE_CHECK(meta_submit.count(5) == 0 && meta_submit.count(8) == 0);
  NVE_CHECK((meta_submit.at(3).flags & (1u << 0)) != 0u); // disabled == true
  NVE_CHECK(meta_submit.at(3).label == "Counter locked");
  NVE_CHECK(meta_submit.at(7).placeholder == "Submitted");
  NVE_CHECK((meta_submit.at(7).flags & (1u << 1)) == 0u); // secureTextEntry == false
  // Only text_slab grew (+1); zero metadata or per-prop JNI DirectByteBuffer allocations!
  NVE_CHECK(g_direct_buffer_allocations <= allocs_before_submit + 2);

  // Custom "keydown" event ("Cmd+k") -> corrupt NaN NVE_DRAW1 on node 8 is safely caught:
  // FLAG_DRAW_LIST_CORRUPT set, draw_list_len == 0, no crash, only node 8 emitted!
  FakeJString keydown_name{"keydown"};
  FakeJString cmd_k{"Cmd+k"};
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
          &env,
          nullptr,
          handle,
          1,
          reinterpret_cast<jstring>(&keydown_name),
          reinterpret_cast<jstring>(&cmd_k)) == 1);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
          &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta)) == 7);
  const auto meta_corrupt = parse_metadata_slab(
      meta_buf1, static_cast<uint64_t>(meta.data[1]), static_cast<uint32_t>(meta.data[7]));
  NVE_CHECK(meta_corrupt.size() == 1u);
  NVE_CHECK(meta_corrupt.count(8) == 1u);
  NVE_CHECK((meta_corrupt.at(8).flags & (1u << 3)) == 0u); // FLAG_HAS_DRAW_LIST cleared
  NVE_CHECK((meta_corrupt.at(8).flags & (1u << 4)) != 0u); // FLAG_DRAW_LIST_CORRUPT set
  NVE_CHECK(meta_corrupt.at(8).draw_list.empty());

  // Custom "keydown" event ("Escape") -> valid `nve-draw1;b64,...` base64 recovery on node 8!
  // FabricHost decodes `nve-draw1;b64,...` directly into `NveSceneView::text_slab` (zero duplication).
  FakeJString esc_key{"Escape"};
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
          &env,
          nullptr,
          handle,
          1,
          reinterpret_cast<jstring>(&keydown_name),
          reinterpret_cast<jstring>(&esc_key)) == 1);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
          &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta)) == 7);
  auto* nodes_buf_rec = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle));
  auto* text_buf_rec = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(&env, nullptr, handle));
  const auto* rec_nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf_rec->address);
  const auto* rec_text = reinterpret_cast<const uint8_t*>(text_buf_rec->address);
  bool saw_recovered_canvas = false;
  for (int i = 0; i < 7; ++i) {
    if (rec_nodes[i].node_id == 8) {
      saw_recovered_canvas = true;
      NVE_CHECK(rec_nodes[i].kind == NVE_NODE_KIND_SKIA_CANVAS);
      NVE_CHECK(rec_nodes[i].text_len > 15u);
      const uint8_t* dl = rec_text + rec_nodes[i].text_offset;
      NVE_CHECK(std::memcmp(dl, "NVE_DRAW1", 9) == 0);
      NVE_CHECK(read_u32_le(dl + 11) == 8u);
    }
  }
  NVE_CHECK(saw_recovered_canvas);
  const auto meta_recovered = parse_metadata_slab(
      meta_buf1, static_cast<uint64_t>(meta.data[1]), static_cast<uint32_t>(meta.data[7]));
  // Emits a default reset entry to clear the prior frame's FLAG_DRAW_LIST_CORRUPT with zero drawList duplication!
  NVE_CHECK(meta_recovered.size() == 1u);
  NVE_CHECK(meta_recovered.count(8) == 1u);
  NVE_CHECK((meta_recovered.at(8).flags & (1u << 4)) == 0u); // FLAG_DRAW_LIST_CORRUPT cleared
  NVE_CHECK(meta_recovered.at(8).draw_list.empty());         // zero-copy via text_slab!

  // Destroying surface MUST release all 6 GlobalRefs with zero leaks!
  Java_dev_nve_android_NveMainActivity_nativeDestroySurface(&env, nullptr, handle);
  NVE_CHECK(g_live_global_refs.empty());
  cleanup_fake_buffers();
}

void test_surface_hotswap_and_atomic_rollback(
    const std::string& v1_path,
    const std::string& v2_path,
    const std::string& bad_iface_path,
    const std::string& no_symbol_path) {
  JNINativeInterface_ table{};
  JNIEnv_ env = make_fake_env(table);
  nve_jni_test_log_reset();

  FakeJString missing_str{"/nonexistent/libnve_missing.so"};
  FakeJString no_sym_str{no_symbol_path};
  FakeJString bad_iface_str{bad_iface_path};
  FakeJString v1_str{v1_path};
  FakeJString v2_str{v2_path};

  // Null handle fails cleanly.
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, 0, reinterpret_cast<jstring>(&missing_str)) == JNI_FALSE);
  NVE_CHECK(g_active_utf_borrows == 0);

  const jlong handle = Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
      &env, nullptr, 77, 390.0f, 844.0f, 3.0f);
  NVE_CHECK(handle != 0);

  // Invalid loads before v1 return JNI_FALSE.
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&missing_str)) == JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&no_sym_str)) == JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&bad_iface_str)) == JNI_FALSE);
  NVE_CHECK(g_active_utf_borrows == 0);

  // Load v1 -> creates surface 77 and renders frame 1.
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&v1_str)) == JNI_TRUE);
  const auto* log = nve_jni_test_log();
  NVE_CHECK(log->v1_create_count == 1);
  NVE_CHECK(log->v1_surface_id == 77);
  NVE_CHECK(log->v1_last_frame_id == 1);
  NVE_CHECK(log->v1_destroy_count == 0);

  FakeLongArray meta{std::vector<jlong>(8, 0)};
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
          &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta)) == 1);
  NVE_CHECK(g_live_global_refs.size() == 6u);

  // Failed hot-reloads while v1 is live MUST NOT tear down v1 or its slab buffers.
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&missing_str)) == JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&no_sym_str)) == JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&bad_iface_str)) == JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(&env, nullptr, handle, nullptr) ==
      JNI_FALSE);
  NVE_CHECK(g_active_utf_borrows == 0);
  NVE_CHECK(log->v1_destroy_count == 0);
  NVE_CHECK(log->v1_snapshot_count == 0);

  // Hot-swap v1 -> v2 snapshots state, destroys v1, creates v2, restores state, resets cached slabs.
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&v2_str)) == JNI_TRUE);
  NVE_CHECK(g_live_global_refs.empty()); // ABA-safe invalidation after reload!
  NVE_CHECK(log->v1_snapshot_count == 1);
  NVE_CHECK(log->v1_destroy_count == 1);
  NVE_CHECK(log->v2_create_count == 1);
  NVE_CHECK(log->v2_surface_id == 77);
  NVE_CHECK(log->v2_restore_count == 1);
  NVE_CHECK(std::string(log->v2_restored_state, log->v2_restored_len) == "v1-counter=42");

  // Re-render v2 -> fresh GlobalRefs allocated (mask == 0x3F).
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
          &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta)) == 1);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(&env, nullptr, handle) == 0x3F);
  NVE_CHECK(g_live_global_refs.size() == 6u);

  // Clean destroy frees v2 and all GlobalRefs.
  Java_dev_nve_android_NveMainActivity_nativeDestroySurface(&env, nullptr, 0);
  Java_dev_nve_android_NveMainActivity_nativeDestroySurface(&env, nullptr, handle);
  NVE_CHECK(log->v2_destroy_count == 1);
  NVE_CHECK(g_live_global_refs.empty());
  cleanup_fake_buffers();
}

} // namespace

int main(int argc, char** argv) {
  NVE_CHECK(argc == 6);
  test_direct_scene_view_cached_buffers_metadata_draw1_and_events(argv[1]);
  test_surface_hotswap_and_atomic_rollback(argv[2], argv[3], argv[4], argv[5]);
  std::cout << "nve_android_scene_test passed\n";
  return 0;
}
