#include "android_jni_test_env.hpp"
#include "command_encoder.hpp"
#include "nve_host_runtime.hpp"
#include "nve_pair_client.hpp"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include <atomic>
#include <chrono>
#include <cstdarg>
#include <cstring>
#include <iostream>
#include <mutex>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

namespace {

using namespace nve::android_test;

bool jni_pair_send_all(int fd, const uint8_t* p, size_t n) {
  size_t off = 0u;
  while (off < n) {
    const ssize_t rc = ::send(fd, p + off, n - off, 0);
    if (rc <= 0) return false;
    off += static_cast<size_t>(rc);
  }
  return true;
}

bool jni_pair_recv_exact(int fd, uint8_t* p, size_t n, int timeout_ms = 2000) {
  size_t off = 0u;
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeout_ms);
  while (off < n) {
    const auto now = std::chrono::steady_clock::now();
    if (now >= deadline) return false;
    pollfd pfd{fd, POLLIN, 0};
    const int rem = static_cast<int>(
        std::chrono::duration_cast<std::chrono::milliseconds>(deadline - now).count());
    if (::poll(&pfd, 1u, std::max(1, std::min(rem, 50))) <= 0) continue;
    const ssize_t rc = ::recv(fd, p + off, n - off, 0);
    if (rc <= 0) return false;
    off += static_cast<size_t>(rc);
  }
  return true;
}

bool jni_pair_write_tcp(int fd, std::span<const uint8_t> payload) {
  return jni_pair_send_all(fd, payload.data(), payload.size());
}

bool jni_pair_read_frame(
    int fd, std::vector<uint8_t>& buf, nve::WireFrameView& out, int timeout_ms = 2000) {
  uint8_t hdr_bytes[nve::kWireHeaderSize]{};
  if (!jni_pair_recv_exact(fd, hdr_bytes, nve::kWireHeaderSize, timeout_ms)) return false;
  nve::WireHeader hdr{};
  if (!nve::wire_v2::decode_header(hdr_bytes, hdr)) return false;
  buf.resize(hdr.total_wire_len());
  std::memcpy(buf.data(), hdr_bytes, nve::kWireHeaderSize);
  if (buf.size() > nve::kWireHeaderSize &&
      !jni_pair_recv_exact(
          fd, buf.data() + nve::kWireHeaderSize, buf.size() - nve::kWireHeaderSize, timeout_ms)) {
    return false;
  }
  return nve::wire_v2::decode_frame(buf, out);
}

std::vector<uint8_t> build_android_pair_frame(uint64_t frame_id, const std::string& label_text) {
  nve::CommandBufferEncoder enc(1u, frame_id);
  enc.create_node(2u, NVE_COMP_VIEW);
  enc.create_node(3u, NVE_COMP_TEXT);
  enc.create_node(4u, NVE_COMP_PRESSABLE);
  enc.create_node(5u, NVE_COMP_TEXT_INPUT);
  enc.set_children(NVE_ROOT_NODE_ID, {2u});
  enc.set_children(2u, {3u, 4u, 5u});

  nve::LayoutProps box{};
  box.width = {1u, 360.0f};
  box.height = {1u, 400.0f};
  enc.set_layout(2u, box);

  nve::LayoutProps btn{};
  btn.width = {1u, 140.0f};
  btn.height = {1u, 44.0f};
  enc.set_layout(4u, btn);
  enc.set_layout(5u, btn);

  enc.set_text(3u, label_text);
  enc.set_text(5u, "initial");
  enc.set_event_mask(4u, nve::kEventMaskPress);
  enc.set_event_mask(5u, 1ull << 3u);
  enc.commit(true);
  const NveBufferSlice slice = enc.finish();
  return std::vector<uint8_t>(slice.ptr, slice.ptr + slice.len);
}

struct FakePairVmState {
  JavaVM_ jvm{};
  JNIInvokeInterface_ invoke{};
  JNIEnv_ callback_env{};
  JNINativeInterface_ callback_table{};
  FakeDirectBuffer fake_class_obj{nullptr, 0, 1};
  FakeDirectBuffer fake_wake_obj{nullptr, 0, 1};
  std::atomic<uint32_t> attach_count{0u};
  std::atomic<uint32_t> detach_count{0u};
  std::atomic<uint32_t> wake_calls{0u};
  std::string last_utf_error;
};

FakePairVmState* g_pair_vm = nullptr;

jint JNICALL fake_vm_get_env(JavaVM*, void** penv, jint) {
  *penv = nullptr;
  return JNI_EDETACHED;
}

jint JNICALL fake_vm_attach(JavaVM*, void** penv, void*) {
  g_pair_vm->attach_count.fetch_add(1u, std::memory_order_relaxed);
  *penv = &g_pair_vm->callback_env;
  return JNI_OK;
}

jint JNICALL fake_vm_detach(JavaVM*) {
  g_pair_vm->detach_count.fetch_add(1u, std::memory_order_relaxed);
  return JNI_OK;
}

jint JNICALL fake_env_get_java_vm(JNIEnv*, JavaVM** vm) {
  *vm = &g_pair_vm->jvm;
  return JNI_OK;
}

jclass JNICALL fake_env_get_object_class(JNIEnv*, jobject) {
  g_pair_vm->fake_class_obj.ref_count += 1;
  return reinterpret_cast<jclass>(&g_pair_vm->fake_class_obj);
}

jmethodID JNICALL fake_env_get_method_id(JNIEnv*, jclass, const char* name, const char* sig) {
  if (std::strcmp(name, "onNativePairFrameReady") == 0 && std::strcmp(sig, "()V") == 0) {
    return reinterpret_cast<jmethodID>(static_cast<uintptr_t>(0x1234));
  }
  return nullptr;
}

jboolean JNICALL fake_env_exception_check(JNIEnv*) { return JNI_FALSE; }
void JNICALL fake_env_exception_clear(JNIEnv*) {}

void JNICALL fake_env_call_void_method_v(JNIEnv*, jobject obj, jmethodID mid, va_list) {
  NVE_CHECK(obj == reinterpret_cast<jobject>(&g_pair_vm->fake_wake_obj));
  NVE_CHECK(mid == reinterpret_cast<jmethodID>(static_cast<uintptr_t>(0x1234)));
  g_pair_vm->wake_calls.fetch_add(1u, std::memory_order_relaxed);
}

void JNICALL fake_env_call_void_method(JNIEnv* env, jobject obj, jmethodID mid, ...) {
  va_list ap;
  va_start(ap, mid);
  fake_env_call_void_method_v(env, obj, mid, ap);
  va_end(ap);
}

jstring JNICALL fake_env_new_string_utf(JNIEnv*, const char* utf) {
  g_pair_vm->last_utf_error = utf != nullptr ? utf : "";
  static FakeJString last_err_box;
  last_err_box.value = g_pair_vm->last_utf_error;
  return reinterpret_cast<jstring>(&last_err_box);
}

void test_paired_thin_client_jni_loopback() {
  FakePairVmState vm_state{};
  g_pair_vm = &vm_state;
  vm_state.invoke.GetEnv = &fake_vm_get_env;
  vm_state.invoke.AttachCurrentThread = &fake_vm_attach;
  vm_state.invoke.DetachCurrentThread = &fake_vm_detach;
  vm_state.jvm.functions = &vm_state.invoke;

  JNINativeInterface_ table{};
  JNIEnv_ env = make_fake_env(table);
  table.GetJavaVM = &fake_env_get_java_vm;
  table.GetObjectClass = &fake_env_get_object_class;
  table.GetMethodID = &fake_env_get_method_id;
  table.ExceptionCheck = &fake_env_exception_check;
  table.ExceptionClear = &fake_env_exception_clear;
  table.CallVoidMethod = &fake_env_call_void_method;
  table.CallVoidMethodV = &fake_env_call_void_method_v;
  table.NewStringUTF = &fake_env_new_string_utf;

  vm_state.callback_table = table;
  vm_state.callback_env.functions = &vm_state.callback_table;

  const std::string token = "nve-android-pair-token";
  const auto frame1 = build_android_pair_frame(1u, "Android Paired 1");
  const auto frame2 = build_android_pair_frame(2u, "Android Paired 2");

  int listen_fd = ::socket(AF_INET, SOCK_STREAM, 0);
  NVE_CHECK(listen_fd >= 0);
  const int reuse = 1;
  (void)::setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  NVE_CHECK(::bind(listen_fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
  socklen_t alen = sizeof(addr);
  NVE_CHECK(::getsockname(listen_fd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
  const uint16_t port = ntohs(addr.sin_port);
  NVE_CHECK(::listen(listen_fd, 4) == 0);

  std::vector<nve::DecodedEvent> rx_events;
  std::mutex rx_mtx;

  std::thread srv([&]() {
    // 1. Reject wrong token cleanly.
    {
      int cfd = ::accept(listen_fd, nullptr, nullptr);
      NVE_CHECK(cfd >= 0);
      std::vector<uint8_t> msg;
      nve::WireFrameView v{};
      NVE_CHECK(jni_pair_read_frame(cfd, msg, v) && v.header.kind == nve::WireKind::Hello);
      const auto rej = nve::wire_v2::encode_frame(
          nve::WireChannel::Control,
          nve::WireKind::Reject,
          1u,
          nve::wire_v2::encode_reject(nve::WireRejectReason::Unauthorized, "bad bearer token"));
      NVE_CHECK(jni_pair_write_tcp(cfd, rej));
      ::close(cfd);
    }

    // 2. Accept valid token, send CommandDelta frames, collect 3 upstream EventBatch events.
    {
      int cfd = ::accept(listen_fd, nullptr, nullptr);
      NVE_CHECK(cfd >= 0);
      std::vector<uint8_t> msg;
      nve::WireFrameView v{};
      NVE_CHECK(jni_pair_read_frame(cfd, msg, v) && v.header.kind == nve::WireKind::Hello);
      nve::WireHello hp{};
      NVE_CHECK(nve::wire_v2::decode_hello(v.payload, hp));
      NVE_CHECK(nve::wire_v2::constant_time_eq(hp.bearer_token, token));

      const auto ack_b = nve::wire_v2::encode_hello_ack({77u, 2u, 0x0Fu});
      NVE_CHECK(jni_pair_write_tcp(
          cfd,
          nve::wire_v2::encode_frame(
              nve::WireChannel::Control, nve::WireKind::HelloAck, 1u, ack_b)));

      NVE_CHECK(jni_pair_write_tcp(
          cfd,
          nve::wire_v2::encode_frame(
              nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 1u, frame1)));
      NVE_CHECK(jni_pair_write_tcp(
          cfd,
          nve::wire_v2::encode_frame(
              nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 2u, frame2)));

      while (jni_pair_read_frame(cfd, msg, v, 3000)) {
        if (v.header.kind == nve::WireKind::EventBatch) {
          std::vector<nve::DecodedEvent> batch;
          if (nve::decode_event_ring(NveBufferSlice{v.payload.data(), v.payload.size()}, batch)) {
            std::lock_guard<std::mutex> lock(rx_mtx);
            rx_events.insert(rx_events.end(), batch.begin(), batch.end());
            if (rx_events.size() >= 3u) break;
          }
        } else if (v.header.kind == nve::WireKind::Bye) {
          break;
        }
      }
      ::close(cfd);
    }
    ::close(listen_fd);
  });

  const jlong handle = Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
      &env, nullptr, 1, 390.0f, 844.0f, 3.0f);
  NVE_CHECK(handle != 0);
  NVE_CHECK(Java_dev_nve_android_NvePairController_nativeIsPaired(&env, nullptr, handle) ==
            JNI_FALSE);

  FakeJString endpoint_str{"127.0.0.1:" + std::to_string(port)};
  FakeJString bad_token_str{"bad-token"};
  FakeJString good_token_str{token};
  FakeJString auto_tr_str{"auto"};
  jobject wake_obj = reinterpret_cast<jobject>(&vm_state.fake_wake_obj);

  NVE_CHECK(Java_dev_nve_android_NvePairController_nativeConnectPaired(
                &env, nullptr, handle,
                reinterpret_cast<jstring>(&endpoint_str),
                reinterpret_cast<jstring>(&bad_token_str),
                reinterpret_cast<jstring>(&auto_tr_str),
                wake_obj) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NvePairController_nativeIsPaired(&env, nullptr, handle) ==
            JNI_FALSE);
  (void)Java_dev_nve_android_NveMainActivity_nativeGetLastError(&env, nullptr, handle);
  NVE_CHECK(vm_state.last_utf_error.find("token") != std::string::npos);
  NVE_CHECK(g_live_global_refs.empty());

  NVE_CHECK(Java_dev_nve_android_NvePairController_nativeConnectPaired(
                &env, nullptr, handle,
                reinterpret_cast<jstring>(&endpoint_str),
                reinterpret_cast<jstring>(&good_token_str),
                reinterpret_cast<jstring>(&auto_tr_str),
                wake_obj) == JNI_TRUE);
  NVE_CHECK(Java_dev_nve_android_NvePairController_nativeIsPaired(&env, nullptr, handle) ==
            JNI_TRUE);

  int total_polled = 0;
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (total_polled < 2 && std::chrono::steady_clock::now() < deadline) {
    total_polled +=
        Java_dev_nve_android_NvePairController_nativePollPairedFrames(&env, nullptr, handle);
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(total_polled >= 1);
  NVE_CHECK(vm_state.wake_calls.load() >= 1u);
  NVE_CHECK(vm_state.attach_count.load() == vm_state.detach_count.load());

  FakeLongArray meta{std::vector<jlong>(9, 0)};
  const jint node_count = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      &env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(node_count == 5);
  auto* nodes_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(&env, nullptr, handle));
  auto* text_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(&env, nullptr, handle));
  const auto* nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf->address);
  const auto* text_bytes = reinterpret_cast<const char*>(text_buf->address);
  bool saw_label = false;
  for (int i = 0; i < node_count; ++i) {
    if (nodes[i].node_id == 3u) {
      const std::string_view label(text_bytes + nodes[i].text_offset, nodes[i].text_len);
      NVE_CHECK(label == "Android Paired 2");
      saw_label = true;
    }
  }
  NVE_CHECK(saw_label);

  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeDispatchPress(
                &env, nullptr, handle, 4, 15.0f, 20.0f) == 1);
  FakeJString typed{"hello-pair"};
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeDispatchTextChange(
                &env, nullptr, handle, 5, reinterpret_cast<jstring>(&typed)) == 1);
  FakeJString ev_name{"keydown"};
  FakeJString ev_payload{"Enter"};
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
                &env, nullptr, handle, 1,
                reinterpret_cast<jstring>(&ev_name),
                reinterpret_cast<jstring>(&ev_payload)) == 1);

  const auto ev_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (std::chrono::steady_clock::now() < ev_deadline) {
    {
      std::lock_guard<std::mutex> lock(rx_mtx);
      if (rx_events.size() >= 3u) break;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  {
    std::lock_guard<std::mutex> lock(rx_mtx);
    NVE_CHECK(rx_events.size() == 3u);
    NVE_CHECK(rx_events[0].kind == nve::EventKind::Press && rx_events[0].node_id == 4u);
    NVE_CHECK(rx_events[1].kind == nve::EventKind::TextChange && rx_events[1].node_id == 5u &&
              rx_events[1].text == "hello-pair");
    NVE_CHECK(rx_events[2].kind == nve::EventKind::Custom && rx_events[2].name == "keydown" &&
              rx_events[2].payload_as_string() == "Enter");
  }

  Java_dev_nve_android_NvePairController_nativeDisconnectPaired(&env, nullptr, handle);
  NVE_CHECK(Java_dev_nve_android_NvePairController_nativeIsPaired(&env, nullptr, handle) ==
            JNI_FALSE);
  Java_dev_nve_android_NveMainActivity_nativeDestroySurface(&env, nullptr, handle);
  NVE_CHECK(g_live_global_refs.empty());
  NVE_CHECK(g_active_utf_borrows == 0);
  srv.join();
  cleanup_fake_buffers();
  g_pair_vm = nullptr;
}

} // namespace

int main() {
  test_paired_thin_client_jni_loopback();
  std::cout << "nve_android_pair_test passed\n";
  return 0;
}
