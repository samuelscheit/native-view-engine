#import <AppKit/AppKit.h>
#import "NveAppleComponentHost.h"
#import "NveMacWindowManager.h"


#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

#include "NveAppleDemoApp.hpp"

using nve::apple::DemoApp;

int main() {
  @autoreleasepool {
    [NSApplication sharedApplication];

    NveMacWindowManager *mgr = [[NveMacWindowManager alloc] init];
    NVE_CHECK(mgr.windowCount == 0u);

    __block BOOL allowPrimaryClose = NO;
    __block std::vector<uint32_t> closedOrder;
    __block int allClosedCalls = 0;

    mgr.shouldCloseWindow = ^BOOL(uint32_t surfaceId) {
      return surfaceId == 1u ? allowPrimaryClose : YES;
    };
    mgr.onWindowClosed = ^(uint32_t surfaceId) {
      closedOrder.push_back(surfaceId);
    };
    mgr.onAllWindowsClosed = ^{
      ++allClosedCalls;
    };

    // Open primary (1) and independent secondary (2).
    RCTNveMacComponentView *v1 = [mgr openWindowWithSurfaceId:1u
                                                        title:@"Primary"
                                                        frame:NSMakeRect(20, 20, 480, 320)
                                                      present:NO];
    RCTNveMacComponentView *v2 = [mgr openWindowWithSurfaceId:2u
                                                        title:@"Inspector"
                                                        frame:NSMakeRect(520, 20, 360, 280)
                                                      present:NO];
    NVE_CHECK(v1 != nil && v2 != nil);
    NVE_CHECK(mgr.windowCount == 2u);
    NVE_CHECK(mgr.focusedSurfaceId == 2u);
    NVE_CHECK([mgr openWindowWithSurfaceId:1u
                                     title:@"Dup"
                                     frame:NSMakeRect(0, 0, 200, 200)
                                   present:NO] == nil);

    // Distinct NveAppleComponentHost instances per window.
    NveAppleComponentHost *h1 = [NveAppleComponentHost hostForRootView:v1];
    NveAppleComponentHost *h2 = [NveAppleComponentHost hostForRootView:v2];
    NVE_CHECK(h1 != nil && h2 != nil && h1 != h2);
    NVE_CHECK([v1 attachNveInterface:&DemoApp::get_interface surfaceId:1u]);
    NVE_CHECK([v2 attachNveInterface:&DemoApp::get_interface surfaceId:2u]);
    [v2 simulatePressOnNodeId:DemoApp::kButton atPoint:NSMakePoint(4, 4)];
    NSTextField *lbl1 = (NSTextField *)[v1 nativeViewForNodeId:DemoApp::kLabel];
    NSTextField *lbl2 = (NSTextField *)[v2 nativeViewForNodeId:DemoApp::kLabel];
    NVE_CHECK([lbl1.stringValue isEqualToString:@"Clicks: 0"]);  // independent runtime surfaces
    NVE_CHECK([lbl2.stringValue isEqualToString:@"Clicks: 1"]);

    // Title, bounds, and always-on-top mutations.
    NVE_CHECK([mgr setTitle:@"Primary Updated" forSurfaceId:1u]);
    NVE_CHECK([[mgr windowForSurfaceId:1u].title isEqualToString:@"Primary Updated"]);
    NVE_CHECK([mgr setFrame:NSMakeRect(40, 40, 500, 340) forSurfaceId:1u]);
    NVE_CHECK([mgr setAlwaysOnTop:YES forSurfaceId:2u]);
    NVE_CHECK([mgr windowForSurfaceId:2u].level == NSFloatingWindowLevel);

    // Open modal sheet (3) parented to 1 -> blocks focusing 1.
    const NSWindowStyleMask sheetMask = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable;
    RCTNveMacComponentView *v3 = [mgr openWindowWithSurfaceId:3u
                                                        title:@"Modal Sheet"
                                                        frame:NSMakeRect(80, 80, 320, 200)
                                                    styleMask:sheetMask
                                              parentSurfaceId:1u
                                                        modal:YES
                                                  alwaysOnTop:NO
                                                 cascadeClose:YES
                                                      present:NO];
    NVE_CHECK(v3 != nil);
    NVE_CHECK([mgr activeModalBlockerForSurfaceId:1u] == 3u);
    NVE_CHECK(![mgr focusWindowWithSurfaceId:1u]);

    // Open popover (4) cascading off sheet (3).
    RCTNveMacComponentView *v4 = [mgr openWindowWithSurfaceId:4u
                                                        title:@"Popover"
                                                        frame:NSMakeRect(100, 100, 180, 120)
                                                    styleMask:sheetMask
                                              parentSurfaceId:3u
                                                        modal:NO
                                                  alwaysOnTop:YES
                                                 cascadeClose:YES
                                                      present:NO];
    NVE_CHECK(v4 != nil);
    NVE_CHECK(mgr.windowCount == 4u);

    // Vetoed close on primary leaves all 4 windows open.
    NVE_CHECK(![mgr requestCloseSurfaceId:1u]);
    NVE_CHECK(mgr.windowCount == 4u);
    NVE_CHECK(allClosedCalls == 0);

    // Allow close on primary -> cascades 4 -> 3 -> 1, leaving independent 2 open.
    allowPrimaryClose = YES;
    NVE_CHECK([mgr requestCloseSurfaceId:1u]);
    NVE_CHECK(mgr.windowCount == 1u);
    NVE_CHECK(allClosedCalls == 0);
    NVE_CHECK((closedOrder == std::vector<uint32_t>{4u, 3u, 1u}));
    NVE_CHECK([mgr windowForSurfaceId:2u] != nil);

    // Closing final window fires onAllWindowsClosed once.
    NVE_CHECK([mgr requestCloseSurfaceId:2u]);
    NVE_CHECK(mgr.windowCount == 0u);
    NVE_CHECK(allClosedCalls == 1);

    std::cout << "nve_macos_window_manager_test passed\n";
  }
  return 0;
}
