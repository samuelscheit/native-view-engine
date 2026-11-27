// macOS host suite over the real `libnve_runtime`: demo-app mount + styling, props, press ->
// outbound events, resize, dylib error codes, Tier-2 reload with state, instant-boot snapshots,
// core-table scenes (text input, image, scroll, accessibility), and pairing errors.
#import <AppKit/AppKit.h>
#import "NveAppleComponentHost.h"
#import "RCTNveMacComponentView.h"

#include "NveAppleDemoApp.hpp"

#include <nve/host/nve2_builder.hpp>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <string>

static int g_failures = 0;
#define CHECK(...)                                                                         \
  do {                                                                                     \
    if (!(__VA_ARGS__)) {                                                                  \
      std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #__VA_ARGS__); \
      ++g_failures;                                                                        \
    }                                                                                      \
  } while (0)

using nve::apple::DemoApp;
using nve::host::Key64;
using nve::host::Nve2FrameBuilder;
using nve::host::set_field;

namespace {

RCTNveMacComponentView *make_view() {
  return [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 400, 300)];
}

NSString *label_text(RCTNveMacComponentView *view, uint32_t node) {
  NSView *v = [view nativeViewForNodeId:node];
  return [v isKindOfClass:[NSTextField class]] ? ((NSTextField *)v).stringValue : nil;
}

bool near(CGFloat a, CGFloat b) { return std::fabs(a - b) < 0.5; }

std::vector<uint8_t> image_src(std::string_view url) {
  std::vector<uint8_t> out{0u};
  const auto n = static_cast<uint32_t>(url.size());
  for (uint32_t s = 0; s < 32u; s += 8u) out.push_back(static_cast<uint8_t>((n >> s) & 0xFFu));
  out.insert(out.end(), url.begin(), url.end());
  return out;
}

std::string png_data_uri() {
  NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:nullptr pixelsWide:4 pixelsHigh:4
      bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
  NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
  return std::string("data:image/png;base64,") + [png base64EncodedStringWithOptions:0].UTF8String;
}

/// ROOT(1) -> [input(2): secure text input, image(3): data-URI src + a11y label, scroll(4) -> row(5)].
NSData *core_table_scene() {
  auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
  Nve2FrameBuilder b;
  b.declare_core_tables().create_nodes({1u, 2u, 3u, 4u, 5u});
  const std::vector<uint8_t> src = image_src(png_data_uri());
  b.define_resource(9u, Key64::of("nve.res.image_src"), src);
  std::array<uint32_t, NVE_TABLE_TEXT_INPUT_WORDS> input{};
  std::copy(std::begin(NVE_TABLE_TEXT_INPUT_DEFAULTS), std::end(NVE_TABLE_TEXT_INPUT_DEFAULTS), input.begin());
  b.patch(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, set_field(input, NVE_TEXT_INPUT_SECURE, 1u), input);
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, NVE_TEXT_INPUT_VALUE.word, "hunter2");
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, NVE_TEXT_INPUT_PLACEHOLDER.word, "Password");
  std::array<uint32_t, NVE_TABLE_IMAGE_WORDS> img{};
  std::copy(std::begin(NVE_TABLE_IMAGE_DEFAULTS), std::end(NVE_TABLE_IMAGE_DEFAULTS), img.begin());
  b.patch(slot(NVE_TABLE_IMAGE_KEY), 3u, set_field(img, NVE_IMAGE_SRC, 9u), img);
  std::array<uint32_t, NVE_TABLE_ACCESSIBILITY_WORDS> a11y{};
  std::copy(std::begin(NVE_TABLE_ACCESSIBILITY_DEFAULTS), std::end(NVE_TABLE_ACCESSIBILITY_DEFAULTS), a11y.begin());
  b.patch(slot(NVE_TABLE_ACCESSIBILITY_KEY), 3u, set_field(a11y, NVE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_IMAGE), a11y);
  b.set_string(slot(NVE_TABLE_ACCESSIBILITY_KEY), 3u, NVE_ACCESSIBILITY_LABEL.word, "Brand logo");
  std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> lay{};
  std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 3u, set_field(lay, NVE_LAYOUT_WIDTH, 0x42000000u) | set_field(lay, NVE_LAYOUT_HEIGHT, 0x42000000u), lay);
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 4u, set_field(lay, NVE_LAYOUT_HEIGHT, 0x42C80000u), lay);  // 100pt
  std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 5u, set_field(lay, NVE_LAYOUT_HEIGHT, 0x43C80000u) | set_field(lay, NVE_LAYOUT_FLEX_SHRINK, 0u), lay);
  std::array<uint32_t, NVE_TABLE_SCROLL_WORDS> scroll{};
  std::copy(std::begin(NVE_TABLE_SCROLL_DEFAULTS), std::end(NVE_TABLE_SCROLL_DEFAULTS), scroll.begin());
  b.patch(slot(NVE_TABLE_SCROLL_KEY), 4u, 0u, scroll);
  b.set_children(1u, {2u, 3u, 4u});
  b.set_children(4u, {5u});
  const std::vector<uint8_t> frame = b.finish();
  return [NSData dataWithBytes:frame.data() length:frame.size()];
}

}  // namespace

int main() {
  @autoreleasepool {
    [NSApplication sharedApplication];

    // 1. Demo app mounts native views with resolved styling and parent-relative frames.
    RCTNveMacComponentView *view = make_view();
    CHECK(view.nveHost != nil);
    NSMutableArray<NSArray *> *events = [NSMutableArray array];
    view.onNveEvent = ^(NSString *name, NSDictionary *payload) { [events addObject:@[ name, payload ]]; };
    CHECK([view attachNveInterface:&DemoApp::get_interface surfaceId:7u]);
    CHECK(view.nveSurfaceId == 7u && view.nveNodeCount == 5u);
    CHECK([label_text(view, DemoApp::kTitle) isEqualToString:@"NVE Apple Host"]);
    CHECK([label_text(view, DemoApp::kLabel) isEqualToString:@"Clicks: 0"]);
    NSView *card = [view nativeViewForNodeId:DemoApp::kCard];
    NSView *button = [view nativeViewForNodeId:DemoApp::kButton];
    CHECK(card != nil && button != nil && button.superview == card);
    CHECK(near(card.layer.cornerRadius, 12.0));
    CHECK(card.layer.backgroundColor != nil);
    CHECK(near(button.frame.origin.x, 16.0) && near(button.frame.size.height, 44.0));  // card padding, relative frame
    NSTextField *title = (NSTextField *)[view nativeViewForNodeId:DemoApp::kTitle];
    CHECK(near(title.font.pointSize, 18.0));  // inherited typography from the card
    CHECK(button.gestureRecognizers.count == 1u && card.gestureRecognizers.count == 0u);
    CHECK(near(view.intrinsicContentSize.width, 320.0) && near(view.intrinsicContentSize.height, 200.0));

    // 2. Props + 3. press -> re-render + outbound custom event.
    CHECK([view applyPropsDictionary:@{@"title" : @"Hello props", @"ignored" : @[ @1, @{@"k" : NSNull.null} ]}]);
    CHECK([label_text(view, DemoApp::kTitle) isEqualToString:@"Hello props"]);
    [view simulatePressOnNodeId:DemoApp::kButton atPoint:CGPointMake(4, 4)];
    [view simulatePressOnNodeId:DemoApp::kButton atPoint:CGPointMake(4, 4)];
    CHECK([label_text(view, DemoApp::kLabel) isEqualToString:@"Clicks: 2"]);
    CHECK(events.count == 2u && [events.lastObject[0] isEqualToString:@"countChanged"]);
    CHECK([events.lastObject[1][@"value"] isEqual:@2]);
    CHECK(view.nveLastReconciledDirtyCount >= 1u && view.nveLastReconciledRemovedCount == 0u);

    // 4. Resize relayouts through the runtime viewport.
    const CGFloat before = card.frame.size.width;
    [view setFrameSize:NSMakeSize(600, 300)];
    CHECK(card.frame.size.width > before + 100.0);

    // 5. Dylib errors keep the running app.
    NSError *err = nil;
    CHECK(![view loadNveDylibAtPath:@"/nonexistent/libapp.dylib" surfaceId:7u error:&err] && err.code == 1);
    CHECK(![view loadNveDylibAtPath:@NVE_TEST_NO_ENTRY_DYLIB surfaceId:7u error:&err] && err.code == 2);
    CHECK(![view loadNveDylibAtPath:@NVE_TEST_NO_SURFACE_DYLIB surfaceId:7u error:&err] && err.code == 3);
    CHECK([label_text(view, DemoApp::kLabel) isEqualToString:@"Clicks: 2"]);
    RCTNveMacComponentView *failing = make_view();
    CHECK(![failing loadNveDylibAtPath:@NVE_TEST_CREATE_FAIL_DYLIB surfaceId:1u error:&err] && err.code == 3);

    // 6. Tier-2 reload keeps state; a failed reload keeps the live app.
    RCTNveMacComponentView *dylibView = make_view();
    CHECK([dylibView loadNveDylibAtPath:@NVE_TEST_STATEFUL_DYLIB surfaceId:3u error:&err]);
    [dylibView simulatePressOnNodeId:DemoApp::kButton atPoint:CGPointZero];
    CHECK([dylibView reloadNveDylibAtPath:@NVE_TEST_STATEFUL_DYLIB error:&err]);
    CHECK([label_text(dylibView, DemoApp::kLabel) isEqualToString:@"Clicks: 1"]);
    CHECK(![dylibView reloadNveDylibAtPath:@NVE_TEST_NO_SURFACE_DYLIB error:&err]);
    [dylibView simulatePressOnNodeId:DemoApp::kButton atPoint:CGPointZero];
    CHECK([label_text(dylibView, DemoApp::kLabel) isEqualToString:@"Clicks: 2"]);

    // 7. Instant boot: save, replay before any app attaches, then restore app state on attach.
    const auto path = std::filesystem::temp_directory_path() / "nve_macos_shell_test.nvesnap";
    CHECK([view saveSnapshotAtPath:@(path.c_str()) schemaVersion:4u]);
    RCTNveMacComponentView *cold = make_view();
    CHECK(![cold replayFrame0SnapshotAtPath:@(path.c_str()) expectedSchemaVersion:5u restoreTimeUs:nullptr]);
    uint64_t restoreUs = 0;
    CHECK([cold replayFrame0SnapshotAtPath:@(path.c_str()) expectedSchemaVersion:4u restoreTimeUs:&restoreUs]);
    CHECK(cold.hasReplayedFrame0 && [label_text(cold, DemoApp::kLabel) isEqualToString:@"Clicks: 2"]);
    CHECK([cold attachNveInterface:&DemoApp::get_interface surfaceId:1u]);
    CHECK([label_text(cold, DemoApp::kLabel) isEqualToString:@"Clicks: 2"]);  // restored app state
    std::filesystem::remove(path);

    // 8. Core tables: secure text input, image with a11y, scroll document sizing.
    RCTNveMacComponentView *scene = make_view();
    CHECK([scene.nveHost replayFrame0:core_table_scene()]);
    NSView *input = [scene nativeViewForNodeId:2u];
    CHECK([input isKindOfClass:[NSSecureTextField class]]);
    CHECK([((NSTextField *)input).stringValue isEqualToString:@"hunter2"]);
    CHECK([((NSTextField *)input).placeholderString isEqualToString:@"Password"]);
    NSView *image = [scene nativeViewForNodeId:3u];
    CHECK([image isKindOfClass:[NveAppleImageView class]] && ((NveAppleImageView *)image).image != nil);
    CHECK([image.accessibilityLabel isEqualToString:@"Brand logo"] && [image.accessibilityRole isEqualToString:NSAccessibilityImageRole]);
    NSView *scroller = [scene nativeViewForNodeId:4u];
    CHECK([scroller isKindOfClass:[NSScrollView class]]);
    CHECK([scene nativeViewForNodeId:5u].superview == ((NSScrollView *)scroller).documentView);
    CHECK(near(((NSScrollView *)scroller).documentView.frame.size.height, 400.0));

    // 9. Pairing errors surface the runtime's diagnostics.
    CHECK(![scene connectPairedEndpoint:@"" token:@"" surfaceId:1u error:&err] && err.localizedDescription.length > 0);
    CHECK(!scene.isPaired);

    // 10. Real Rust apps (`check-all.sh`: kitchen-sink + studio `cdylib`s) mount, render text,
    //     reload with state, and round-trip an instant-boot snapshot.
    for (const char *env : {"NVE_TEST_KITCHEN_SINK_DYLIB", "NVE_TEST_STUDIO_DYLIB"}) {
      const char *dylib = std::getenv(env);
      if (dylib == nullptr || *dylib == '\0') continue;
      RCTNveMacComponentView *real = [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 1024, 768)];
      CHECK([real loadNveDylibAtPath:@(dylib) surfaceId:11u error:&err]);
      // Async apps (Dioxus tasks) render after a runtime wake, delivered through the main queue.
      for (int i = 0; i < 200 && real.nveNodeCount <= 10u; ++i) {
        [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
      }
      CHECK(real.nveNodeCount > 10u);
      NSUInteger labels = 0;
      NSMutableArray<NSView *> *stack = [NSMutableArray arrayWithObject:real];
      while (stack.count > 0) {
        NSView *v = stack.lastObject;
        [stack removeLastObject];
        if ([v isKindOfClass:[NSTextField class]] && ((NSTextField *)v).stringValue.length > 0) ++labels;
        [stack addObjectsFromArray:v.subviews];
      }
      CHECK(labels > 0u);
      CHECK([real reloadNveDylibAtPath:@(dylib) error:&err]);
      const auto real_path = std::filesystem::temp_directory_path() / "nve_macos_shell_real.nvesnap";
      CHECK([real saveSnapshotAtPath:@(real_path.c_str()) schemaVersion:1u]);
      RCTNveMacComponentView *boot = [[RCTNveMacComponentView alloc] initWithFrame:NSMakeRect(0, 0, 1024, 768)];
      CHECK([boot replayFrame0SnapshotAtPath:@(real_path.c_str()) expectedSchemaVersion:1u restoreTimeUs:nullptr]);
      CHECK(boot.nveNodeCount == real.nveNodeCount);
      std::filesystem::remove(real_path);
      std::printf("  real app %s: %u nodes, %lu labels\n", env, real.nveNodeCount, static_cast<unsigned long>(labels));
    }

    std::printf("macos_shell_test: %s (%d failure(s))\n", g_failures == 0 ? "ok" : "FAILED", g_failures);
  }
  return g_failures == 0 ? 0 : 1;
}