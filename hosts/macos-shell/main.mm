#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>
#import "NveAppleComponentHost.h"
#import "NveMacWindowManager.h"
#import "RCTNveMacComponentView.h"

#include "NveAppleDemoApp.hpp"

#include <nve/host/snapshot_file.hpp>

#include <csignal>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <string>
#include <vector>

namespace {

bool env_flag(const char *name) {
  const char *v = std::getenv(name);
  return v != nullptr && *v != '\0' && std::strcmp(v, "0") != 0 && std::strcmp(v, "false") != 0;
}

}  // namespace

@interface NveMacAppDelegate : NSObject <NSApplicationDelegate>
@property (nonatomic, copy, nullable) NSString *dylibPath;
@property (nonatomic, copy, nullable) NSString *pairConnect;
@property (nonatomic, copy, nullable) NSString *pairToken;
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
            "[nve-macos-shell] Instant-boot Frame-0 restore (%s) from %s in %llu us (nodes=%u)\n",
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
      [rootView attachNveInterface:&nve::apple::DemoApp::get_interface
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
    NSString *snapshotPath = nil;
    uint32_t schemaVersion = 1u;
    BOOL noSnapshot = NO;
    BOOL resetState = NO;
    for (int i = 1; i < argc; ++i) {
      if (std::strcmp(argv[i], "--help") == 0 || std::strcmp(argv[i], "-h") == 0) {
        std::printf(
            "Usage: %s [--dylib <path/to/libapp.dylib>]"
            " [--pair-connect <nve://pair?...|ws(s)://...>] [--pair-token <token>]"
            " [--snapshot <path.nvesnap>] [--no-snapshot] [--reset-state]"
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
      } else if (std::strcmp(argv[i], "--snapshot") == 0 && i + 1 < argc) {
        snapshotPath = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--schema") == 0 && i + 1 < argc) {
        const long parsed = std::strtol(argv[++i], nullptr, 10);
        if (parsed >= 0) {
          schemaVersion = static_cast<uint32_t>(parsed);
        }
      }
    }

    if (env_flag("NVE_NO_SNAPSHOT")) {
      noSnapshot = YES;
    }
    if (env_flag("NVE_RESET_STATE")) {
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
        const std::string resolved = nve::host::SnapshotFile::default_path(appIdentity).string();
        snapshotPath = [NSString stringWithUTF8String:resolved.c_str()];
      }
      if (resetState && snapshotPath.length > 0) {
        std::error_code ec;
        std::filesystem::remove(snapshotPath.fileSystemRepresentation, ec);
        std::fprintf(
            stderr,
            "[nve-macos-shell] Cleared cached snapshot at %s (--reset-state / NVE_RESET_STATE)\n",
            snapshotPath.UTF8String);
      }
      if (dylibPath.length > 0) {
        // Build-time Frame-0 shipped next to the dylib (`libapp.dylib.nvesnap`).
        sidecarFrame0Path = [dylibPath stringByAppendingString:@".nvesnap"];
      }
    }

    NSApplication *app = [NSApplication sharedApplication];
    [app setActivationPolicy:NSApplicationActivationPolicyRegular];
    NveMacAppDelegate *delegate = [[NveMacAppDelegate alloc] init];
    delegate.dylibPath = dylibPath;
    delegate.pairConnect = pairConnect;
    delegate.pairToken = pairToken;
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
