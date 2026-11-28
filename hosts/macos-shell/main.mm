#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>
#import "NveAppleComponentHost.h"
#import "NveMacWindowManager.h"
#import "RCTNveMacComponentView.h"

#include "NveAppleDemoApp.hpp"

#include <csignal>
#include <cstdint>
#include <cstdio>
#include <cstring>

@interface NveMacAppDelegate : NSObject <NSApplicationDelegate>
@property (nonatomic, copy, nullable) NSString *dylibPath;
@property (nonatomic, copy, nullable) NSString *pairConnect;
@property (nonatomic, copy, nullable) NSString *pairToken;
@property (nonatomic, strong) NveMacWindowManager *windowManager;
@property (nonatomic, assign) uint32_t primarySurfaceId;
@property (nonatomic, strong, nullable) dispatch_source_t sigintSource;
@property (nonatomic, strong, nullable) dispatch_source_t sigtermSource;
@end

@implementation NveMacAppDelegate

/// SIGINT / SIGTERM terminate through AppKit so windows (and their runtimes) tear down in order.
- (void)_installSignalHandlers {
  std::signal(SIGINT, SIG_IGN);
  std::signal(SIGTERM, SIG_IGN);
  auto makeSource = [](int sig) -> dispatch_source_t {
    dispatch_source_t src =
        dispatch_source_create(DISPATCH_SOURCE_TYPE_SIGNAL, static_cast<uintptr_t>(sig), 0, dispatch_get_main_queue());
    dispatch_source_set_event_handler(src, ^{
      [NSApp terminate:nil];
    });
    dispatch_resume(src);
    return src;
  };
  self.sigintSource = makeSource(SIGINT);
  self.sigtermSource = makeSource(SIGTERM);
}

- (void)_cancelSignalSources {
  if (self.sigintSource != nil) dispatch_source_cancel(self.sigintSource);
  if (self.sigtermSource != nil) dispatch_source_cancel(self.sigtermSource);
  self.sigintSource = nil;
  self.sigtermSource = nil;
}

- (void)dealloc {
  [self _cancelSignalSources];
}

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
  (void)notification;
  self.windowManager = [[NveMacWindowManager alloc] init];
  self.primarySurfaceId = 1u;
  self.windowManager.onAllWindowsClosed = ^{
    [NSApp terminate:nil];
  };

  RCTNveMacComponentView *rootView = [self.windowManager openWindowWithSurfaceId:self.primarySurfaceId
                                                                           title:@"NVE macOS Host Shell"
                                                                           frame:NSMakeRect(0, 0, 640, 420)
                                                                         present:NO];
  NSWindow *window = [self.windowManager windowForSurfaceId:self.primarySurfaceId];
  if (rootView == nil || window == nil) return;
  [self _installSignalHandlers];

  [window center];
  [window makeKeyAndOrderFront:nil];
  [NSApp activateIgnoringOtherApps:YES];

  if (self.pairConnect.length > 0) {
    NveAppleComponentHost *host = [NveAppleComponentHost hostForRootView:rootView];
    NSError *err = nil;
    if (!host || ![host connectPairedEndpoint:self.pairConnect
                                        token:(self.pairToken ?: @"")
                                    surfaceId:self.primarySurfaceId
                                        error:&err]) {
      NSLog(@"Failed to connect paired client to %@: %@", self.pairConnect, err.localizedDescription ?: @"unknown error");
    } else {
      [self.windowManager setTitle:[NSString stringWithFormat:@"NVE macOS Host Shell (Paired: %@)", self.pairConnect]
                      forSurfaceId:self.primarySurfaceId];
    }
  } else if (self.dylibPath.length > 0) {
    NSError *err = nil;
    if (![rootView loadNveDylibAtPath:self.dylibPath surfaceId:self.primarySurfaceId error:&err]) {
      NSLog(@"Failed to load NVE dylib %@: %@", self.dylibPath, err.localizedDescription);
    }
  } else {
    [rootView attachNveInterface:&nve::apple::DemoApp::get_interface surfaceId:self.primarySurfaceId];
  }
}

- (void)applicationWillTerminate:(NSNotification *)notification {
  (void)notification;
  [self _cancelSignalSources];
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
    NSString *pairTransport = NSProcessInfo.processInfo.environment[@"NVE_PAIR_TRANSPORT"];
    for (int i = 1; i < argc; ++i) {
      if (std::strcmp(argv[i], "--help") == 0 || std::strcmp(argv[i], "-h") == 0) {
        std::printf(
            "Usage: %s [--dylib <path/to/libapp.dylib>]"
            " [--pair-connect <nve://pair?...|ws(s)://...>] [--pair-token <token>]"
            " [--pair-transport <auto|quic|wt|wss|ws>]\n",
            argv[0]);
        return 0;
      } else if (std::strcmp(argv[i], "--dylib") == 0 && i + 1 < argc) {
        dylibPath = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-connect") == 0 && i + 1 < argc) {
        pairConnect = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-token") == 0 && i + 1 < argc) {
        pairToken = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strcmp(argv[i], "--pair-transport") == 0) {
        if (i + 1 >= argc) {
          std::fprintf(stderr, "error: --pair-transport requires a value (auto|quic|wt|wss|ws)\n");
          return 2;
        }
        pairTransport = [NSString stringWithUTF8String:argv[++i]];
      } else if (std::strncmp(argv[i], "--pair-transport=", 17) == 0) {
        pairTransport = [NSString stringWithUTF8String:argv[i] + 17];
      }
    }

    if (pairTransport.length > 0) {
      const char *rawTransport = pairTransport.UTF8String ?: "";
      if (!nve::apple::canonical_pair_transport(rawTransport).has_value()) {
        std::fprintf(stderr,
                     "error: invalid --pair-transport '%s' (expected auto|quic|wt|wss|ws)\n",
                     rawTransport);
        return 2;
      }
      if (pairConnect.length > 0) {
        const auto rewritten = nve::apple::apply_pair_transport(pairConnect.UTF8String ?: "", rawTransport);
        if (!rewritten.has_value()) {
          std::fprintf(stderr, "error: failed to apply --pair-transport '%s' to '%s'\n",
                       rawTransport, pairConnect.UTF8String ?: "");
          return 2;
        }
        pairConnect = [NSString stringWithUTF8String:rewritten->c_str()];
      }
    }

    NSApplication *app = [NSApplication sharedApplication];
    [app setActivationPolicy:NSApplicationActivationPolicyRegular];
    NveMacAppDelegate *delegate = [[NveMacAppDelegate alloc] init];
    delegate.dylibPath = dylibPath;
    delegate.pairConnect = pairConnect;
    delegate.pairToken = pairToken;
    app.delegate = delegate;
    [app run];
  }
  return 0;
}
