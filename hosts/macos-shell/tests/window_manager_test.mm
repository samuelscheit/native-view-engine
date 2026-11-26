#import <AppKit/AppKit.h>
#import "NveAppleComponentHost.h"
#import "NveMacWindowManager.h"

#include "nve_rn_component.hpp"

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

namespace {

struct MultiWinMockApp {
  uint32_t surface_id{1};
  int clicks{0};
  std::vector<uint8_t> cmd_buf;

  static NveAppHandle create(uint32_t sid) {
    auto *self = new MultiWinMockApp();
    self->surface_id = sid;
    return self;
  }
  static void destroy(NveAppHandle h) { delete static_cast<MultiWinMockApp *>(h); }

  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
  }

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<MultiWinMockApp *>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0u;
    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0u);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0u);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0u);
      ++ops;
    };
    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };
    auto set_text = [&](uint32_t id, const std::string &txt) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(txt.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), txt.begin(), txt.end());
      ++ops;
    };

    create_node(2u, NVE_COMP_TEXT);
    create_node(3u, NVE_COMP_PRESSABLE);
    set_children(NVE_ROOT_NODE_ID, {2u, 3u});
    set_text(2u, "S" + std::to_string(self->surface_id) + ":" + std::to_string(self->clicks));
    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1u);
    ++ops;
    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto *self = static_cast<MultiWinMockApp *>(h);
    if (!batch.ptr) return 0u;
    for (uint32_t i = 0; i < batch.len; ++i) {
      if (batch.ptr[i].kind == NVE_EVENT_PRESS && batch.ptr[i].node_id == 3u) {
        ++self->clicks;
        return 1u;
      }
    }
    return 0u;
  }

  static const void *get_interface(uint32_t iface_id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    return iface_id == NVE_IFACE_SURFACE ? &kSurface : nullptr;
  }
};

} // namespace

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
    NVE_CHECK([v1 attachNveInterface:&MultiWinMockApp::get_interface surfaceId:1u]);
    NVE_CHECK([v2 attachNveInterface:&MultiWinMockApp::get_interface surfaceId:2u]);
    [v2 simulatePressOnNodeId:3u atPoint:NSMakePoint(4, 4)];
    NSTextField *lbl1 = (NSTextField *)[v1 nativeViewForNodeId:2u];
    NSTextField *lbl2 = (NSTextField *)[v2 nativeViewForNodeId:2u];
    NVE_CHECK([lbl1.stringValue isEqualToString:@"S1:0"]);
    NVE_CHECK([lbl2.stringValue isEqualToString:@"S2:1"]);

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
