#include "nve_nitro_core.hpp"

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

#define NVE_CHECK(expr)                                                               \
  do {                                                                                \
    if (!(expr)) {                                                                    \
      std::fprintf(stderr, "NVE_CHECK failed at %s:%d: %s\n", __FILE__, __LINE__,     \
                   #expr);                                                            \
      std::abort();                                                                   \
    }                                                                                 \
  } while (false)

using namespace nve::nitro;

namespace {

class MockCounterHybrid final : public HybridObject {
 public:
  explicit MockCounterHybrid(int64_t initial = 0) : HybridObject("MockCounter"), value_(initial) {}

  size_t memory_size() const noexcept override { return sizeof(*this) + payload_.size(); }

  void dispose() override {
    if (!is_disposed()) {
      ++dispose_calls_;
      payload_.clear();
      HybridObject::dispose();
    }
  }

  int64_t value() const noexcept { return value_; }
  int dispose_calls() const noexcept { return dispose_calls_; }

  NitroVariant invoke_method(std::string_view method, std::span<const NitroVariant> args) override {
    if (is_disposed()) return NitroVariant::from_error("DISPOSED");
    if (method == "increment") {
      int64_t delta = 1;
      if (!args.empty()) {
        if (auto d = args[0].as_i64()) delta = *d;
        else if (auto ab = args[0].as_array_buffer(); ab && ab->size() == 1u) delta = ab->data()[0];
      }
      value_ += delta;
      return NitroVariant::from_i64(value_);
    }
    if (method == "echoBytes") {
      if (args.empty()) return NitroVariant::from_bytes(ArrayBuffer::allocate(0u));
      if (auto ab = args[0].as_array_buffer()) {
        return NitroVariant::from_bytes(ArrayBuffer::copy(ab->span()));
      }
      return NitroVariant::from_error("EXPECTED_BYTES");
    }
    if (method == "tag") {
      return NitroVariant::from_string("counter:" + std::to_string(value_));
    }
    if (method == "fail") {
      return NitroVariant::from_error("EXPLICIT_FAILURE");
    }
    return HybridObject::invoke_method(method, args);
  }

  NitroVariant get_property(std::string_view prop) const override {
    if (prop == "value") return NitroVariant::from_i64(value_);
    return HybridObject::get_property(prop);
  }

  bool set_property(std::string_view prop, const NitroVariant& val) override {
    if (prop == "value") {
      if (auto n = val.as_i64()) {
        value_ = *n;
        return true;
      }
    }
    return HybridObject::set_property(prop, val);
  }

 private:
  int64_t value_{0};
  int dispose_calls_{0};
  std::vector<uint8_t> payload_{1, 2, 3, 4};
};

class OtherHybrid final : public HybridObject {
 public:
  OtherHybrid() : HybridObject("OtherHybrid") {}
};

void test_array_buffer_zero_copy_and_owning() {
  // Owning allocate
  auto owned = ArrayBuffer::allocate(16u);
  NVE_CHECK(owned->size() == 16u);
  NVE_CHECK(owned->is_owning());
  NVE_CHECK(!owned->empty());
  for (uint8_t b : owned->span()) NVE_CHECK(b == 0u);
  owned->data()[0] = 0xABu;
  owned->data()[15] = 0xCDu;
  const auto copied = owned->copy_bytes();
  NVE_CHECK(copied.size() == 16u && copied[0] == 0xABu && copied[15] == 0xCDu);

  // Owning copy & from_vector
  const std::vector<uint8_t> src = {10u, 20u, 30u, 40u};
  auto dup = ArrayBuffer::copy(src);
  NVE_CHECK(dup->is_owning() && dup->size() == 4u && dup->data() != src.data());
  NVE_CHECK(dup->copy_bytes() == src);

  std::vector<uint8_t> moved_vec = {5u, 6u, 7u};
  const uint8_t* orig_ptr = moved_vec.data();
  auto from_vec = ArrayBuffer::from_vector(std::move(moved_vec));
  NVE_CHECK(from_vec->is_owning() && from_vec->data() == orig_ptr && from_vec->size() == 3u);

  // Borrowed zero-copy wrap (pointer identity preserved, is_owning == false)
  uint8_t stack_buf[4] = {1u, 2u, 3u, 4u};
  auto borrowed = ArrayBuffer::wrap(stack_buf, sizeof(stack_buf));
  NVE_CHECK(!borrowed->is_owning());
  NVE_CHECK(borrowed->data() == stack_buf);
  NVE_CHECK(borrowed->size() == 4u);
  borrowed->data()[2] = 99u;
  NVE_CHECK(stack_buf[2] == 99u);
  NVE_CHECK(borrowed->copy_bytes() == (std::vector<uint8_t>{1u, 2u, 99u, 4u}));

  // External owned wrap with custom deleter
  bool deleter_fired = false;
  uint8_t* heap_raw = new uint8_t[8]{9u, 8u, 7u, 6u, 5u, 4u, 3u, 2u};
  {
    auto ext_owned = ArrayBuffer::wrap_owned(heap_raw, 8u, [&]() {
      delete[] heap_raw;
      deleter_fired = true;
    });
    NVE_CHECK(ext_owned->is_owning());
    NVE_CHECK(ext_owned->data() == heap_raw);
    NVE_CHECK(!deleter_fired);
  }
  NVE_CHECK(deleter_fired);

  // Empty edge case
  auto empty_buf = ArrayBuffer::wrap(nullptr, 0u);
  NVE_CHECK(empty_buf->empty() && empty_buf->size() == 0u && empty_buf->copy_bytes().empty());
}

void test_promise_resolve_reject_chaining_and_threads() {
  // Immediate resolved + chaining
  auto p1 = Promise<int>::resolved(21);
  NVE_CHECK(p1.is_resolved() && !p1.is_pending() && !p1.is_rejected());
  NVE_CHECK(!p1.resolve(99)); // Idempotent first-wins
  NVE_CHECK(!p1.reject("too late"));
  auto r1 = p1.await_result();
  NVE_CHECK(r1.is_ok() && r1.value() == 21);

  auto p2 = p1.then([](const int& v) { return v * 2; })
                .then([](const int& v) { return std::to_string(v); });
  auto r2 = p2.await_result();
  NVE_CHECK(r2.is_ok() && r2.value() == "42");

  // Async cross-thread resolve + void chaining
  auto async_p = Promise<std::string>::create();
  NVE_CHECK(async_p.is_pending());
  NVE_CHECK(!async_p.await_result_for(std::chrono::milliseconds(5)).has_value());

  std::atomic<int> void_seen{0};
  auto void_chain = async_p.then([&](const std::string& s) {
    if (s == "hello-nitro") void_seen.fetch_add(1);
  });

  std::thread producer([async_p]() mutable {
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
    NVE_CHECK(async_p.resolve("hello-nitro"));
  });

  auto async_res = async_p.await_result();
  NVE_CHECK(async_res.is_ok() && async_res.value() == "hello-nitro");
  auto void_res = void_chain.await_result();
  NVE_CHECK(void_res.is_ok() && void_seen.load() == 1);
  producer.join();

  // Rejection propagation & catch_error
  auto p_err = Promise<int>::create();
  std::string caught_err;
  bool then_ran = false;
  auto chained_err = p_err.then([&](const int& v) {
    then_ran = true;
    return v + 1;
  });
  chained_err.catch_error([&](const std::string& e) { caught_err = e; });

  NVE_CHECK(p_err.reject("CAMERA_UNAVAILABLE"));
  NVE_CHECK(!p_err.reject("SECOND_ERROR"));
  NVE_CHECK(!then_ran);
  NVE_CHECK(caught_err == "CAMERA_UNAVAILABLE");
  auto r_err = chained_err.await_result();
  NVE_CHECK(r_err.is_err() && r_err.error() == "CAMERA_UNAVAILABLE");

  // Already-rejected catch_error fires immediately
  std::string immediate_err;
  Promise<void>::rejected("BOOM").catch_error([&](const std::string& e) { immediate_err = e; });
  NVE_CHECK(immediate_err == "BOOM");

  // Promise<void> resolve + chain
  auto pv = Promise<void>::create();
  auto pv_next = pv.then([]() { return 777; });
  NVE_CHECK(pv.resolve());
  NVE_CHECK(pv_next.await_result().value() == 777);
}

void test_hybrid_object_lifecycle_and_registry() {
  HybridObjectRegistry reg;
  NVE_CHECK(!reg.has_hybrid_object("MockCounter"));
  NVE_CHECK(reg.create<MockCounterHybrid>("MockCounter") == nullptr);

  reg.register_hybrid_object("MockCounter", []() { return std::make_shared<MockCounterHybrid>(10); });
  NVE_CHECK(reg.has_hybrid_object("MockCounter"));

  // Type mismatch returns nullptr safely
  NVE_CHECK(reg.create<OtherHybrid>("MockCounter") == nullptr);

  auto c1 = reg.create<MockCounterHybrid>("MockCounter");
  NVE_CHECK(c1 != nullptr);
  NVE_CHECK(c1->name() == "MockCounter");
  NVE_CHECK(c1->memory_size() >= sizeof(MockCounterHybrid));
  NVE_CHECK(c1->value() == 10);
  NVE_CHECK(c1->get_property("value").as_i64() == 10);
  NVE_CHECK(c1->set_property("value", NitroVariant::from_i64(25)));
  NVE_CHECK(c1->value() == 25);

  const NitroVariant inc_arg = NitroVariant::from_i64(5);
  auto inc_res = c1->invoke_method("increment", std::span<const NitroVariant>(&inc_arg, 1u));
  NVE_CHECK(inc_res.as_i64() == 30);

  // Override registration cleanly replaces factory
  reg.register_hybrid_object("MockCounter", []() { return std::make_shared<MockCounterHybrid>(100); });
  auto c2 = reg.create<MockCounterHybrid>("MockCounter");
  NVE_CHECK(c2 != nullptr && c2->value() == 100);

  // Idempotent dispose
  NVE_CHECK(!c1->is_disposed());
  c1->dispose();
  c1->dispose();
  NVE_CHECK(c1->is_disposed());
  NVE_CHECK(c1->dispose_calls() == 1);
  NVE_CHECK(c1->invoke_method("increment", {}).is_error());

  // Concurrent multi-threaded lookup/create stress test
  std::atomic<int> created{0};
  std::vector<std::thread> workers;
  for (int t = 0; t < 4; ++t) {
    workers.emplace_back([&]() {
      for (int i = 0; i < 100; ++i) {
        auto inst = reg.create<MockCounterHybrid>("MockCounter");
        if (inst && inst->value() == 100) created.fetch_add(1);
      }
    });
  }
  for (auto& w : workers) w.join();
  NVE_CHECK(created.load() == 400);

  NVE_CHECK(reg.unregister_hybrid_object("MockCounter"));
  NVE_CHECK(!reg.unregister_hybrid_object("MockCounter"));
  NVE_CHECK(!reg.has_hybrid_object("MockCounter"));
}

void test_c_abi_handle_bridge() {
  auto& global_reg = HybridObjectRegistry::global();
  global_reg.register_hybrid_object("GlobalCounter", []() { return std::make_shared<MockCounterHybrid>(5); });

  NVE_CHECK(nve_nitro_has_registered("GlobalCounter") == 1);
  NVE_CHECK(nve_nitro_has_registered("DoesNotExist") == 0);
  NVE_CHECK(nve_nitro_create("DoesNotExist") == nullptr);
  NVE_CHECK(nve_nitro_create(nullptr) == nullptr);

  NveNitroHandle h = nve_nitro_create("GlobalCounter");
  NVE_CHECK(h != nullptr);
  NVE_CHECK(std::strcmp(nve_nitro_get_name(h), "MockCounter") == 0);
  NVE_CHECK(nve_nitro_memory_size(h) >= sizeof(MockCounterHybrid));
  NVE_CHECK(nve_nitro_is_disposed(h) == 0);

  // Retain/release refcounting keeps object alive until final release
  nve_nitro_retain(h);
  nve_nitro_release(h);
  NVE_CHECK(nve_nitro_is_disposed(h) == 0);

  // Binary invoke: increment by 7 -> 12 (8-byte little-endian i64)
  const uint8_t delta = 7u;
  uint8_t out_i64[8]{};
  size_t out_len = 0u;
  NVE_CHECK(nve_nitro_invoke_bytes(h, "increment", &delta, 1u, out_i64, sizeof(out_i64), &out_len) == NVE_NITRO_OK);
  NVE_CHECK(out_len == 8u);
  int64_t decoded = 0;
  std::memcpy(&decoded, out_i64, sizeof(decoded));
  NVE_CHECK(decoded == 12);

  // Buffer-too-small query returns required length without crashing
  uint8_t tiny[2]{};
  size_t req_len = 0u;
  NVE_CHECK(nve_nitro_invoke_bytes(h, "tag", nullptr, 0u, tiny, sizeof(tiny), &req_len) ==
            NVE_NITRO_ERR_BUFFER_TOO_SMALL);
  NVE_CHECK(req_len == 10u); // "counter:12"
  std::vector<uint8_t> tag_buf(req_len);
  NVE_CHECK(nve_nitro_invoke_bytes(h, "tag", nullptr, 0u, tag_buf.data(), tag_buf.size(), &req_len) == NVE_NITRO_OK);
  NVE_CHECK(std::string(reinterpret_cast<const char*>(tag_buf.data()), req_len) == "counter:12");

  // Echo binary bytes
  const uint8_t raw_in[4] = {0xDEu, 0xADu, 0xBEu, 0xEFu};
  uint8_t raw_out[4]{};
  NVE_CHECK(nve_nitro_invoke_bytes(h, "echoBytes", raw_in, 4u, raw_out, 4u, &req_len) == NVE_NITRO_OK);
  NVE_CHECK(req_len == 4u && std::memcmp(raw_in, raw_out, 4u) == 0);

  // Unknown method & explicit error
  NVE_CHECK(nve_nitro_invoke_bytes(h, "unknownMethod", nullptr, 0u, nullptr, 0u, &req_len) == NVE_NITRO_ERR_NOT_FOUND);
  NVE_CHECK(nve_nitro_invoke_bytes(h, "fail", nullptr, 0u, nullptr, 0u, &req_len) == NVE_NITRO_ERR_INVOCATION_FAILED);

  // Dispose rejects subsequent calls
  nve_nitro_dispose(h);
  NVE_CHECK(nve_nitro_is_disposed(h) == 1);
  NVE_CHECK(nve_nitro_invoke_bytes(h, "tag", nullptr, 0u, nullptr, 0u, &req_len) == NVE_NITRO_ERR_DISPOSED);

  nve_nitro_release(h);
  NVE_CHECK(nve_nitro_is_disposed(nullptr) == 1);
  NVE_CHECK(nve_nitro_get_name(nullptr) == nullptr);
  NVE_CHECK(nve_nitro_memory_size(nullptr) == 0u);
  global_reg.unregister_hybrid_object("GlobalCounter");
}

} // namespace

int main() {
  test_array_buffer_zero_copy_and_owning();
  test_promise_resolve_reject_chaining_and_threads();
  test_hybrid_object_lifecycle_and_registry();
  test_c_abi_handle_bridge();
  std::puts("nve_nitro_bridge_test: ALL PASSED");
  return 0;
}
