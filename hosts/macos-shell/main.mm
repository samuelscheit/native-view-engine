#import <AppKit/AppKit.h>
#import "NveAppleComponentHost.h"
#import "NveMacWindowManager.h"
#import "RCTNveMacComponentView.h"

#include "nve_rn_component.hpp"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

namespace {

// Minimal built-in demo NVE app when launched without `--dylib <path>`.
class BuiltinMacDemoApp {
 public:
  explicit BuiltinMacDemoApp(uint32_t sid) : surface_id_(sid) {}

  static NveAppHandle create(uint32_t sid) { return new BuiltinMacDemoApp(sid); }
  static void destroy(NveAppHandle h) { delete static_cast<BuiltinMacDemoApp *>(h); }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<BuiltinMacDemoApp *>(h);
    self->encode_frame(frame_id);
    return NveBufferSlice{self->cmd_buf_.data(), self->cmd_buf_.size()};
  }

  static uint32_t dispatch_events_direct(NveAppHandle h, NveEventBatch batch) {
    auto *self = static_cast<BuiltinMacDemoApp *>(h);
    if (batch.ptr == nullptr) return 0;
    for (uint32_t i = 0; i < batch.len; ++i) {
      const NveDirectEvent &ev = batch.ptr[i];
      if (ev.kind == static_cast<uint8_t>(nve::EventKind::Press) && ev.node_id == 4) {
        ++self->count_;
        return 1;
      }
    }
    return 0;
  }

  static NveBufferSlice snapshot_state(NveAppHandle h) {
    auto *self = static_cast<BuiltinMacDemoApp *>(h);
    self->state_buf_ = std::to_string(self->count_);
    return NveBufferSlice{
        reinterpret_cast<const uint8_t *>(self->state_buf_.data()),
        self->state_buf_.size(),
    };
  }

  static void restore_state(NveAppHandle h, NveBufferSlice state) {
    auto *self = static_cast<BuiltinMacDemoApp *>(h);
    if (state.ptr == nullptr || state.len == 0u) return;
    const std::string raw(reinterpret_cast<const char *>(state.ptr), state.len);
    self->count_ = std::atoi(raw.c_str());
  }

  static const void *get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = nullptr,
        .dispatch_events_direct = &dispatch_events_direct,
    };
    static const NveDevApi kDev{
        .apply_hot_patch = nullptr,
        .snapshot_state = &snapshot_state,
        .restore_state = &restore_state,
    };
    if (interface_id == NVE_IFACE_SURFACE) return &kSurface;
    if (interface_id == NVE_IFACE_DEV) return &kDev;
    return nullptr;
  }

 private:
  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf_.insert(cmd_buf_.end(), p, p + sizeof(T));
  }

  void push_str(const std::string &s) {
    cmd_buf_.insert(cmd_buf_.end(), s.begin(), s.end());
  }

  void push_dim(uint8_t tag, float val) {
    push<uint8_t>(tag);
    push<float>(val);
  }

  void push_insets(float t, float r, float b, float l) {
    push<float>(t);
    push<float>(r);
    push<float>(b);
    push<float>(l);
  }

  void encode_frame(uint64_t frame_id) {
    cmd_buf_.clear();
    uint32_t ops = 0;
    push<uint32_t>(NVE_ABI_MAGIC);
    push<uint16_t>(NVE_ABI_VERSION);
    push<uint16_t>(0);
    push<uint32_t>(surface_id_);
    push<uint64_t>(frame_id);
    push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      push<uint8_t>(NVE_OP_CREATE_NODE);
      push<uint32_t>(id);
      push<uint16_t>(kind);
      push<uint16_t>(0);
      ++ops;
    };

    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      push<uint8_t>(NVE_OP_SET_CHILDREN);
      push<uint32_t>(parent);
      push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) push<uint32_t>(k);
      ++ops;
    };

    auto set_text = [&](uint32_t id, const std::string &txt) {
      push<uint8_t>(NVE_OP_SET_TEXT);
      push<uint32_t>(id);
      push<uint32_t>(static_cast<uint32_t>(txt.size()));
      push_str(txt);
      ++ops;
    };

    auto set_style = [&](uint32_t id, uint32_t bg, uint32_t fg, float radius, float font_size) {
      push<uint8_t>(NVE_OP_SET_STYLE);
      push<uint32_t>(id);
      push<uint32_t>(bg);
      push<uint32_t>(fg);
      push<uint32_t>(0u);
      push<float>(0.0f);
      push<float>(radius);
      push<float>(1.0f);
      push<float>(font_size);
      push<int32_t>(0);
      ++ops;
    };

    auto set_layout = [&](uint32_t id, uint8_t wtag, float w, uint8_t htag, float h, float pad, float gap) {
      push<uint8_t>(NVE_OP_SET_LAYOUT);
      push<uint32_t>(id);
      push_dim(wtag, w);
      push_dim(htag, h);
      push_dim(0, 0.0f);
      push_dim(0, 0.0f);
      push_dim(0, 0.0f);
      push_dim(0, 0.0f);
      push<uint8_t>(0); /* Column */
      push<uint8_t>(1); /* Center */
      push<uint8_t>(2); /* Center */
      push<float>(0.0f);
      push<float>(1.0f);
      push<float>(gap);
      push_insets(pad, pad, pad, pad);
      push_insets(0, 0, 0, 0);
      ++ops;
    };

    create_node(2, NVE_COMP_VIEW);
    create_node(3, NVE_COMP_TEXT);
    create_node(4, NVE_COMP_PRESSABLE);
    create_node(5, NVE_COMP_TEXT);

    set_children(NVE_ROOT_NODE_ID, {2});
    set_children(2, {3, 4});
    set_children(4, {5});

    set_layout(2, 2, 100.0f, 2, 100.0f, 24.0f, 16.0f);
    set_style(2, 0xFF1E1B18u, 0xFFFFFFFFu, 0.0f, 16.0f);

    set_text(3, "NVE macOS Host Shell — Count: " + std::to_string(count_));
    set_style(3, 0x00000000u, 0xFFFFFFFFu, 0.0f, 22.0f);

    set_layout(4, 1, 200.0f, 1, 44.0f, 10.0f, 0.0f);
    set_style(4, 0xFFFF6B35u, 0xFFFFFFFFu, 10.0f, 16.0f);

    set_text(5, "Click to Increment");
    set_style(5, 0x00000000u, 0xFFFFFFFFu, 0.0f, 15.0f);

    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(1);
    ++ops;

    std::memcpy(cmd_buf_.data() + 20, &ops, sizeof(uint32_t));
  }

  uint32_t surface_id_{1};
  int count_{0};
  std::vector<uint8_t> cmd_buf_;
  std::string state_buf_;
};

} // namespace

@interface NveMacAppDelegate : NSObject <NSApplicationDelegate>
@property (nonatomic, copy, nullable) NSString *dylibPath;
@property (nonatomic, copy, nullable) NSString *pairConnect;
@property (nonatomic, copy, nullable) NSString *pairToken;
@property (nonatomic, copy, nullable) NSString *pairTransport;
@property (nonatomic, copy, nullable) NSString *snapshotPath;
@property (nonatomic, assign) uint32_t schemaVersion;
@property (nonatomic, assign) BOOL snapshotSaved;
@property (nonatomic, strong) NveMacWindowManager *windowManager;
@property (nonatomic, assign) uint32_t primarySurfaceId;
@end

@implementation NveMacAppDelegate

- (void)_persistSnapshotIfConfigured {
  RCTNveMacComponentView *rootView =
      [self.windowManager componentViewForSurfaceId:self.primarySurfaceId];
  if (self.snapshotSaved || self.snapshotPath.length == 0 || rootView == nil) {
    return;
  }
  self.snapshotSaved = YES;
  if ([rootView saveSnapshotAtPath:self.snapshotPath schemaVersion:self.schemaVersion]) {
    std::fprintf(
        stderr,
        "[nve-macos-shell] Saved Frame-0 + state snapshot to %s (schema v%u)\n",
        self.snapshotPath.UTF8String,
        self.schemaVersion);
  } else {
    std::fprintf(
        stderr,
        "[nve-macos-shell] Warning: failed to save snapshot to %s\n",
        self.snapshotPath.UTF8String);
  }
}

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
  (void)notification;
  self.windowManager = [[NveMacWindowManager alloc] init];
  self.primarySurfaceId = 1u;

  __weak NveMacAppDelegate *weakSelf = self;
  self.windowManager.shouldCloseWindow = ^BOOL(uint32_t surfaceId) {
    NveMacAppDelegate *strongSelf = weakSelf;
    if (strongSelf != nil && surfaceId == strongSelf.primarySurfaceId) {
      [strongSelf _persistSnapshotIfConfigured];
    }
    return YES;
  };
  self.windowManager.onAllWindowsClosed = ^{
    [NSApp terminate:nil];
  };

  const NSRect frame = NSMakeRect(0, 0, 640, 420);
  RCTNveMacComponentView *rootView =
      [self.windowManager openWindowWithSurfaceId:self.primarySurfaceId
                                            title:@"NVE macOS Host Shell"
                                            frame:frame
                                          present:NO];
  NSWindow *window = [self.windowManager windowForSurfaceId:self.primarySurfaceId];
  if (rootView == nil || window == nil) {
    return;
  }

  if (self.snapshotPath.length > 0 &&
      [[NSFileManager defaultManager] fileExistsAtPath:self.snapshotPath]) {
    uint64_t restoreUs = 0;
    if ([rootView replayFrame0SnapshotAtPath:self.snapshotPath
                       expectedSchemaVersion:self.schemaVersion
                               restoreTimeUs:&restoreUs]) {
      std::fprintf(
          stderr,
          "[nve-macos-shell] Instant-boot Frame-0 mmap restore from %s in %llu us\n",
          self.snapshotPath.UTF8String,
          static_cast<unsigned long long>(restoreUs));
    }
  }

  [window center];
  [window makeKeyAndOrderFront:nil];
  [NSApp activateIgnoringOtherApps:YES];

  if (self.pairConnect.length > 0) {
    NveAppleComponentHost *host = [NveAppleComponentHost hostForRootView:rootView];
    NSError *err = nil;
    if (!host || ![host connectPairedEndpoint:self.pairConnect
                                        token:(self.pairToken ?: @"")
                                    transport:self.pairTransport
                                    surfaceId:self.primarySurfaceId
                                        error:&err]) {
      NSLog(@"Failed to connect paired client to %@: %@",
            self.pairConnect,
            err.localizedDescription ?: @"unknown error");
    } else {
      [self.windowManager
          setTitle:[NSString stringWithFormat:@"NVE macOS Host Shell (Paired: %@)", self.pairConnect]
      forSurfaceId:self.primarySurfaceId];
    }
  } else if (self.dylibPath.length > 0) {
    NSError *err = nil;
    if (![rootView loadNveDylibAtPath:self.dylibPath surfaceId:self.primarySurfaceId error:&err]) {
      NSLog(@"Failed to load NVE dylib %@: %@", self.dylibPath, err.localizedDescription);
    }
  } else {
    [rootView attachNveInterface:&BuiltinMacDemoApp::get_interface surfaceId:self.primarySurfaceId];
  }
}

- (void)applicationWillTerminate:(NSNotification *)notification {
  (void)notification;
  [self _persistSnapshotIfConfigured];
  [self.windowManager closeAllWindows];
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
  (void)sender;
  return self.windowManager.windowCount == 0u;
}

@end

int main(int argc, const char *argv[]) {
  @autoreleasepool {
    NSString *dylibPath = nil;
    NSString *pairConnect = nil;
    NSString *pairToken = nil;
    NSString *pairTransport = nil;
    NSString *snapshotPath = nil;
    uint32_t schemaVersion = 1u;
    for (int i = 1; i + 1 < argc; ++i) {
      if (std::strcmp(argv[i], "--dylib") == 0) {
        dylibPath = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-connect") == 0) {
        pairConnect = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-token") == 0) {
        pairToken = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-transport") == 0) {
        pairTransport = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--snapshot") == 0) {
        snapshotPath = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--schema") == 0) {
        const long parsed = std::strtol(argv[++i], nullptr, 10);
        if (parsed >= 0) {
          schemaVersion = static_cast<uint32_t>(parsed);
        }
      }
    }
    NSApplication *app = [NSApplication sharedApplication];
    [app setActivationPolicy:NSApplicationActivationPolicyRegular];
    NveMacAppDelegate *delegate = [[NveMacAppDelegate alloc] init];
    delegate.dylibPath = dylibPath;
    delegate.pairConnect = pairConnect;
    delegate.pairToken = pairToken;
    delegate.pairTransport = pairTransport;
    delegate.snapshotPath = snapshotPath;
    delegate.schemaVersion = schemaVersion;
    app.delegate = delegate;
    [app run];
  }
  return 0;
}
