#import "NveAppleComponentHostInternal.h"

#include <nve/host/ascii.hpp>
#include <nve/host/bound_table.hpp>

#include <algorithm>
#include <string>

namespace nve::apple {

namespace {

std::string_view trim_ascii(std::string_view s) noexcept {
  while (!s.empty() && (s.front() == ' ' || s.front() == '\t' || s.front() == '\r' || s.front() == '\n')) s.remove_prefix(1u);
  while (!s.empty() && (s.back() == ' ' || s.back() == '\t' || s.back() == '\r' || s.back() == '\n')) s.remove_suffix(1u);
  return s;
}

bool starts_with_iequals(std::string_view s, std::string_view prefix) noexcept {
  return s.size() >= prefix.size() && host::ascii_iequals(s.substr(0u, prefix.size()), prefix);
}

std::string strip_transport_query(std::string_view uri, std::string_view append_transport) {
  const size_t qpos = uri.find('?');
  if (qpos == std::string_view::npos) {
    return append_transport.empty() ? std::string(uri)
                                    : (std::string(uri) + "?transport=" + std::string(append_transport));
  }
  std::string out(uri.substr(0u, qpos));
  std::string kept;
  std::string_view query = uri.substr(qpos + 1u);
  while (!query.empty()) {
    const size_t amp = query.find('&');
    const std::string_view part = amp == std::string_view::npos ? query : query.substr(0u, amp);
    query = amp == std::string_view::npos ? std::string_view{} : query.substr(amp + 1u);
    if (part.empty()) continue;
    const size_t eq = part.find('=');
    const std::string_view key = eq == std::string_view::npos ? part : part.substr(0u, eq);
    if (host::ascii_iequals(key, "transport")) continue;
    if (!kept.empty()) kept.push_back('&');
    kept.append(part);
  }
  if (!append_transport.empty()) {
    if (!kept.empty()) kept.push_back('&');
    kept.append("transport=");
    kept.append(append_transport);
  }
  if (!kept.empty()) {
    out.push_back('?');
    out.append(kept);
  }
  return out;
}

std::string committed_segment(std::string_view before, std::string_view after) {
  if (before == after) return {};
  size_t prefix = 0u;
  const size_t min_len = std::min(before.size(), after.size());
  while (prefix < min_len && before[prefix] == after[prefix]) ++prefix;
  while (prefix > 0u && prefix < after.size() && (static_cast<uint8_t>(after[prefix]) & 0xC0u) == 0x80u) --prefix;
  size_t suffix = 0u;
  while (suffix < (before.size() - prefix) && suffix < (after.size() - prefix) &&
         before[before.size() - 1u - suffix] == after[after.size() - 1u - suffix]) {
    ++suffix;
  }
  while (suffix > 0u && (static_cast<uint8_t>(after[after.size() - suffix]) & 0xC0u) == 0x80u) --suffix;
  const size_t end = after.size() - suffix;
  return end > prefix ? std::string(after.substr(prefix, end - prefix)) : std::string{};
}

NSString *clamp_max_length(NSString *text, uint32_t max_len, bool &clamped) {
  clamped = false;
  NSString *safe = text ?: @"";
  if (max_len == 0u || safe.length <= max_len) return safe;
  __block uint32_t count = 0u;
  __block NSUInteger end = safe.length;
  __block bool did_clamp = false;
  [safe enumerateSubstringsInRange:NSMakeRange(0u, safe.length)
                           options:NSStringEnumerationByComposedCharacterSequences | NSStringEnumerationSubstringNotRequired
                        usingBlock:^(__unused NSString *sub, NSRange r, __unused NSRange enc, BOOL *stop) {
                          if (count == max_len) {
                            end = r.location;
                            did_clamp = true;
                            *stop = YES;
                            return;
                          }
                          ++count;
                        }];
  clamped = did_clamp;
  return did_clamp ? [safe substringToIndex:end] : safe;
}

}  // namespace

std::optional<PairUrlConfig> parse_pair_url(std::string_view url) {
  const std::string_view trimmed = trim_ascii(url);
  if (trimmed.empty()) return std::nullopt;
  NSURL *nsUrl = [NSURL URLWithString:nsstring_from_sv(trimmed)];
  if (nsUrl == nil) return std::nullopt;
  NSURLComponents *comps = [NSURLComponents componentsWithURL:nsUrl resolvingAgainstBaseURL:NO];
  if (comps == nil || ![comps.scheme.lowercaseString isEqualToString:@"nve"] ||
      ![comps.host.lowercaseString isEqualToString:@"pair"]) {
    return std::nullopt;
  }
  PairUrlConfig cfg{};
  std::string explicit_endpoint;
  bool has_host = false;
  for (NSURLQueryItem *item in comps.queryItems) {
    if (item.value.length == 0u) continue;
    NSString *key = item.name.lowercaseString;
    const char *val = item.value.UTF8String ?: "";
    if ([key isEqualToString:@"endpoint"] || [key isEqualToString:@"connect"] || [key isEqualToString:@"url"]) {
      explicit_endpoint = val;
    } else if ([key isEqualToString:@"token"]) {
      cfg.token = val;
    } else if ([key isEqualToString:@"transport"]) {
      cfg.transport = val;
    } else if ([key isEqualToString:@"host"]) {
      has_host = true;
    }
  }
  if (!explicit_endpoint.empty()) {
    cfg.endpoint = std::move(explicit_endpoint);
    return cfg;
  }
  if (!has_host) return std::nullopt;
  // Preserve canonical `nve://pair?host=...&quic=...&wss=...&token=...&cert_hash=...` intact
  // (lowercasing the `nve://` scheme for `nve-wire`'s `parse_wire_uri`).
  cfg.endpoint = "nve://" + std::string(trimmed.substr(6u));
  return cfg;
}

std::optional<std::string_view> canonical_pair_transport(std::string_view raw) noexcept {
  const std::string_view s = trim_ascii(raw);
  if (s.empty() || host::ascii_iequals(s, "auto") || host::ascii_iequals(s, "happy-eyeballs")) return "auto";
  if (host::ascii_iequals(s, "quic") || host::ascii_iequals(s, "udp")) return "quic";
  if (host::ascii_iequals(s, "wt") || host::ascii_iequals(s, "webtransport") || host::ascii_iequals(s, "h3")) {
    return "webtransport";
  }
  if (host::ascii_iequals(s, "wss") || host::ascii_iequals(s, "tls")) return "wss";
  if (host::ascii_iequals(s, "ws") || host::ascii_iequals(s, "tcp") || host::ascii_iequals(s, "websocket")) {
    return "ws";
  }
  return std::nullopt;
}

std::optional<std::string> apply_pair_transport(std::string_view uri, std::string_view transport) {
  const std::string_view trimmed_uri = trim_ascii(uri);
  const std::string_view trimmed_transport = trim_ascii(transport);
  if (trimmed_transport.empty()) return std::string(trimmed_uri);
  const std::optional<std::string_view> canon = canonical_pair_transport(trimmed_transport);
  if (!canon) return std::nullopt;
  if (trimmed_uri.empty()) return std::string();

  if (starts_with_iequals(trimmed_uri, "nve://")) {
    const std::string normalized = "nve://" + std::string(trimmed_uri.substr(6u));
    return strip_transport_query(normalized, *canon == "auto" ? std::string_view{} : *canon);
  }

  std::string_view rest = trimmed_uri;
  std::string_view orig_scheme;
  const size_t scheme_sep = rest.find("://");
  const size_t slash_or_q = rest.find_first_of("/?");
  if (scheme_sep != std::string_view::npos && (slash_or_q == std::string_view::npos || scheme_sep < slash_or_q)) {
    const std::string_view raw_scheme = rest.substr(0u, scheme_sep);
    bool recognized = false;
    for (std::string_view ok : {"quic", "udp", "wt", "https", "wss", "ws", "tcp"}) {
      if (host::ascii_iequals(raw_scheme, ok)) {
        recognized = true;
        break;
      }
    }
    if (!recognized) return std::nullopt;
    orig_scheme = rest.substr(0u, scheme_sep + 3u);
    rest.remove_prefix(scheme_sep + 3u);
  }

  const std::string cleaned_rest = strip_transport_query(rest, {});
  if (*canon == "auto") {
    std::string lower_scheme;
    lower_scheme.reserve(orig_scheme.size());
    for (char c : orig_scheme) lower_scheme.push_back(static_cast<char>(c >= 'A' && c <= 'Z' ? c - 'A' + 'a' : c));
    // Normalize wire-unsupported aliases (`udp://` -> `quic://`, `tcp://` -> `ws://`) to match `parse_wire_uri`.
    if (lower_scheme == "udp://") {
      lower_scheme = "quic://";
    } else if (lower_scheme == "tcp://") {
      lower_scheme = "ws://";
    }
    std::string out;
    out.reserve(lower_scheme.size() + cleaned_rest.size());
    out.append(lower_scheme);
    out.append(cleaned_rest);
    return out;
  }
  const std::string_view scheme = (*canon == "quic")           ? "quic://"
                                  : (*canon == "webtransport") ? "wt://"
                                  : (*canon == "wss")          ? "wss://"
                                                               : "ws://";
  std::string out;
  out.reserve(scheme.size() + cleaned_rest.size());
  out.append(scheme);
  out.append(cleaned_rest);
  return out;
}

}  // namespace nve::apple

@implementation NveAppleComponentHost (Session)

#pragma mark - Controlled text & IME (shared AppKit / UIKit)

- (nullable NSString *)textInputValueForNodeId:(uint32_t)nodeId {
  auto read_from = [&](const nve::host::SceneReader &scene) -> NSString * {
    const uint32_t idx = nve::host::node_index(nodeId);
    const NveNodeRow *row = scene.is_live(idx) ? scene.row(idx) : nullptr;
    if (row == nullptr || row->generation != nve::host::node_generation(nodeId)) return nil;
    nve::host::BoundTable table;
    table.bind(scene, NVE_TABLE_TEXT_INPUT_DESC);
    if (!table.has(idx)) return nil;
    return nve::apple::nsstring_from_sv(table.at(idx).str(NVE_TABLE_TEXT_INPUT_VALUE, scene));
  };
  if (_activeScene != nullptr) return read_from(_activeScene->reader());
  auto guard = _session.acquire_scene();
  return guard ? read_from(guard->reader()) : nil;
}

- (void)clearTextInputStateForNodeId:(uint32_t)nodeId {
  auto it = _textInputs.find(nodeId);
  if (it == _textInputs.end()) return;
  const bool was_composing = it->second.composing;
  const bool was_focused = it->second.focused;
  _textInputs.erase(it);
  if (was_composing) [self dispatchCompositionEndOnNodeId:nodeId text:@""];
  if (was_focused) [self dispatchFocusOnNodeId:nodeId focused:NO];
}

- (void)setTextInputMaxLength:(uint32_t)maxLength forNodeId:(uint32_t)nodeId {
  _textInputs[nodeId].max_length = maxLength;
}

- (BOOL)isTextInputComposingForNodeId:(uint32_t)nodeId {
  auto it = _textInputs.find(nodeId);
  return (it != _textInputs.end() && it->second.composing) ? YES : NO;
}

- (BOOL)shouldApplyAppText:(NSString *)value
                  toNodeId:(uint32_t)nodeId
                 shownText:(NSString *)shown
             hasMarkedText:(BOOL)hasMarkedText {
  auto &st = _textInputs[nodeId];
  const std::string val_str = value.UTF8String ?: "";
  if (hasMarkedText) {
    st.pending_app = val_str;
    return NO;
  }
  if (st.composing) {
    const std::string shown_str = shown.UTF8String ?: "";
    const std::string committed = nve::apple::committed_segment(st.pre_comp, shown_str);
    st.composing = false;
    st.comp_text.clear();
    st.pre_comp.clear();
    std::vector<nve::apple::TextEventSpec> batch;
    batch.push_back({NVE_TEXT_INPUT_EVENT_COMPOSITION_END, committed});
    const bool emitted = st.controlled.user_edited(shown_str);
    if (emitted) {
      st.last_committed = shown_str;
      if (_reconciling) st.synced = false;
      batch.push_back({NVE_TEXT_INPUT_EVENT_CHANGE, shown_str});
    }
    [self _dispatchTextEvents:batch nodeId:nodeId];
    auto it = _textInputs.find(nodeId);
    if (it == _textInputs.end()) return NO;
    if (emitted) {
      it->second.pending_app = val_str;
      return NO;
    }
  }
  auto it = _textInputs.find(nodeId);
  if (it == _textInputs.end()) return NO;
  auto &live = it->second;
  live.pending_app.reset();
  // Short-circuit unchanged app value before scanning `in_flight_` (`A -> B -> A` echo race).
  if (live.synced && val_str == live.controlled.app_value()) {
    return NO;
  }
  const bool write = live.controlled.app_published(val_str, shown.UTF8String ?: "");
  live.synced = true;
  if (write) live.last_committed = val_str;
  return write ? YES : NO;
}

- (void)handleTextInputEditOnNodeId:(uint32_t)nodeId
                               text:(NSString *)text
                      hasMarkedText:(BOOL)hasMarkedText
                         markedText:(nullable NSString *)markedText
                          writeBack:(void (^)(NSString *nextText))writeBack {
  auto &st = _textInputs[nodeId];
  if (st.controlled.writing()) return;
  if (hasMarkedText) {
    const std::string preedit = markedText.UTF8String ?: "";
    if (!st.composing) {
      st.composing = true;
      st.pre_comp = st.last_committed;
      st.comp_text = preedit;
      [self dispatchCompositionStartOnNodeId:nodeId text:markedText ?: @""];
    } else if (preedit != st.comp_text) {
      st.comp_text = preedit;
      [self dispatchCompositionUpdateOnNodeId:nodeId text:markedText ?: @""];
    }
    return;
  }

  bool clamped = false;
  NSString *safeText = nve::apple::clamp_max_length(text, st.max_length, clamped);
  const std::string cur = safeText.UTF8String ?: "";
  if (clamped && writeBack) {
    const auto scope = st.controlled.host_write();
    writeBack(safeText);
  }

  // Compute CompositionEnd + TextChange BEFORE dispatching and emit atomically in one batch so
  // synchronous dispatch -> renderFrame -> node destruction can never dangle `st` or fire stale `cur`.
  std::vector<nve::apple::TextEventSpec> batch;
  if (st.composing) {
    const std::string committed = nve::apple::committed_segment(st.pre_comp, cur);
    st.composing = false;
    st.comp_text.clear();
    st.pre_comp.clear();
    batch.push_back({NVE_TEXT_INPUT_EVENT_COMPOSITION_END, committed});
  }
  const bool emitted = st.controlled.user_edited(cur);
  if (emitted) {
    st.last_committed = cur;
    st.pending_app.reset();
    batch.push_back({NVE_TEXT_INPUT_EVENT_CHANGE, cur});
  }
  if (!batch.empty()) {
    [self _dispatchTextEvents:batch nodeId:nodeId];
  }

  auto it = _textInputs.find(nodeId);
  if (it == _textInputs.end()) return;

  if (emitted) {
    if (_appAttached && !_pairRequested) {
      if (NSString *answer = [self textInputValueForNodeId:nodeId]) {
        const bool snap = it->second.controlled.app_answered(answer.UTF8String ?: "");
        it->second.synced = true;
        if (snap) {
          it->second.last_committed = it->second.controlled.app_value();
          const auto scope = it->second.controlled.host_write();
          if (writeBack) writeBack(nve::apple::nsstring_from_sv(it->second.controlled.app_value()));
        }
      }
    }
    return;
  }

  if (it->second.pending_app.has_value()) {
    const std::string deferred = std::move(*it->second.pending_app);
    it->second.pending_app.reset();
    if ((!it->second.synced || deferred != it->second.controlled.app_value()) &&
        it->second.controlled.app_published(deferred, cur)) {
      it->second.synced = true;
      it->second.last_committed = deferred;
      const auto scope = it->second.controlled.host_write();
      if (writeBack) writeBack(nve::apple::nsstring_from_sv(deferred));
    }
  }
}

- (void)handleTextInputFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused {
  auto &st = _textInputs[nodeId];
  if (st.focused == (focused == YES)) return;
  st.focused = (focused == YES);
  if (!focused && st.composing) {
    st.composing = false;
    st.comp_text.clear();
    st.pre_comp.clear();
    [self dispatchCompositionEndOnNodeId:nodeId text:@""];
  }
  [self dispatchFocusOnNodeId:nodeId focused:focused];
}

- (BOOL)handleTextInputSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text hasMarkedText:(BOOL)hasMarkedText {
  if (hasMarkedText) return NO;
  const std::string cur = text.UTF8String ?: "";
  std::vector<nve::apple::TextEventSpec> batch;
  auto it = _textInputs.find(nodeId);
  if (it != _textInputs.end()) {
    if (it->second.composing) {
      const std::string committed = nve::apple::committed_segment(it->second.pre_comp, cur);
      it->second.composing = false;
      it->second.comp_text.clear();
      it->second.pre_comp.clear();
      batch.push_back({NVE_TEXT_INPUT_EVENT_COMPOSITION_END, committed});
    }
    if (cur != it->second.last_committed && it->second.controlled.user_edited(cur)) {
      it->second.last_committed = cur;
      it->second.pending_app.reset();
      batch.push_back({NVE_TEXT_INPUT_EVENT_CHANGE, cur});
    }
  }
  batch.push_back({NVE_TEXT_INPUT_EVENT_SUBMIT, cur});
  [self _dispatchTextEvents:batch nodeId:nodeId];
  return YES;
}

#pragma mark - Tier-2 reload

- (BOOL)reloadDylibAtPath:(NSString *)dylibPath error:(NSError **)error {
  return [self _loadAppModule:dylibPath carryState:_appAttached error:error];  // failure: live app untouched
}

#pragma mark - Pairing

- (BOOL)connectPairedEndpoint:(NSString *)uri token:(NSString *)token surfaceId:(uint32_t)surfaceId error:(NSError **)error {
  _surfaceId = surfaceId;
  // `pair_connect` swaps atomically: on success the runtime has dropped any attached app (so its
  // code may unload); on failure the running app is untouched.
  if (_session.pair_connect(uri.UTF8String ?: "", token.UTF8String ?: "") != NVE_RUNTIME_OK) {
    if (error) {
      const std::string detail = _session.last_error();
      *error = [self _errorWithCode:static_cast<NSInteger>(nve::host::ModuleError::Open)
                        description:[NSString stringWithFormat:@"Pairing with %@ failed: %s", uri, detail.c_str()]];
    }
    return NO;
  }
  _appAttached = NO;
  _pairRequested = YES;
  [self renderFrame];
  return YES;
}

- (void)disconnectPaired {
  if (!_pairRequested) return;
  (void)_session.pair_disconnect();
  _pairRequested = NO;
}

#pragma mark - Frame-0 (ABI v6) & raw NVE2 submission

- (BOOL)saveFrame0ToPath:(NSString *)path
              appBuildId:(uint64_t)appBuildId
                 envHash:(uint64_t)envHash
                   error:(NSError **)error {
  if (path.length == 0) {
    if (error) *error = [self _frame0Error:@"Saving Frame-0 failed" status:NVE_RUNTIME_INVALID_ARG];
    return NO;
  }
  const NveRuntimeStatus st = _session.save_frame0_file(path.UTF8String ?: "", appBuildId, envHash);
  if (st != NVE_RUNTIME_OK) {
    if (error) *error = [self _frame0Error:[NSString stringWithFormat:@"Saving Frame-0 %@ failed", path] status:st];
    return NO;
  }
  return YES;
}

- (BOOL)bootFrame0FromPath:(NSString *)path
                appBuildId:(uint64_t)appBuildId
                   envHash:(uint64_t)envHash
                     error:(NSError **)error {
  if (_appAttached) {
    if (error) *error = [self _frame0Error:@"Cannot boot Frame-0 while an app is attached" status:NVE_RUNTIME_BUSY];
    return NO;
  }
  if (path.length == 0) {
    if (error) *error = [self _frame0Error:@"Empty Frame-0 path" status:NVE_RUNTIME_INVALID_ARG];
    return NO;
  }
  const NveRuntimeStatus st = _session.boot_frame0_file(path.UTF8String ?: "", appBuildId, envHash);
  if (st != NVE_RUNTIME_OK) {
    if (error) *error = [self _frame0Error:[NSString stringWithFormat:@"Booting Frame-0 %@ failed", path] status:st];
    return NO;
  }
  [self renderFrame];
  [self syncNativeViewsFromHost];
  _hasReplayedFrame0 = YES;
  return YES;
}

- (BOOL)bootFrame0FromBytes:(NSData *)envelope
                 appBuildId:(uint64_t)appBuildId
                    envHash:(uint64_t)envHash
                      error:(NSError **)error {
  if (_appAttached) {
    if (error) *error = [self _frame0Error:@"Cannot boot Frame-0 while an app is attached" status:NVE_RUNTIME_BUSY];
    return NO;
  }
  if (envelope.length == 0) {
    if (error) *error = [self _frame0Error:@"Empty Frame-0 envelope" status:NVE_RUNTIME_INVALID_ARG];
    return NO;
  }
  const std::span<const uint8_t> bytes(static_cast<const uint8_t *>(envelope.bytes), envelope.length);
  const NveRuntimeStatus st = _session.boot_frame0_bytes(bytes, appBuildId, envHash);
  if (st != NVE_RUNTIME_OK) {
    if (error) *error = [self _frame0Error:@"Booting Frame-0 bytes failed" status:st];
    return NO;
  }
  [self renderFrame];
  [self syncNativeViewsFromHost];
  _hasReplayedFrame0 = YES;
  return YES;
}

- (BOOL)submitNve2Frame:(NSData *)frame {
  if (_appAttached || frame.length == 0) return NO;
  const std::span<const uint8_t> bytes(static_cast<const uint8_t *>(frame.bytes), frame.length);
  if (_session.submit_nve2(bytes) != NVE_RUNTIME_OK) return NO;
  [self renderFrame];
  [self syncNativeViewsFromHost];  // `submit_nve2` publishes immediately; mount it now
  return YES;
}

@end
