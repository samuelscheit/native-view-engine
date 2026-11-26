#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>
#import "NveAppleComponentHost.h"
#import "NveMacWindowManager.h"
#import "RCTNveMacComponentView.h"

#include "nve_rn_component.hpp"
#include "nve_snapshot.hpp"

#include <csignal>
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

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
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
        .dispatch_events = &dispatch_events,
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
@property (nonatomic, copy, nullable) NSString *sidecarFrame0Path;
@property (nonatomic, assign) uint32_t schemaVersion;
@property (nonatomic, assign) BOOL noSnapshot;
@property (nonatomic, assign) BOOL resetState;
@property (nonatomic, strong) NveMacWindowManager *windowManager;
@property (nonatomic, weak, nullable) NSWindow *primaryWindow;
@property (nonatomic, assign) uint32_t primarySurfaceId;
@property (nonatomic, strong, nullable) dispatch_source_t sigintSource;
@property (nonatomic, strong, nullable) dispatch_source_t sigtermSource;
@end

@implementation NveMacAppDelegate

- (void)dealloc {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
  if (self.sigintSource != nil) {
    dispatch_source_cancel(self.sigintSource);
    self.sigintSource = nil;
  }
  if (self.sigtermSource != nil) {
    dispatch_source_cancel(self.sigtermSource);
    self.sigtermSource = nil;
  }
}

- (void)_persistSnapshotWithReason:(const char *)reason
                      explicitView:(nullable RCTNveMacComponentView *)explicitView {
  if (self.noSnapshot || self.snapshotPath.length == 0) {
    return;
  }
  RCTNveMacComponentView *rootView =
      explicitView ?: [self.windowManager componentViewForSurfaceId:self.primarySurfaceId];
  if (rootView == nil || rootView.nveNodeCount == 0u) {
    return;
  }
  if ([rootView saveSnapshotAtPath:self.snapshotPath schemaVersion:self.schemaVersion]) {
    std::fprintf(
        stderr,
        "[nve-macos-shell] Saved Frame-0 + state snapshot to %s (schema v%u, trigger=%s)\n",
        self.snapshotPath.UTF8String,
        self.schemaVersion,
        reason);
  } else {
    std::fprintf(
        stderr,
        "[nve-macos-shell] Warning: failed to save snapshot to %s (trigger=%s)\n",
        self.snapshotPath.UTF8String,
        reason);
  }
}

- (void)_handleWindowWillClose:(NSNotification *)notification {
  NSWindow *closingWindow = notification.object;
  if (![closingWindow isKindOfClass:[NSWindow class]] || closingWindow != self.primaryWindow) {
    return;
  }
  RCTNveMacComponentView *rootView =
      [closingWindow.contentView isKindOfClass:[RCTNveMacComponentView class]]
          ? (RCTNveMacComponentView *)closingWindow.contentView
          : nil;
  [self _persistSnapshotWithReason:"window-close" explicitView:rootView];
  self.primaryWindow = nil;
}

- (void)_installSignalHandlers {
  std::signal(SIGINT, SIG_IGN);
  std::signal(SIGTERM, SIG_IGN);
  __weak NveMacAppDelegate *weakSelf = self;
  auto makeSource = [weakSelf](int sig, const char *sigName) -> dispatch_source_t {
    dispatch_source_t src = dispatch_source_create(
        DISPATCH_SOURCE_TYPE_SIGNAL,
        static_cast<uintptr_t>(sig),
        0,
        dispatch_get_main_queue());
    dispatch_source_set_event_handler(src, ^{
      NveMacAppDelegate *strongSelf = weakSelf;
      if (strongSelf != nil) {
        [strongSelf _persistSnapshotWithReason:sigName explicitView:nil];
      }
      [NSApp terminate:nil];
    });
    dispatch_resume(src);
    return src;
  };
  self.sigintSource = makeSource(SIGINT, "SIGINT");
  self.sigtermSource = makeSource(SIGTERM, "SIGTERM");
}

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
  (void)notification;
  self.windowManager = [[NveMacWindowManager alloc] init];
  self.primarySurfaceId = 1u;

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
  self.primaryWindow = window;

  [[NSNotificationCenter defaultCenter] addObserver:self
                                           selector:@selector(_handleWindowWillClose:)
                                               name:NSWindowWillCloseNotification
                                             object:window];
  [self _installSignalHandlers];

  BOOL replayedFrame0 = NO;
  if (!self.noSnapshot) {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSString *replayPath = nil;
    const char *replaySource = "user-snapshot";
    if (self.snapshotPath.length > 0 && [fm fileExistsAtPath:self.snapshotPath]) {
      replayPath = self.snapshotPath;
    } else if (!self.resetState && self.sidecarFrame0Path.length > 0 &&
               [fm fileExistsAtPath:self.sidecarFrame0Path]) {
      replayPath = self.sidecarFrame0Path;
      replaySource = "sidecar-frame0";
    }
    if (replayPath.length > 0) {
      uint64_t restoreUs = 0;
      if ([rootView replayFrame0SnapshotAtPath:replayPath
                         expectedSchemaVersion:self.schemaVersion
                                 restoreTimeUs:&restoreUs]) {
        replayedFrame0 = YES;
        std::fprintf(
            stderr,
            "[nve-macos-shell] Instant-boot Frame-0 mmap restore (%s) from %s in %llu us (nodes=%u)\n",
            replaySource,
            replayPath.UTF8String,
            static_cast<unsigned long long>(restoreUs),
            rootView.nveNodeCount);
      }
    }
  }

  [window center];
  [window makeKeyAndOrderFront:nil];
  [window displayIfNeeded];
  [NSApp activateIgnoringOtherApps:YES];

  __weak NveMacAppDelegate *weakSelf = self;
  void (^mountLiveApp)(void) = ^{
    NveMacAppDelegate *strongSelf = weakSelf;
    if (strongSelf == nil) {
      return;
    }
    if (strongSelf.pairConnect.length > 0) {
      NveAppleComponentHost *host = [NveAppleComponentHost hostForRootView:rootView];
      NSError *err = nil;
      if (!host || ![host connectPairedEndpoint:strongSelf.pairConnect
                                          token:(strongSelf.pairToken ?: @"")
                                      transport:strongSelf.pairTransport
                                      surfaceId:strongSelf.primarySurfaceId
                                          error:&err]) {
        NSLog(@"Failed to connect paired client to %@: %@",
              strongSelf.pairConnect,
              err.localizedDescription ?: @"unknown error");
      } else {
        [strongSelf.windowManager
            setTitle:[NSString stringWithFormat:@"NVE macOS Host Shell (Paired: %@)",
                                                strongSelf.pairConnect]
        forSurfaceId:strongSelf.primarySurfaceId];
      }
    } else if (strongSelf.dylibPath.length > 0) {
      NSError *err = nil;
      if (![rootView loadNveDylibAtPath:strongSelf.dylibPath
                              surfaceId:strongSelf.primarySurfaceId
                                  error:&err]) {
        NSLog(@"Failed to load NVE dylib %@: %@",
              strongSelf.dylibPath,
              err.localizedDescription);
      } else if (replayedFrame0) {
        std::fprintf(
            stderr,
            "[nve-macos-shell] Mounted live dylib %s with restored app state (nodes=%u)\n",
            strongSelf.dylibPath.UTF8String,
            rootView.nveNodeCount);
      }
    } else {
      [rootView attachNveInterface:&BuiltinMacDemoApp::get_interface
                         surfaceId:strongSelf.primarySurfaceId];
      if (replayedFrame0) {
        std::fprintf(
            stderr,
            "[nve-macos-shell] Mounted built-in app with restored app state (nodes=%u)\n",
            rootView.nveNodeCount);
      }
    }
  };

  if (replayedFrame0) {
    dispatch_async(dispatch_get_main_queue(), mountLiveApp);
  } else {
    mountLiveApp();
  }
}

- (void)applicationDidResignActive:(NSNotification *)notification {
  (void)notification;
  [self _persistSnapshotWithReason:"resign-active" explicitView:nil];
}

- (void)applicationWillTerminate:(NSNotification *)notification {
  (void)notification;
  [self _persistSnapshotWithReason:"terminate" explicitView:nil];
  self.primaryWindow = nil;
  [[NSNotificationCenter defaultCenter] removeObserver:self];
  if (self.sigintSource != nil) {
    dispatch_source_cancel(self.sigintSource);
    self.sigintSource = nil;
  }
  if (self.sigtermSource != nil) {
    dispatch_source_cancel(self.sigtermSource);
    self.sigtermSource = nil;
  }
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
    BOOL noSnapshot = NO;
    BOOL resetState = NO;
    for (int i = 1; i < argc; ++i) {
      if (std::strcmp(argv[i], "--help") == 0 || std::strcmp(argv[i], "-h") == 0) {
        std::printf(
            "Usage: %s [--dylib <path/to/libapp.dylib>]"
            " [--pair-connect <host:port|url>] [--pair-token <token>]"
            " [--pair-transport <udp|tcp|ws>]"
            " [--snapshot <path.nvereloc>] [--no-snapshot] [--reset-state]"
            " [--schema <ver>]\n",
            argv[0]);
        return 0;
      } else if (std::strcmp(argv[i], "--no-snapshot") == 0) {
        noSnapshot = YES;
      } else if (std::strcmp(argv[i], "--reset-state") == 0) {
        resetState = YES;
      } else if (std::strcmp(argv[i], "--dylib") == 0 && i + 1 < argc) {
        dylibPath = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-connect") == 0 && i + 1 < argc) {
        pairConnect = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-token") == 0 && i + 1 < argc) {
        pairToken = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-transport") == 0 && i + 1 < argc) {
        pairTransport = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--snapshot") == 0 && i + 1 < argc) {
        snapshotPath = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--schema") == 0 && i + 1 < argc) {
        const long parsed = std::strtol(argv[++i], nullptr, 10);
        if (parsed >= 0) {
          schemaVersion = static_cast<uint32_t>(parsed);
        }
      }
    }

    if (nve::is_snapshot_disabled()) {
      noSnapshot = YES;
    }
    if (nve::is_snapshot_reset_requested()) {
      resetState = YES;
    }

    NSString *sidecarFrame0Path = nil;
    if (noSnapshot) {
      snapshotPath = nil;
    } else {
      if (snapshotPath.length == 0 && pairConnect.length == 0) {
        const std::string appIdentity =
            dylibPath.length > 0 ? std::string(dylibPath.UTF8String)
                                 : std::string("nve-macos-shell-builtin");
        const std::string resolved = nve::resolve_default_snapshot_path(appIdentity);
        if (!resolved.empty()) {
          snapshotPath = [NSString stringWithUTF8String:resolved.c_str()];
        }
      }
      if (resetState && snapshotPath.length > 0) {
        nve::remove_snapshot_file_if_exists(snapshotPath.UTF8String);
        std::fprintf(
            stderr,
            "[nve-macos-shell] Cleared cached snapshot at %s (--reset-state / NVE_RESET_STATE)\n",
            snapshotPath.UTF8String);
      }
      if (dylibPath.length > 0) {
        const std::string sidecar = nve::resolve_sidecar_frame0_path(dylibPath.UTF8String);
        if (!sidecar.empty()) {
          sidecarFrame0Path = [NSString stringWithUTF8String:sidecar.c_str()];
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
    delegate.sidecarFrame0Path = sidecarFrame0Path;
    delegate.schemaVersion = schemaVersion;
    delegate.noSnapshot = noSnapshot;
    delegate.resetState = resetState;
    app.delegate = delegate;
    [app run];
  }
  return 0;
}
