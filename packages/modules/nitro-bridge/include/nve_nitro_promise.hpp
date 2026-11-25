#pragma once

#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <type_traits>
#include <utility>
#include <vector>

namespace nve::nitro {

// -----------------------------------------------------------------------------
// Promise<T> & Promise<void> (Thread-Safe Future + Callback Chaining)
// -----------------------------------------------------------------------------

enum class PromiseStatus : uint8_t {
  Pending = 0,
  Resolved = 1,
  Rejected = 2,
};

template <typename T>
struct PromiseResult {
  bool ok{false};
  std::optional<T> value_opt{};
  std::string error_msg{};

  bool is_ok() const noexcept { return ok; }
  bool is_err() const noexcept { return !ok; }
  const T& value() const { return *value_opt; }
  T& value() { return *value_opt; }
  const std::string& error() const noexcept { return error_msg; }
};

template <>
struct PromiseResult<void> {
  bool ok{false};
  std::string error_msg{};

  bool is_ok() const noexcept { return ok; }
  bool is_err() const noexcept { return !ok; }
  const std::string& error() const noexcept { return error_msg; }
};

namespace detail {

template <typename T>
struct PromiseState {
  std::mutex mu;
  std::condition_variable cv;
  PromiseStatus status{PromiseStatus::Pending};
  std::optional<T> value{};
  std::string error{};
  std::vector<std::function<void(const T&)>> on_resolve;
  std::vector<std::function<void(const std::string&)>> on_reject;
};

template <>
struct PromiseState<void> {
  std::mutex mu;
  std::condition_variable cv;
  PromiseStatus status{PromiseStatus::Pending};
  std::string error{};
  std::vector<std::function<void()>> on_resolve;
  std::vector<std::function<void(const std::string&)>> on_reject;
};

} // namespace detail

template <typename T>
class Promise final {
 public:
  Promise() : state_(std::make_shared<detail::PromiseState<T>>()) {}

  static Promise<T> create() { return Promise<T>(); }

  static Promise<T> resolved(T value) {
    Promise<T> p;
    p.resolve(std::move(value));
    return p;
  }

  static Promise<T> rejected(std::string error) {
    Promise<T> p;
    p.reject(std::move(error));
    return p;
  }

  bool resolve(T value) {
    std::vector<std::function<void(const T&)>> callbacks;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      if (state_->status != PromiseStatus::Pending) return false;
      state_->value = std::move(value);
      state_->status = PromiseStatus::Resolved;
      callbacks = std::move(state_->on_resolve);
      state_->on_reject.clear();
    }
    state_->cv.notify_all();
    for (auto& cb : callbacks) {
      if (cb) cb(*state_->value);
    }
    return true;
  }

  bool reject(std::string error) {
    std::vector<std::function<void(const std::string&)>> callbacks;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      if (state_->status != PromiseStatus::Pending) return false;
      state_->error = std::move(error);
      state_->status = PromiseStatus::Rejected;
      callbacks = std::move(state_->on_reject);
      state_->on_resolve.clear();
    }
    state_->cv.notify_all();
    for (auto& cb : callbacks) {
      if (cb) cb(state_->error);
    }
    return true;
  }

  PromiseStatus status() const noexcept {
    std::lock_guard<std::mutex> lock(state_->mu);
    return state_->status;
  }

  bool is_pending() const noexcept { return status() == PromiseStatus::Pending; }
  bool is_resolved() const noexcept { return status() == PromiseStatus::Resolved; }
  bool is_rejected() const noexcept { return status() == PromiseStatus::Rejected; }
  bool is_settled() const noexcept { return status() != PromiseStatus::Pending; }

  template <typename Fn>
  auto then(Fn&& fn) const {
    using Ret = std::invoke_result_t<Fn, const T&>;
    Promise<Ret> next = Promise<Ret>::create();
    auto fn_shared = std::make_shared<std::decay_t<Fn>>(std::forward<Fn>(fn));

    auto run_resolve = [fn_shared, next](const T& val) mutable {
      if constexpr (std::is_void_v<Ret>) {
        (*fn_shared)(val);
        next.resolve();
      } else {
        next.resolve((*fn_shared)(val));
      }
    };
    auto run_reject = [next](const std::string& err) mutable { next.reject(err); };

    PromiseStatus current = PromiseStatus::Pending;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      current = state_->status;
      if (current == PromiseStatus::Pending) {
        state_->on_resolve.push_back(std::move(run_resolve));
        state_->on_reject.push_back(std::move(run_reject));
        return next;
      }
    }
    if (current == PromiseStatus::Resolved) {
      run_resolve(*state_->value);
    } else {
      run_reject(state_->error);
    }
    return next;
  }

  template <typename Fn>
  const Promise<T>& catch_error(Fn&& fn) const {
    PromiseStatus current = PromiseStatus::Pending;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      current = state_->status;
      if (current == PromiseStatus::Pending) {
        state_->on_reject.emplace_back(std::forward<Fn>(fn));
        return *this;
      }
    }
    if (current == PromiseStatus::Rejected) {
      fn(state_->error);
    }
    return *this;
  }

  PromiseResult<T> await_result() const {
    std::unique_lock<std::mutex> lock(state_->mu);
    state_->cv.wait(lock, [&] { return state_->status != PromiseStatus::Pending; });
    if (state_->status == PromiseStatus::Resolved) {
      return PromiseResult<T>{.ok = true, .value_opt = *state_->value, .error_msg = {}};
    }
    return PromiseResult<T>{.ok = false, .value_opt = std::nullopt, .error_msg = state_->error};
  }

  std::optional<PromiseResult<T>> await_result_for(std::chrono::milliseconds timeout) const {
    std::unique_lock<std::mutex> lock(state_->mu);
    if (!state_->cv.wait_for(lock, timeout, [&] { return state_->status != PromiseStatus::Pending; })) {
      return std::nullopt;
    }
    if (state_->status == PromiseStatus::Resolved) {
      return PromiseResult<T>{.ok = true, .value_opt = *state_->value, .error_msg = {}};
    }
    return PromiseResult<T>{.ok = false, .value_opt = std::nullopt, .error_msg = state_->error};
  }

 private:
  std::shared_ptr<detail::PromiseState<T>> state_;
};

template <>
class Promise<void> final {
 public:
  Promise() : state_(std::make_shared<detail::PromiseState<void>>()) {}

  static Promise<void> create() { return Promise<void>(); }

  static Promise<void> resolved() {
    Promise<void> p;
    p.resolve();
    return p;
  }

  static Promise<void> rejected(std::string error) {
    Promise<void> p;
    p.reject(std::move(error));
    return p;
  }

  bool resolve() {
    std::vector<std::function<void()>> callbacks;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      if (state_->status != PromiseStatus::Pending) return false;
      state_->status = PromiseStatus::Resolved;
      callbacks = std::move(state_->on_resolve);
      state_->on_reject.clear();
    }
    state_->cv.notify_all();
    for (auto& cb : callbacks) {
      if (cb) cb();
    }
    return true;
  }

  bool reject(std::string error) {
    std::vector<std::function<void(const std::string&)>> callbacks;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      if (state_->status != PromiseStatus::Pending) return false;
      state_->error = std::move(error);
      state_->status = PromiseStatus::Rejected;
      callbacks = std::move(state_->on_reject);
      state_->on_resolve.clear();
    }
    state_->cv.notify_all();
    for (auto& cb : callbacks) {
      if (cb) cb(state_->error);
    }
    return true;
  }

  PromiseStatus status() const noexcept {
    std::lock_guard<std::mutex> lock(state_->mu);
    return state_->status;
  }

  bool is_pending() const noexcept { return status() == PromiseStatus::Pending; }
  bool is_resolved() const noexcept { return status() == PromiseStatus::Resolved; }
  bool is_rejected() const noexcept { return status() == PromiseStatus::Rejected; }
  bool is_settled() const noexcept { return status() != PromiseStatus::Pending; }

  template <typename Fn>
  auto then(Fn&& fn) const {
    using Ret = std::invoke_result_t<Fn>;
    Promise<Ret> next = Promise<Ret>::create();
    auto fn_shared = std::make_shared<std::decay_t<Fn>>(std::forward<Fn>(fn));

    auto run_resolve = [fn_shared, next]() mutable {
      if constexpr (std::is_void_v<Ret>) {
        (*fn_shared)();
        next.resolve();
      } else {
        next.resolve((*fn_shared)());
      }
    };
    auto run_reject = [next](const std::string& err) mutable { next.reject(err); };

    PromiseStatus current = PromiseStatus::Pending;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      current = state_->status;
      if (current == PromiseStatus::Pending) {
        state_->on_resolve.push_back(std::move(run_resolve));
        state_->on_reject.push_back(std::move(run_reject));
        return next;
      }
    }
    if (current == PromiseStatus::Resolved) {
      run_resolve();
    } else {
      run_reject(state_->error);
    }
    return next;
  }

  template <typename Fn>
  const Promise<void>& catch_error(Fn&& fn) const {
    PromiseStatus current = PromiseStatus::Pending;
    {
      std::lock_guard<std::mutex> lock(state_->mu);
      current = state_->status;
      if (current == PromiseStatus::Pending) {
        state_->on_reject.emplace_back(std::forward<Fn>(fn));
        return *this;
      }
    }
    if (current == PromiseStatus::Rejected) {
      fn(state_->error);
    }
    return *this;
  }

  PromiseResult<void> await_result() const {
    std::unique_lock<std::mutex> lock(state_->mu);
    state_->cv.wait(lock, [&] { return state_->status != PromiseStatus::Pending; });
    if (state_->status == PromiseStatus::Resolved) {
      return PromiseResult<void>{.ok = true, .error_msg = {}};
    }
    return PromiseResult<void>{.ok = false, .error_msg = state_->error};
  }

  std::optional<PromiseResult<void>> await_result_for(std::chrono::milliseconds timeout) const {
    std::unique_lock<std::mutex> lock(state_->mu);
    if (!state_->cv.wait_for(lock, timeout, [&] { return state_->status != PromiseStatus::Pending; })) {
      return std::nullopt;
    }
    if (state_->status == PromiseStatus::Resolved) {
      return PromiseResult<void>{.ok = true, .error_msg = {}};
    }
    return PromiseResult<void>{.ok = false, .error_msg = state_->error};
  }

 private:
  std::shared_ptr<detail::PromiseState<void>> state_;
};

} // namespace nve::nitro
