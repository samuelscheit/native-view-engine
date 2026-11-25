#include "nve_scene_builder.hpp"

#include <jni.h>

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <string_view>
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

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jlongArray out_meta);

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
};

std::vector<FakeDirectBuffer*> g_direct_buffers;

const char* JNICALL fake_get_string_utf_chars(JNIEnv*, jstring str, jboolean* is_copy) {
  if (is_copy) *is_copy = JNI_FALSE;
  return reinterpret_cast<const FakeJString*>(str)->value.c_str();
}

void JNICALL fake_release_string_utf_chars(JNIEnv*, jstring, const char*) {}

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
  auto* buf = new FakeDirectBuffer{address, capacity};
  g_direct_buffers.push_back(buf);
  return reinterpret_cast<jobject>(buf);
}

void* JNICALL fake_get_direct_buffer_address(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->address;
}

jlong JNICALL fake_get_direct_buffer_capacity(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->capacity;
}

void test_scene_table_builder_incremental_and_compaction() {
  nve::SceneTableBuilder builder;

  std::vector<nve::RenderedNodeSnapshot> frame1;
  frame1.reserve(64);
  for (uint32_t id = 1; id <= 64; ++id) {
    nve::RenderedNodeSnapshot s{};
    s.node_id = id;
    s.kind = id == 1 ? NVE_COMP_ROOT : (id % 2 == 0 ? NVE_COMP_TEXT : NVE_COMP_PRESSABLE);
    s.text = id % 2 == 0 ? ("item-" + std::to_string(id)) : "";
    s.layout = {static_cast<float>(id), static_cast<float>(id * 2), 100.0f, 32.0f};
    s.opacity = 0.5f;
    s.font_size = 15.0f;
    if (id == 1) {
      s.children = {2, 3, 4};
    }
    frame1.push_back(std::move(s));
  }

  builder.sync(1, frame1);
  NveSceneView v1 = builder.view();
  NVE_CHECK(nve::SceneTableBuilder::verify_view_schema(v1));
  NVE_CHECK(v1.node_count == 64);
  NVE_CHECK(v1.dirty_count == 64);
  NVE_CHECK(v1.removed_count == 0);

  // Frame 2: mutate only 2 nodes -> dirty_count MUST be exactly 2.
  frame1[9].text = "updated-10 🚀";
  frame1[41].layout.x = 777.0f;
  builder.sync(2, frame1);
  NveSceneView v2 = builder.view();
  NVE_CHECK(v2.frame_id == 2);
  NVE_CHECK(v2.node_count == 64);
  NVE_CHECK(v2.dirty_count == 2);
  NVE_CHECK(v2.removed_count == 0);
  NVE_CHECK(builder.text_for(v2.nodes[v2.dirty_indices[0]]) == "updated-10 🚀");
  NVE_CHECK(v2.nodes[v2.dirty_indices[1]].x == 777.0f);

  // Frame 3: remove 3 nodes -> removed_count == 3.
  frame1.erase(frame1.begin() + 61, frame1.end());
  builder.sync(3, frame1);
  NveSceneView v3 = builder.view();
  NVE_CHECK(v3.node_count == 61);
  NVE_CHECK(v3.removed_count == 3);

  builder.compact_slabs();
  NveSceneView v3c = builder.view();
  bool found_updated = false;
  for (uint32_t i = 0; i < v3c.node_count; ++i) {
    if (v3c.nodes[i].node_id == 10) {
      NVE_CHECK(builder.text_for(v3c.nodes[i]) == "updated-10 🚀");
      found_updated = true;
    }
  }
  NVE_CHECK(found_updated);
}

void test_android_surface_jni_direct_byte_buffers(const std::string& mock_scene_so) {
  JNINativeInterface_ table{};
  table.GetStringUTFChars = &fake_get_string_utf_chars;
  table.ReleaseStringUTFChars = &fake_release_string_utf_chars;
  table.GetArrayLength = &fake_get_array_length;
  table.SetLongArrayRegion = &fake_set_long_array_region;
  table.NewDirectByteBuffer = &fake_new_direct_byte_buffer;
  table.GetDirectBufferAddress = &fake_get_direct_buffer_address;
  table.GetDirectBufferCapacity = &fake_get_direct_buffer_capacity;

  JNIEnv_ env{};
  env.functions = &table;

  const jlong handle = Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
      &env, nullptr, 1, 412.0f, 915.0f, 2.0f);
  NVE_CHECK(handle != 0);

  FakeJString so_str{mock_scene_so};
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&so_str)) == JNI_TRUE);

  // Frame 1: initial 4 nodes (root=1, text=2, pressable=3, banner=4).
  FakeLongArray meta{std::vector<jlong>(7, 0)};
  const jint count1 = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(count1 == 4);
  NVE_CHECK(static_cast<uint64_t>(meta.data[0]) == NVE_SCENE_SCHEMA_HASH);
  NVE_CHECK(meta.data[1] == 1); // frame_id
  NVE_CHECK(meta.data[2] == 4); // node_count
  NVE_CHECK(meta.data[3] == 4); // dirty_count
  NVE_CHECK(meta.data[4] == 0); // removed_count

  auto* nodes_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle));
  auto* text_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(&env, nullptr, handle));
  auto* child_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetChildrenSlabBuffer(&env, nullptr, handle));
  auto* dirty_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(&env, nullptr, handle));

  NVE_CHECK(nodes_buf->capacity == 4 * static_cast<jlong>(sizeof(NveRenderNode)));
  NVE_CHECK(dirty_buf->capacity == 4 * static_cast<jlong>(sizeof(uint32_t)));
  NVE_CHECK(child_buf->capacity == 3 * static_cast<jlong>(sizeof(uint32_t)));

  const auto* nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf->address);
  const auto* text_slab = reinterpret_cast<const char*>(text_buf->address);
  const auto* children_slab = reinterpret_cast<const uint32_t*>(child_buf->address);

  bool saw_root = false;
  bool saw_label = false;
  bool saw_pressable = false;
  for (int i = 0; i < 4; ++i) {
    if (nodes[i].node_id == 1) {
      saw_root = true;
      NVE_CHECK(nodes[i].child_count == 3);
      NVE_CHECK(children_slab[nodes[i].child_offset + 0] == 2);
      NVE_CHECK(children_slab[nodes[i].child_offset + 1] == 3);
      NVE_CHECK(children_slab[nodes[i].child_offset + 2] == 4);
    } else if (nodes[i].node_id == 2) {
      saw_label = true;
      NVE_CHECK((nodes[i].flags & NVE_NODE_LEAF_TEXT) != 0);
      const std::string_view txt(text_slab + nodes[i].text_offset, nodes[i].text_len);
      NVE_CHECK(txt == "Count: 0 🚀");
    } else if (nodes[i].node_id == 3) {
      saw_pressable = true;
      NVE_CHECK((nodes[i].flags & NVE_NODE_INTERACTIVE) != 0);
    }
  }
  NVE_CHECK(saw_root && saw_label && saw_pressable);

  // Press 1 -> updates ONLY node 2 text ("Count: 1 🚀"), so dirty_count == 1!
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeDispatchPress(&env, nullptr, handle, 3, 10.0f, 10.0f) ==
      1);
  const jint count2 = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(count2 == 4);
  NVE_CHECK(meta.data[3] == 1); // dirty_count == 1 (O(changed)!)
  NVE_CHECK(meta.data[4] == 0); // removed_count == 0

  nodes_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle));
  text_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(&env, nullptr, handle));
  dirty_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(&env, nullptr, handle));
  nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf->address);
  text_slab = reinterpret_cast<const char*>(text_buf->address);
  const uint32_t dirty_idx = reinterpret_cast<const uint32_t*>(dirty_buf->address)[0];
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
  NVE_CHECK(count3 == 3);
  NVE_CHECK(meta.data[4] == 1); // removed_count == 1
  auto* removed_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetRemovedIdsBuffer(&env, nullptr, handle));
  NVE_CHECK(removed_buf->capacity == static_cast<jlong>(sizeof(uint32_t)));
  NVE_CHECK(reinterpret_cast<const uint32_t*>(removed_buf->address)[0] == 4);

  Java_dev_nve_android_NveMainActivity_nativeDestroySurface(&env, nullptr, handle);
  for (auto* b : g_direct_buffers) {
    delete b;
  }
  g_direct_buffers.clear();
}

} // namespace

int main(int argc, char** argv) {
  NVE_CHECK(argc == 2);
  test_scene_table_builder_incremental_and_compaction();
  test_android_surface_jni_direct_byte_buffers(argv[1]);
  std::cout << "nve_android_scene_test passed\n";
  return 0;
}
