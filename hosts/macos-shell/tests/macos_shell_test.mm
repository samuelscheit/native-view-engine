#import <AppKit/AppKit.h>
#import "NveMacFabricHost.h"
#import "RCTNveMacComponentView.h"

#include "nve_rn_component.hpp"
#include "mock_jsi_recorder.hpp"

#include <cmath>
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

#undef assert
#define assert(expr) NVE_CHECK(expr)

namespace {

bool approx_eq(CGFloat a, CGFloat b, CGFloat eps = 1.0) {
  return std::fabs(a - b) <= eps;
}

struct MockMacRustComponent {
  uint32_t surface_id{1};
  int count{0};
  std::string title{"Initial"};
  std::vector<uint8_t> cmd_buf;
  nve::EventRingWriter outbound;
  std::vector<uint8_t> outbound_snap;

  static NveAppHandle create(uint32_t sid) {
    auto *self = new MockMacRustComponent();
    self->surface_id = sid;
    self->outbound.clear();
    return self;
  }

  static void destroy(NveAppHandle h) { delete static_cast<MockMacRustComponent *>(h); }

  template <typename T>
  void push(T v) {
    const auto *p = reinterpret_cast<const uint8_t *>(&v);
    cmd_buf.insert(cmd_buf.end(), p, p + sizeof(T));
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

  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    self->cmd_buf.clear();
    uint32_t ops = 0;

    self->push<uint32_t>(NVE_ABI_MAGIC);
    self->push<uint16_t>(NVE_ABI_VERSION);
    self->push<uint16_t>(0);
    self->push<uint32_t>(self->surface_id);
    self->push<uint64_t>(frame_id);
    self->push<uint32_t>(0);

    auto create_node = [&](uint32_t id, uint16_t kind) {
      self->push<uint8_t>(NVE_OP_CREATE_NODE);
      self->push<uint32_t>(id);
      self->push<uint16_t>(kind);
      self->push<uint16_t>(0);
      ++ops;
    };

    auto set_children = [&](uint32_t parent, std::initializer_list<uint32_t> kids) {
      self->push<uint8_t>(NVE_OP_SET_CHILDREN);
      self->push<uint32_t>(parent);
      self->push<uint32_t>(static_cast<uint32_t>(kids.size()));
      for (uint32_t k : kids) self->push<uint32_t>(k);
      ++ops;
    };

    auto set_text = [&](uint32_t id, const std::string &s) {
      self->push<uint8_t>(NVE_OP_SET_TEXT);
      self->push<uint32_t>(id);
      self->push<uint32_t>(static_cast<uint32_t>(s.size()));
      self->cmd_buf.insert(self->cmd_buf.end(), s.begin(), s.end());
      ++ops;
    };

    auto set_layout = [&](uint32_t id,
                          uint8_t dir,
                          uint8_t wtag,
                          float w,
                          uint8_t htag,
                          float h,
                          float grow,
                          float pad,
                          float gap) {
      self->push<uint8_t>(NVE_OP_SET_LAYOUT);
      self->push<uint32_t>(id);
      self->push_dim(wtag, w);
      self->push_dim(htag, h);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push_dim(0, 0.0f);
      self->push<uint8_t>(dir);
      self->push<uint8_t>(0); /* FlexStart */
      self->push<uint8_t>(0); /* Stretch */
      self->push<float>(grow);
      self->push<float>(1.0f);
      self->push<float>(gap);
      self->push_insets(pad, pad, pad, pad);
      self->push_insets(0, 0, 0, 0);
      ++ops;
    };

    auto set_style = [&](uint32_t id, uint32_t bg, uint32_t fg, float radius, float font_size) {
      self->push<uint8_t>(NVE_OP_SET_STYLE);
      self->push<uint32_t>(id);
      self->push<uint32_t>(bg);
      self->push<uint32_t>(fg);
      self->push<uint32_t>(0xFF0000FFu);
      self->push<float>(1.5f);
      self->push<float>(radius);
      self->push<float>(0.95f);
      self->push<float>(font_size);
      self->push<int32_t>(2);
      ++ops;
    };

    create_node(2, NVE_COMP_VIEW);
    create_node(3, NVE_COMP_TEXT);
    create_node(4, NVE_COMP_PRESSABLE);

    set_children(NVE_ROOT_NODE_ID, {2});
    set_children(2, {3, 4});

    // Row container: 100% x 100%, padding=20, gap=10
    set_layout(2, 1, 2, 100.0f, 2, 100.0f, 0.0f, 20.0f, 10.0f);
    set_style(2, 0xFF202020u, 0xFFFFFFFFu, 8.0f, 16.0f);

    // Label: fixed width=120, height=40
    set_layout(3, 0, 1, 120.0f, 1, 40.0f, 0.0f, 0.0f, 0.0f);
    set_text(3, self->title + ":" + std::to_string(self->count));
    set_style(3, 0x00000000u, 0xFFFFFFFFu, 0.0f, 18.0f);

    // Pressable button: flex_grow=1, height=40
    set_layout(4, 0, 0, 0.0f, 1, 40.0f, 1.0f, 0.0f, 0.0f);
    set_style(4, 0xFFFF5500u, 0xFFFFFFFFu, 12.0f, 16.0f);

    self->push<uint8_t>(NVE_OP_COMMIT);
    self->push<uint8_t>(1);
    ++ops;

    std::memcpy(self->cmd_buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{self->cmd_buf.data(), self->cmd_buf.size()};
  }

  static uint32_t dispatch_events(NveAppHandle h, NveBufferSlice events) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    std::vector<nve::DecodedEvent> decoded;
    if (!nve::decode_event_ring(events, decoded)) return 0;
    for (const auto &ev : decoded) {
      if (ev.kind == nve::EventKind::Press && ev.node_id == 4) {
        ++self->count;
        const std::string payload = "{\"count\":" + std::to_string(self->count) + "}";
        self->outbound.push_custom(
            self->surface_id,
            4,
            ev.timestamp_us,
            "onIncrement",
            reinterpret_cast<const uint8_t *>(payload.data()),
            payload.size());
        return 1;
      }
    }
    return 0;
  }

  static uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch_json) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    self->title.assign(reinterpret_cast<const char *>(patch_json.ptr), patch_json.len);
    return 1;
  }

  static NveBufferSlice snapshot_state(NveAppHandle) { return {nullptr, 0}; }
  static void restore_state(NveAppHandle, NveBufferSlice) {}

  static uint32_t set_props(NveAppHandle h, NveBufferSlice props) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    std::string json(reinterpret_cast<const char *>(props.ptr), props.len);
    if (json.find("Updated") != std::string::npos) {
      self->title = "Updated";
    }
    const std::string payload = "{\"ok\":true}";
    self->outbound.push_custom(
        self->surface_id,
        NVE_ROOT_NODE_ID,
        500,
        "onPropsApplied",
        reinterpret_cast<const uint8_t *>(payload.data()),
        payload.size());
    return 1;
  }

  static NveBufferSlice drain_emitted_events(NveAppHandle h) {
    auto *self = static_cast<MockMacRustComponent *>(h);
    self->outbound_snap = self->outbound.finalize();
    self->outbound.clear();
    return NveBufferSlice{self->outbound_snap.data(), self->outbound_snap.size()};
  }

  static const void *get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{
        .create = &create,
        .destroy = &destroy,
        .render = &render,
        .dispatch_events = &dispatch_events,
    };
    static const NveComponentApi kComponent{
        .set_props = &set_props,
        .drain_emitted_events = &drain_emitted_events,
    };
    static const NveDevApi kDev{
        .apply_hot_patch = &apply_hot_patch,
        .snapshot_state = &snapshot_state,
        .restore_state = &restore_state,
    };
    switch (interface_id) {
      case NVE_IFACE_SURFACE: return &kSurface;
      case NVE_IFACE_COMPONENT: return &kComponent;
      case NVE_IFACE_DEV: return &kDev;
      default: return nullptr;
    }
  }
};

} // namespace

@interface TestMacComponentDelegate : NSObject <RCTNveMacComponentViewDelegate>
@property (nonatomic, strong) NSMutableArray<NSString *> *events;
@end

@implementation TestMacComponentDelegate
- (instancetype)init {
  self = [super init];
  if (self) {
    _events = [NSMutableArray array];
  }
  return self;
}
- (void)nveComponentView:(NSView *)view
            didEmitEvent:(NSString *)eventName
                 payload:(NSDictionary<NSString *, id> *)payload {
  (void)view;
  (void)payload;
  [self.events addObject:eventName];
}
@end

int main() {
  @autoreleasepool {
    // 1. Test RCTNveMacComponentView with live AppKit subviews + Yoga layout
    RCTNveMacComponentView *view =
        [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 400, 160)];
    assert(view.isFlipped == YES);

    TestMacComponentDelegate *delegate = [[TestMacComponentDelegate alloc] init];
    view.delegate = delegate;

    __block NSMutableDictionary<NSString *, NSDictionary *> *received =
        [NSMutableDictionary dictionary];
    view.onNveEvent = ^(NSString *eventName, NSDictionary<NSString *, id> *payload) {
      received[eventName] = payload;
    };

    assert([view attachNveInterface:&MockMacRustComponent::get_interface surfaceId:9]);
    assert(view.nveSurfaceId == 9);
    assert(view.nveNodeCount == 4);

    NSView *container = [view nativeViewForNodeId:2];
    NSView *labelView = [view nativeViewForNodeId:3];
    NSView *buttonView = [view nativeViewForNodeId:4];
    assert(container != nil && labelView != nil && buttonView != nil);
    assert([labelView isKindOfClass:[NSTextField class]]);
    assert(labelView.superview == container);
    assert(buttonView.superview == container);
    assert(buttonView.gestureRecognizers.count == 1);

    // Check computed Flexbox layout on actual AppKit frames:
    // container = 400x160, pad=20, gap=10 -> inner width = 360.
    // label = (20, 20, 120, 40), button = (150, 20, 230, 40).
    assert(approx_eq(container.frame.size.width, 400.0));
    assert(approx_eq(container.frame.size.height, 160.0));
    assert(approx_eq(labelView.frame.origin.x, 20.0));
    assert(approx_eq(labelView.frame.origin.y, 20.0));
    assert(approx_eq(labelView.frame.size.width, 120.0));
    assert(approx_eq(labelView.frame.size.height, 40.0));
    assert(approx_eq(buttonView.frame.origin.x, 150.0));
    assert(approx_eq(buttonView.frame.origin.y, 20.0));
    assert(approx_eq(buttonView.frame.size.width, 230.0));
    assert(approx_eq(buttonView.frame.size.height, 40.0));

    NSTextField *label = (NSTextField *)labelView;
    assert([label.stringValue isEqualToString:@"Initial:0"]);
    assert(approx_eq(buttonView.layer.cornerRadius, 12.0));

    // 2. Apply props via NSDictionary -> JSON -> set_props -> re-render + outbound event
    assert([view applyPropsDictionary:@{@"title" : @"Updated"}]);
    assert([label.stringValue isEqualToString:@"Updated:0"]);
    assert(received[@"onPropsApplied"] != nil);
    assert([received[@"onPropsApplied"][@"ok"] boolValue] == YES);

    // 3. Simulate click on pressable node 4 -> count increments + onIncrement emitted
    [view simulatePressOnNodeId:4 atPoint:NSMakePoint(12.0, 12.0)];
    assert([label.stringValue isEqualToString:@"Updated:1"]);
    assert(received[@"onIncrement"] != nil);
    assert([received[@"onIncrement"][@"count"] intValue] == 1);
    assert(delegate.events.count == 2);

    // 4. Resize view -> Yoga recalculates flex_grow button width dynamically!
    [view setFrameSize:NSMakeSize(500.0, 160.0)];
    // New inner width = 460; label = 120; gap = 10 -> button = 330
    assert(approx_eq(buttonView.frame.size.width, 330.0));

    // 5. Test NveMacFabricHostController standalone lifecycle + error reporting
    NveSurfaceConfig cfg{.surface_id = 3, .width = 320.0f, .height = 240.0f, .scale_factor = 2.0f};
    NveMacFabricHostController *ctrl =
        [[NveMacFabricHostController alloc] initWithSurfaceConfig:cfg];
    assert([ctrl attachInterface:&MockMacRustComponent::get_interface]);
    assert(ctrl.isAppLoaded == YES);
    assert(ctrl.nodeCount == 4);
    assert([ctrl applyHotPatchJson:@"Patched"]);

    NSError *loadErr = nil;
    assert([ctrl loadRustAppDylibAtPath:@"/nonexistent/libmissing.dylib" error:&loadErr] == NO);
    assert(loadErr != nil);

    // 6. Real Rust Dioxus cdylib test: verify nested Text->Text inheritance, visible colors,
    //    non-zero AppKit label frames, live click -> Count: 1, and optional PNG render.
    const char *dylibEnv = std::getenv("NVE_TEST_COUNTER_DYLIB");
    if (dylibEnv && dylibEnv[0] != '\0') {
      RCTNveMacComponentView *counterView =
          [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 640, 420)];
      NSError *counterErr = nil;
      assert([counterView loadNveDylibAtPath:[NSString stringWithUTF8String:dylibEnv]
                                   surfaceId:1
                                       error:&counterErr]);
      assert(counterErr == nil);

      // Collect all leaf NSTextField labels recursively.
      std::vector<NSTextField *> labels;
      std::function<void(NSView *)> walk = [&](NSView *v) {
        if ([v isKindOfClass:[NSTextField class]]) {
          labels.push_back((NSTextField *)v);
        }
        for (NSView *sub in v.subviews) {
          walk(sub);
        }
      };
      walk(counterView);
      assert(labels.size() >= 4);

      bool sawTitle = false, sawCount0 = false, sawInc = false, sawReset = false;
      for (NSTextField *lbl : labels) {
        assert(lbl.frame.size.width > 20.0);
        assert(lbl.frame.size.height >= 16.0);
        NSColor *srgb = [lbl.textColor colorUsingColorSpace:[NSColorSpace sRGBColorSpace]];
        assert(srgb != nil && srgb.alphaComponent > 0.99);
        if ([lbl.stringValue isEqualToString:@"NATIVE VIEW ENGINE"]) sawTitle = true;
        if ([lbl.stringValue isEqualToString:@"Count: 0"]) {
          sawCount0 = true;
          assert(approx_eq(lbl.font.pointSize, 28.0));
        }
        if ([lbl.stringValue isEqualToString:@"Increment +"]) sawInc = true;
        if ([lbl.stringValue isEqualToString:@"Reset"]) sawReset = true;
      }
      assert(sawTitle && sawCount0 && sawInc && sawReset);

      const char *shotPath = std::getenv("NVE_TEST_SCREENSHOT_PATH");
      if (shotPath && shotPath[0] != '\0') {
        NSBitmapImageRep *rep =
            [counterView bitmapImageRepForCachingDisplayInRect:counterView.bounds];
        [counterView cacheDisplayInRect:counterView.bounds toBitmapImageRep:rep];
        NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
        assert(png != nil && png.length > 0);
        [png writeToFile:[NSString stringWithUTF8String:shotPath] atomically:YES];
      }
    }

    std::cout << "nve_macos_shell_test passed ("
              << (nve::fabric_host_has_yoga_layout() ? "Yoga+JSI+AppKit" : "Standalone+AppKit")
              << ")\n";
  }
  return 0;
}
