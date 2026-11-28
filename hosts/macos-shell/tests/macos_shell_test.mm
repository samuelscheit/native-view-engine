// macOS host suite over the real `libnve_runtime`: demo-app mount + styling, props, press ->
// outbound events, resize, calls re-entering from reconcile, runtime-loaded dylib error codes,
// Tier-2 reload with state, core-table scenes (text input, image, scroll, accessibility, rewritten
// image resources), min-content measurement, and pairing errors.
#import <AppKit/AppKit.h>
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"
#import "RCTNveMacComponentView.h"

#include "NveAppleDemoApp.hpp"
#include "NveAppleValueBridge.h"

#include <nve/canvas/canvas.hpp>
#include <nve/canvas/draw_list_encoder.hpp>
#include <nve/host/bin_codec.hpp>
#include <nve/host/core_resources.hpp>
#include <nve/host/host_session.hpp>
#include <nve/host/nve2_builder.hpp>

#include <array>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
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

NSData *png(NSInteger pixels) {
  NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:nullptr pixelsWide:pixels pixelsHigh:pixels
      bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
  return [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
}

std::string png_data_uri(NSInteger pixels = 4) {
  return std::string("data:image/png;base64,") + [png(pixels) base64EncodedStringWithOptions:0].UTF8String;
}

/// Revision 1: ROOT(1) -> [image(2), image(3), scroll(4, 100pt) -> [row(5, 400pt), row(6, 200pt)]];
/// both images read inline-PNG resource 9. Revision 2 detaches the still-live image 3 and shrinks
/// row 5 to 100pt (row 6 only moves). Revision 3 rewrites tag-1 inline-PNG resource 9 to 8x8 px
/// while leaving node 2 clean.
NSData *detach_scene(uint64_t revision) {
  auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
  auto height = [&](Nve2FrameBuilder &b, uint32_t node, uint32_t bits) {
    std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> lay{};
    std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
    b.patch(slot(NVE_TABLE_LAYOUT_KEY), node,
            set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, bits) | set_field(lay, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0u), lay);
  };
  Nve2FrameBuilder b(revision);
  if (revision == 1u) {
    b.declare_core_tables().create_nodes({1u, 2u, 3u, 4u, 5u, 6u});
    NSData *bytes = png(4);
    b.define_resource(9u, nve::host::kImageSrcKind,
                      nve::host::encode_image_bytes(
                          std::span<const uint8_t>(static_cast<const uint8_t *>(bytes.bytes), bytes.length)));
    std::array<uint32_t, NVE_TABLE_IMAGE_WORDS> img{};
    std::copy(std::begin(NVE_TABLE_IMAGE_DEFAULTS), std::end(NVE_TABLE_IMAGE_DEFAULTS), img.begin());
    for (const uint32_t node : {2u, 3u}) b.patch(slot(NVE_TABLE_IMAGE_KEY), node, set_field(img, NVE_TABLE_IMAGE_SRC, 9u), img);
    std::array<uint32_t, NVE_TABLE_SCROLL_WORDS> scroll{};
    std::copy(std::begin(NVE_TABLE_SCROLL_DEFAULTS), std::end(NVE_TABLE_SCROLL_DEFAULTS), scroll.begin());
    b.patch(slot(NVE_TABLE_SCROLL_KEY), 4u, 0u, scroll);
    height(b, 4u, 0x42C80000u);  // 100pt
    height(b, 5u, 0x43C80000u);  // 400pt
    height(b, 6u, 0x43480000u);  // 200pt
    b.set_children(1u, {2u, 3u, 4u});
    b.set_children(4u, {5u, 6u});
  } else if (revision == 2u) {
    b.set_children(1u, {2u, 4u});
    height(b, 5u, 0x42C80000u);
  } else {
    NSData *bytes = png(8);
    b.define_resource(9u, nve::host::kImageSrcKind,
                      nve::host::encode_image_bytes(
                          std::span<const uint8_t>(static_cast<const uint8_t *>(bytes.bytes), bytes.length)));
  }
  const std::vector<uint8_t> frame = b.finish();
  return [NSData dataWithBytes:frame.data() length:frame.size()];
}

/// Deferred `z_index` batch restacking scene: ROOT(1) -> box(2) -> [3, 4, 5].
/// Revision 1 mounts `[3, 4, 5]` with z=0; revision 2 updates z(3)=5, z(5)=-1 without changing
/// children; revision 3 updates z(4)=10 without changing children.
NSData *z_order_scene(uint64_t revision) {
  auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
  auto set_z = [&](Nve2FrameBuilder &b, uint32_t node, int32_t z) {
    std::array<uint32_t, NVE_TABLE_VISUAL_WORDS> vis{};
    std::copy(std::begin(NVE_TABLE_VISUAL_DEFAULTS), std::end(NVE_TABLE_VISUAL_DEFAULTS), vis.begin());
    b.patch(slot(NVE_TABLE_VISUAL_KEY), node, set_field(vis, NVE_TABLE_VISUAL_Z_INDEX, static_cast<uint32_t>(z)), vis);
  };
  Nve2FrameBuilder b(revision);
  if (revision == 1u) {
    b.declare_core_tables().create_nodes({1u, 2u, 3u, 4u, 5u});
    b.set_children(1u, {2u});
    b.set_children(2u, {3u, 4u, 5u});
  } else if (revision == 2u) {
    set_z(b, 3u, 5);
    set_z(b, 5u, -1);
  } else {
    set_z(b, 4u, 10);
  }
  const std::vector<uint8_t> frame = b.finish();
  return [NSData dataWithBytes:frame.data() length:frame.size()];
}

/// ROOT(1) -> [input(2): secure text input, image(3): data-URI src + tint + a11y label,
///             scroll(4) -> row(5), decorative(6): presentation role].
NSData *core_table_scene() {
  auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
  Nve2FrameBuilder b;
  b.declare_core_tables().create_nodes({1u, 2u, 3u, 4u, 5u, 6u});
  const std::vector<uint8_t> src = nve::host::encode_image_url(png_data_uri());
  b.define_resource(9u, nve::host::kImageSrcKind, src);
  std::array<uint32_t, NVE_TABLE_TEXT_INPUT_WORDS> input{};
  std::copy(std::begin(NVE_TABLE_TEXT_INPUT_DEFAULTS), std::end(NVE_TABLE_TEXT_INPUT_DEFAULTS), input.begin());
  b.patch(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, set_field(input, NVE_TABLE_TEXT_INPUT_SECURE, 1u), input);
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, NVE_TABLE_TEXT_INPUT_VALUE.word, "hunter2");
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, NVE_TABLE_TEXT_INPUT_PLACEHOLDER.word, "Password");
  std::array<uint32_t, NVE_TABLE_IMAGE_WORDS> img{};
  std::copy(std::begin(NVE_TABLE_IMAGE_DEFAULTS), std::end(NVE_TABLE_IMAGE_DEFAULTS), img.begin());
  b.patch(slot(NVE_TABLE_IMAGE_KEY), 3u,
          set_field(img, NVE_TABLE_IMAGE_SRC, 9u) | set_field(img, NVE_TABLE_IMAGE_TINT, 0xFF00FF00u), img);
  std::array<uint32_t, NVE_TABLE_ACCESSIBILITY_WORDS> a11y{};
  std::copy(std::begin(NVE_TABLE_ACCESSIBILITY_DEFAULTS), std::end(NVE_TABLE_ACCESSIBILITY_DEFAULTS), a11y.begin());
  b.patch(slot(NVE_TABLE_ACCESSIBILITY_KEY), 3u, set_field(a11y, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_IMAGE), a11y);
  b.set_string(slot(NVE_TABLE_ACCESSIBILITY_KEY), 3u, NVE_TABLE_ACCESSIBILITY_LABEL.word, "Brand logo");
  std::copy(std::begin(NVE_TABLE_ACCESSIBILITY_DEFAULTS), std::end(NVE_TABLE_ACCESSIBILITY_DEFAULTS), a11y.begin());
  b.patch(slot(NVE_TABLE_ACCESSIBILITY_KEY), 6u,
          set_field(a11y, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_PRESENTATION), a11y);
  std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> lay{};
  std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 3u, set_field(lay, NVE_TABLE_LAYOUT_WIDTH, 0x42000000u) | set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x42000000u), lay);
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 4u, set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x42C80000u), lay);  // 100pt
  std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 5u, set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x43C80000u) | set_field(lay, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0u), lay);
  std::array<uint32_t, NVE_TABLE_SCROLL_WORDS> scroll{};
  std::copy(std::begin(NVE_TABLE_SCROLL_DEFAULTS), std::end(NVE_TABLE_SCROLL_DEFAULTS), scroll.begin());
  b.patch(slot(NVE_TABLE_SCROLL_KEY), 4u, 0u, scroll);
  b.set_children(1u, {2u, 3u, 4u, 6u});
  b.set_children(4u, {5u});
  const std::vector<uint8_t> frame = b.finish();
  return [NSData dataWithBytes:frame.data() length:frame.size()];
}

/// Typography cascade scene. Keyframe: ROOT(1) -> parent(2, Typography 30pt) -> [mid(3) -> leaf(4),
/// recycled leaf `5 | 1 << 24`]; `revision` 2 changes the parent to 20pt and dirties `mid`'s
/// Visual (a dirty non-typography child between the change and `leaf`); revision 3 detaches the
/// parent's Typography.
constexpr uint32_t kRecycledLeaf = 5u | (1u << 24u);

NSData *typography_scene(uint64_t revision) {
  auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
  auto typography = [&](Nve2FrameBuilder &b, float size) {
    std::array<uint32_t, NVE_TABLE_TYPOGRAPHY_WORDS> row{};
    std::copy(std::begin(NVE_TABLE_TYPOGRAPHY_DEFAULTS), std::end(NVE_TABLE_TYPOGRAPHY_DEFAULTS), row.begin());
    uint32_t bits = 0u;
    std::memcpy(&bits, &size, sizeof bits);
    b.patch(slot(NVE_TABLE_TYPOGRAPHY_KEY), 2u,
            set_field(row, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, bits),
            row);
  };
  Nve2FrameBuilder b(revision);
  if (revision == 1u) {
    b.declare_core_tables().create_nodes({1u, 2u, 3u, 4u, kRecycledLeaf});
    b.set_string(slot(NVE_TABLE_TEXT_KEY), 4u, NVE_TABLE_TEXT_TEXT.word, "nested");
    b.set_string(slot(NVE_TABLE_TEXT_KEY), kRecycledLeaf, NVE_TABLE_TEXT_TEXT.word, "recycled");
    b.set_children(1u, {2u});
    b.set_children(2u, {3u, kRecycledLeaf});
    b.set_children(3u, {4u});
    typography(b, 30.0f);
  } else if (revision == 2u) {
    typography(b, 20.0f);
    std::array<uint32_t, NVE_TABLE_VISUAL_WORDS> visual{};
    std::copy(std::begin(NVE_TABLE_VISUAL_DEFAULTS), std::end(NVE_TABLE_VISUAL_DEFAULTS), visual.begin());
    b.patch(slot(NVE_TABLE_VISUAL_KEY), 3u, set_field(visual, NVE_TABLE_VISUAL_BACKGROUND, 0xFF0000FFu), visual);
  } else {
    b.detach(slot(NVE_TABLE_TYPOGRAPHY_KEY), {2u});
  }
  const std::vector<uint8_t> frame = b.finish();
  return [NSData dataWithBytes:frame.data() length:frame.size()];
}

/// Revision 2 of `core_table_scene`: only resource 9's bytes change (an 8px PNG); node 3 stays clean.
NSData *rewritten_image_scene() {
  Nve2FrameBuilder b(2u);
  b.define_resource(9u, nve::host::kImageSrcKind, nve::host::encode_image_url(png_data_uri(8)));
  const std::vector<uint8_t> frame = b.finish();
  return [NSData dataWithBytes:frame.data() length:frame.size()];
}

CGFloat font_size(RCTNveMacComponentView *view, uint32_t node) {
  NSView *v = [view nativeViewForNodeId:node];
  return [v isKindOfClass:[NSTextField class]] ? ((NSTextField *)v).font.pointSize : -1.0;
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
    CGSize rootSize = CGSizeZero;
    CHECK([view nativeViewForNodeId:1u] == nil);  // the root's style paints on the host view itself
    CHECK([view.nveHost getRootLayoutSize:&rootSize] && rootSize.width > 0.0 &&
          near(view.intrinsicContentSize.width, rootSize.width) && near(view.intrinsicContentSize.height, rootSize.height));

    // 2. Props + 3. press -> re-render + outbound custom event.
    CHECK([view applyPropsDictionary:@{@"title" : @"Hello props", @"ignored" : @[ @1, @{@"k" : NSNull.null} ]}]);
    CHECK([label_text(view, DemoApp::kTitle) isEqualToString:@"Hello props"]);
    // Inherited typography: only the card's row changes, its text descendants restyle too.
    CHECK([view applyPropsDictionary:@{@"accent" : @(0xFF00FF00u)}]);
    for (const uint32_t inherits : {DemoApp::kTitle, DemoApp::kLabel}) {
      NSColor *c = [((NSTextField *)[view nativeViewForNodeId:inherits]).textColor colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
      CHECK(c != nil && near(c.greenComponent, 1.0) && near(c.redComponent, 0.0));
    }
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
    CHECK(view.nveHost.lastReconciledMovedCount >= 1u);  // re-placed children without a content update

    // 4b. Calls made from inside reconcile (a frame-change observer) run on the next main-queue
    //     turn, and the deferred block never keeps a released host alive.
    card.postsFrameChangedNotifications = YES;
    __block BOOL deferredAccepted = NO;
    id observer = [NSNotificationCenter.defaultCenter addObserverForName:NSViewFrameDidChangeNotification object:card queue:nil
        usingBlock:^(NSNotification *) { deferredAccepted = [view applyPropsDictionary:@{@"title" : @"Deferred"}]; }];
    [view setFrameSize:NSMakeSize(500, 300)];
    [NSNotificationCenter.defaultCenter removeObserver:observer];
    CHECK(deferredAccepted && ![label_text(view, DemoApp::kTitle) isEqualToString:@"Deferred"]);
    [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
    CHECK([label_text(view, DemoApp::kTitle) isEqualToString:@"Deferred"]);
    __weak NveAppleComponentHost *released = nil;
    @autoreleasepool {
      RCTNveMacComponentView *transient = make_view();
      CHECK([transient attachNveInterface:&DemoApp::get_interface surfaceId:8u]);
      NSView *transientCard = [transient nativeViewForNodeId:DemoApp::kCard];
      transientCard.postsFrameChangedNotifications = YES;
      id o = [NSNotificationCenter.defaultCenter addObserverForName:NSViewFrameDidChangeNotification object:transientCard
          queue:nil usingBlock:^(NSNotification *) { [transient.nveHost dispatchPressOnNodeId:DemoApp::kButton x:0 y:0]; }];
      [transient setFrameSize:NSMakeSize(700, 300)];
      [NSNotificationCenter.defaultCenter removeObserver:o];
      released = transient.nveHost;
    }
    CHECK(released == nil);  // the pending press holds no strong reference
    [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];  // and runs as a no-op

    // 5. Load errors (the runtime loads app modules) keep the running app.
    NSError *err = nil;
    using nve::host::ModuleError;
    auto code = [](ModuleError e) { return static_cast<NSInteger>(e); };
    CHECK(![view loadNveDylibAtPath:@"" surfaceId:7u error:&err] && err.code == code(ModuleError::Open));
    CHECK(![view loadNveDylibAtPath:@"/nonexistent/libapp.dylib" surfaceId:7u error:&err] && err.code == code(ModuleError::Open));
    CHECK(![view loadNveDylibAtPath:@NVE_TEST_NO_ENTRY_DYLIB surfaceId:7u error:&err] && err.code == code(ModuleError::MissingEntry));
    CHECK(![view loadNveDylibAtPath:@NVE_TEST_NO_SURFACE_DYLIB surfaceId:7u error:&err] && err.code == code(ModuleError::MissingSurface));
    CHECK([err.localizedDescription containsString:@"Loading"]);
    CHECK([label_text(view, DemoApp::kLabel) isEqualToString:@"Clicks: 2"]);
    RCTNveMacComponentView *failing = make_view();
    CHECK(![failing loadNveDylibAtPath:@NVE_TEST_CREATE_FAIL_DYLIB surfaceId:1u error:&err] && err.code == code(ModuleError::Attach));
    CHECK(!failing.nveHost.isAppAttached);

    // 6. Tier-2 reload keeps state; a failed reload keeps the live app.
    RCTNveMacComponentView *dylibView = make_view();
    CHECK([dylibView loadNveDylibAtPath:@NVE_TEST_STATEFUL_DYLIB surfaceId:3u error:&err]);
    [dylibView simulatePressOnNodeId:DemoApp::kButton atPoint:CGPointZero];
    CHECK([dylibView reloadNveDylibAtPath:@NVE_TEST_STATEFUL_DYLIB error:&err]);
    CHECK([label_text(dylibView, DemoApp::kLabel) isEqualToString:@"Clicks: 1"]);
    CHECK(![dylibView reloadNveDylibAtPath:@NVE_TEST_NO_SURFACE_DYLIB error:&err]);
    [dylibView simulatePressOnNodeId:DemoApp::kButton atPoint:CGPointZero];
    CHECK([label_text(dylibView, DemoApp::kLabel) isEqualToString:@"Clicks: 2"]);

    // 7. Core tables: secure text input, image with a11y, scroll document sizing.
    RCTNveMacComponentView *scene = make_view();
    CHECK([scene.nveHost submitNve2Frame:core_table_scene()]);
    NSView *input = [scene nativeViewForNodeId:2u];
    CHECK([input isKindOfClass:[NSSecureTextField class]]);
    CHECK([((NSTextField *)input).stringValue isEqualToString:@"hunter2"]);
    CHECK([((NSTextField *)input).placeholderString isEqualToString:@"Password"]);
    NSView *image = [scene nativeViewForNodeId:3u];
    CHECK([image isKindOfClass:[NveAppleImageView class]] && ((NveAppleImageView *)image).image != nil);
    NSColor *tint = [((NveAppleImageView *)image).contentTintColor colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    CHECK(tint != nil && near(tint.greenComponent, 1.0) && near(tint.redComponent, 0.0));
    CHECK([image.accessibilityLabel isEqualToString:@"Brand logo"] && [image.accessibilityRole isEqualToString:NSAccessibilityImageRole]);
    CHECK(![scene nativeViewForNodeId:6u].accessibilityElement);  // `NVE_ARIA_ROLE_PRESENTATION` -> non-element
    NSView *scroller = [scene nativeViewForNodeId:4u];
    CHECK([scroller isKindOfClass:[NSScrollView class]]);
    CHECK([scene nativeViewForNodeId:5u].superview == ((NSScrollView *)scroller).documentView);
    CHECK(near(((NSScrollView *)scroller).documentView.frame.size.height, 400.0));
    // A clean image node whose resource bytes were rewritten re-decodes; nothing else updates.
    NSImage *before4 = ((NveAppleImageView *)image).image;
    CHECK(near(before4.size.width, 4.0));
    CHECK([scene.nveHost submitNve2Frame:rewritten_image_scene()]);
    CHECK([scene nativeViewForNodeId:3u] == image && near(((NveAppleImageView *)image).image.size.width, 8.0));
    CHECK(scene.nveLastReconciledDirtyCount == 1u && scene.nveLastReconciledRemovedCount == 0u);

    // 7'. Inline images decode once per resource, re-decode on tag-1 rewrite, and clear their
    //     cache in `clearNativeNodeViews`; a still-live detached child leaves its parent; scroll
    //     content follows a resized child and its moved sibling.
    {
      RCTNveMacComponentView *det = make_view();
      __weak NSImage *weakImg8 = nil;
      @autoreleasepool {
        CHECK([det.nveHost submitNve2Frame:detach_scene(1u)]);
        NveAppleImageView *a = (NveAppleImageView *)[det nativeViewForNodeId:2u];
        NveAppleImageView *b = (NveAppleImageView *)[det nativeViewForNodeId:3u];
        CHECK([a isKindOfClass:[NveAppleImageView class]] && [b isKindOfClass:[NveAppleImageView class]]);
        CHECK(a.image != nil && a.image == b.image);  // one decode shared by both nodes
        NSScrollView *sv = (NSScrollView *)[det nativeViewForNodeId:4u];
        CHECK([sv isKindOfClass:[NSScrollView class]] && near(sv.documentView.frame.size.height, 600.0));
        NSView *parent = b.superview;
        CHECK(parent != nil && [parent.subviews containsObject:b]);
        CHECK([det.nveHost submitNve2Frame:detach_scene(2u)]);
        CHECK([det nativeViewForNodeId:3u] == b);  // still live, so still realized ...
        CHECK(b.superview == nil && ![parent.subviews containsObject:b]);  // ... but detached
        CHECK(a.superview == parent && sv.superview == parent);
        CHECK(near([det nativeViewForNodeId:6u].frame.origin.y, 100.0));
        CHECK(near(sv.documentView.frame.size.height, 300.0));

        // Rewriting tag-1 inline image under the same resource id 9 evicts cache and re-decodes.
        NSImage *img4 = a.image;
        CHECK(near(img4.size.width, 4.0));
        CHECK([det.nveHost submitNve2Frame:detach_scene(3u)]);
        CHECK([det nativeViewForNodeId:2u] == a);
        CHECK(a.image != nil && a.image != img4 && near(a.image.size.width, 8.0));
        weakImg8 = a.image;
        [det.nveHost clearNativeNodeViews];
      }
      CHECK([det nativeViewForNodeId:2u] == nil && [det nativeViewForNodeId:3u] == nil);
      CHECK(weakImg8 == nil);  // `det` is still alive; `_adapter.inline_images` released its NSImage on clear
    }

    // 7''. Deferred `z_index` batch restacking updates sibling order in `finish_sync` without a
    //      `set_children` change on the parent.
    {
      RCTNveMacComponentView *zview = make_view();
      CHECK([zview.nveHost submitNve2Frame:z_order_scene(1u)]);
      NSView *box = [zview nativeViewForNodeId:2u];
      NSView *n3 = [zview nativeViewForNodeId:3u];
      NSView *n4 = [zview nativeViewForNodeId:4u];
      NSView *n5 = [zview nativeViewForNodeId:5u];
      CHECK(box != nil && [box.subviews isEqualToArray:(@[ n3, n4, n5 ])]);
      CHECK([zview.nveHost submitNve2Frame:z_order_scene(2u)]);
      CHECK([box.subviews isEqualToArray:(@[ n5, n4, n3 ])]);
      CHECK([zview.nveHost submitNve2Frame:z_order_scene(3u)]);
      CHECK([box.subviews isEqualToArray:(@[ n5, n3, n4 ])]);
    }

    // 7a. CSS Text 3 min-content parity across `Word`, `BreakWord`, `Anywhere`, `BreakAll`, and
    //     `None` (`white-space: nowrap | pre` never collapses to single words).
    {
      NveRuntimeTextStyle base{};
      base.font_size = 14.0f;
      base.font_weight = 400u;
      const auto measure = [](std::string_view sv, const NveRuntimeTextStyle &st, float max_w) {
        float w = 0.0f, h = 0.0f;
        nve::apple::native_text_measure(nullptr, NveStrSlice{reinterpret_cast<const uint8_t *>(sv.data()), sv.size()},
                                        &st, max_w, &w, &h);
        return std::pair{w, h};
      };
      constexpr std::string_view kPhrase = "considerably long";
      const auto [lw, lh] = measure(kPhrase, base, INFINITY);
      const auto [ww, wh] = measure("considerably", base, INFINITY);
      const auto [mw_word, mh_word] = measure(kPhrase, base, 0.0f);
      CHECK(mw_word > 0.0f && mw_word < lw && mh_word > lh && std::fabs(mw_word - ww) <= 1.0f);

      NveRuntimeTextStyle break_word = base;
      break_word.overflow_wrap = NVE_OVERFLOW_WRAP_BREAK_WORD;
      const auto [mw_bw, mh_bw] = measure(kPhrase, break_word, 0.0f);
      CHECK(std::fabs(mw_bw - ww) <= 1.0f && mh_bw > lh);

      NveRuntimeTextStyle anywhere = base;
      anywhere.overflow_wrap = NVE_OVERFLOW_WRAP_ANYWHERE;
      const auto [mw_any, mh_any] = measure(kPhrase, anywhere, 0.0f);
      CHECK(mw_any > 0.0f && mw_any < ww && mh_any > mh_word);

      NveRuntimeTextStyle wb_word = base;
      wb_word.word_break = NVE_WORD_BREAK_BREAK_WORD;
      const auto [mw_wbw, mh_wbw] = measure(kPhrase, wb_word, 0.0f);
      CHECK(mw_wbw > 0.0f && mw_wbw < ww && mh_wbw > mh_word);

      NveRuntimeTextStyle break_all = base;
      break_all.word_break = NVE_WORD_BREAK_BREAK_ALL;
      const auto [mw_ba, mh_ba] = measure(kPhrase, break_all, 0.0f);
      CHECK(mw_ba > 0.0f && mw_ba < ww && mh_ba > mh_word);

      // Whitespace-only min-content parity: `Anywhere` and `BreakAll` skip whitespace clusters
      // just like `Word`/`BreakWord` (falling back to unconstrained layout instead of space-width wrapping).
      const auto [ws_word_w, ws_word_h] = measure("   ", base, 0.0f);
      const auto [ws_any_w, ws_any_h] = measure("   ", anywhere, 0.0f);
      const auto [ws_ba_w, ws_ba_h] = measure("   ", break_all, 0.0f);
      CHECK(near(ws_any_w, ws_word_w) && near(ws_any_h, ws_word_h));
      CHECK(near(ws_ba_w, ws_word_w) && near(ws_ba_h, ws_word_h));

      // Long repeated-character string exercises per-call cluster deduplication.
      const std::string repeated = std::string(1000u, 'i') + "W";
      const auto [rep_w, rep_h] = measure(repeated, anywhere, 0.0f);
      const auto [single_w, single_h] = measure("W", anywhere, INFINITY);
      CHECK(std::fabs(rep_w - single_w) <= 1.0f && rep_h > single_h);

      for (const uint8_t ws : {uint8_t{NVE_WHITE_SPACE_NO_WRAP}, uint8_t{NVE_WHITE_SPACE_PRE}}) {
        NveRuntimeTextStyle nw = base;
        nw.white_space = ws;
        const auto [nw0, nh0] = measure(kPhrase, nw, 0.0f);
        const auto [nw40, nh40] = measure(kPhrase, nw, 40.0f);
        CHECK(near(nw0, lw) && near(nh0, lh) && near(nw40, lw) && near(nh40, lh));
      }
      (void)wh;
    }

    // 7b. Inherited typography cascades through dirty non-typography children to a recycled
    //     (generation 1) child, and clearing an ancestor's Typography restyles the subtree.
    RCTNveMacComponentView *typo = make_view();
    CHECK([typo.nveHost submitNve2Frame:typography_scene(1u)]);
    CHECK(near(font_size(typo, 4u), 30.0) && near(font_size(typo, kRecycledLeaf), 30.0));
    CHECK([typo.nveHost submitNve2Frame:typography_scene(2u)]);
    CHECK(near(font_size(typo, 4u), 20.0));             // through the dirty `mid`
    CHECK(near(font_size(typo, kRecycledLeaf), 20.0));  // raw NodeId resolved to its slot
    CHECK([typo.nveHost submitNve2Frame:typography_scene(3u)]);
    CHECK(font_size(typo, 4u) > 0.0 && !near(font_size(typo, 4u), 20.0));
    CHECK(font_size(typo, kRecycledLeaf) > 0.0 && !near(font_size(typo, kRecycledLeaf), 20.0));

    // 7c. Foundation values: mixed-type dictionary keys are skipped (never compared), nesting is
    //     depth-limited per level.
    CHECK([view applyPropsDictionary:(NSDictionary *)@{@"title" : @"Mixed keys", @1 : @"numeric key", NSNull.null : @2}]);
    CHECK([label_text(view, DemoApp::kTitle) isEqualToString:@"Mixed keys"]);
    {
      nve::apple::ValueArena arena;
      id deep = @"leaf";
      for (int i = 0; i < 40; ++i) deep = @{@"k" : deep};
      const NveValue v = nve::apple::nve_value_from_objc_object(deep, arena);
      uint32_t levels = 0u;
      for (const NveValue *p = &v; p->tag == NVE_VAL_MAP && p->data.map_val.len == 1u; p = &p->data.map_val.ptr[0].value) {
        ++levels;
      }
      CHECK(levels == 33u);  // depth 0..32 are maps; the 34th level is cut to null
      id list = @"leaf";
      for (int i = 0; i < 40; ++i) list = @[ list ];
      const NveValue a = nve::apple::nve_value_from_objc_object(list, arena);
      levels = 0u;
      for (const NveValue *p = &a; p->tag == NVE_VAL_ARRAY && p->data.array_val.len == 1u; p = &p->data.array_val.ptr[0]) {
        ++levels;
      }
      CHECK(levels == 33u);  // arrays and dictionaries count one level each
    }

    // 7d. Track 1B-1d uniform 64-byte `NveEvent` bridge: interaction, text, scroll, key, and custom `NveBinValue`.
    {
      nve::host::OwnedEvents evs;
      CHECK(evs.push_text_change(7u, 2u, 100u, "hello"));
      CHECK(evs.push_scroll(7u, 4u, 200u, 10.0f, 20.0f, 300.0f, 400.0f));
      CHECK(evs.push_key(7u, 2u, 300u, NVE_INTERACTION_EVENT_KEY_DOWN, "Enter", "Enter", 0u, 13u));
      nve::host::BinWriter bin;
      const auto m = bin.begin_map(1u);
      bin.write_map_key("msg");
      bin.write_str("ok");
      bin.end(m);
      CHECK(evs.push_custom(7u, 3u, 400u, "customPing", bin.bytes()));
      const NveEventBatch batch = evs.batch();
      const auto span = nve::host::batch_events(batch);
      CHECK(span.size() == 4u);
      CHECK([nve::apple::event_name(span[0], batch) isEqualToString:@"textChange"]);
      CHECK([nve::apple::payload_dictionary_from_event(span[0], batch)[@"text"] isEqualToString:@"hello"]);
      CHECK([nve::apple::event_name(span[1], batch) isEqualToString:@"scroll"]);
      CHECK([nve::apple::payload_dictionary_from_event(span[1], batch)[@"height"] isEqual:@400.0f]);
      CHECK([nve::apple::event_name(span[2], batch) isEqualToString:@"keyDown"]);
      CHECK([nve::apple::payload_dictionary_from_event(span[2], batch)[@"key"] isEqualToString:@"Enter"]);
      CHECK([nve::apple::event_name(span[3], batch) isEqualToString:@"customPing"]);
      CHECK([nve::apple::payload_dictionary_from_event(span[3], batch)[@"value"][@"msg"] isEqualToString:@"ok"]);
    }

    // 7e. FlatResource kind validation rejects `nve.res.draw_list` payloads on `nve.image` lookups
    //     (including the colliding `[1, 1, 0]` wire vector and valid `ImageSrc` bytes under the wrong
    //     kind), `NveAppleSceneReconciler` passes `kImageSrcKind`, and empty `EncodedBytes` (`[1, 0]`)
    //     never routes through the URL cache.
    {
      const Key64 kDrawListKind = Key64::of("nve.res.draw_list");
      // `[1, 1, 0]` = valid DrawList([Clear(0)]) and valid ImageSrc::EncodedBytes([0]).
      const std::vector<uint8_t> colliding = {1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0};
      const std::vector<uint8_t> valid_png_src = nve::host::encode_image_url(png_data_uri(4));
      auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
      auto make_img_scene = [&](Key64 kind, std::vector<uint8_t> payload, bool with_draw_holders) -> NSData * {
        Nve2FrameBuilder b(1u);
        b.declare_core_tables();
        constexpr uint8_t kHolderSlot = 30u;
        const std::array<uint32_t, 1> holder_def = {0u};
        const std::array<uint8_t, 1> holder_fx = {0u};
        if (with_draw_holders) {
          b.declare({Key64::of("test.draw_holder"), Key64::of("test.draw_holder.v1"), kHolderSlot, true, 0u,
                     holder_def, holder_fx, 0u, 1u, "test.draw_holder"});
          b.create_nodes({1u, 2u, 3u, 4u});
        } else {
          b.create_nodes({1u, 2u});
        }
        b.define_resource(9u, kind, payload);
        std::array<uint32_t, NVE_TABLE_IMAGE_WORDS> img{};
        std::copy(std::begin(NVE_TABLE_IMAGE_DEFAULTS), std::end(NVE_TABLE_IMAGE_DEFAULTS), img.begin());
        b.patch(slot(NVE_TABLE_IMAGE_KEY), 2u, set_field(img, NVE_TABLE_IMAGE_SRC, 9u), img);
        if (with_draw_holders) {
          b.define_resource(10u, kDrawListKind, colliding);
          b.define_resource(11u, kDrawListKind, valid_png_src);
          const std::array<uint32_t, 1> r10 = {10u}, r11 = {11u};
          b.patch(kHolderSlot, 3u, 1u, r10);
          b.patch(kHolderSlot, 4u, 1u, r11);
          b.set_children(1u, {2u, 3u, 4u});
        } else {
          b.set_children(1u, {2u});
        }
        const std::vector<uint8_t> frame = b.finish();
        return [NSData dataWithBytes:frame.data() length:frame.size()];
      };
      // Rust `WireDecoder` rejects binding a `kDrawListKind` resource directly to `nve.image.src`.
      RCTNveMacComponentView *v_wire = make_view();
      CHECK(![v_wire.nveHost submitNve2Frame:make_img_scene(kDrawListKind, colliding, false)]);
      // Host-side lookup enforces `expectedKind` on live runtime resources (colliding `[1, 1, 0]` and PNG bytes).
      RCTNveMacComponentView *v1 = make_view();
      CHECK([v1.nveHost submitNve2Frame:make_img_scene(nve::host::kImageSrcKind, valid_png_src, true)]);
      CHECK(((NveAppleImageView *)[v1 nativeViewForNodeId:2u]).image != nil);
      CHECK([v1.nveHost resourceBytesForId:9u expectedKind:nve::host::kImageSrcKind.value] != nil);
      CHECK([v1.nveHost resourceBytesForId:9u expectedKind:kDrawListKind.value] == nil);
      NSData *draw10 = [v1.nveHost resourceBytesForId:10u expectedKind:kDrawListKind.value];
      CHECK(draw10 != nil && draw10.length == colliding.size());
      CHECK([v1.nveHost resourceBytesForId:10u expectedKind:nve::host::kImageSrcKind.value] == nil);
      CHECK([v1.nveHost resourceBytesForId:11u expectedKind:kDrawListKind.value] != nil);
      CHECK([v1.nveHost resourceBytesForId:11u expectedKind:nve::host::kImageSrcKind.value] == nil);
      // Direct `NveAppleSceneReconciler` check: requests `kImageSrcKind`, rejects mismatched kind.
      {
        auto sess = nve::host::RuntimeSession::create(nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION));
        CHECK(sess.has_value());
        NSData *f = make_img_scene(nve::host::kImageSrcKind, valid_png_src, false);
        CHECK(sess->submit_nve2(std::span<const uint8_t>(static_cast<const uint8_t *>(f.bytes), f.length)) == NVE_RUNTIME_OK);
        __block uint64_t seen_kind = 0u;
        RCTNveMacComponentView *adapter = make_view();
        NveAppleSceneReconciler *rec = [[NveAppleSceneReconciler alloc]
            initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                  resources:^NSData *(uint32_t rid, uint64_t expectedKind) {
                    seen_kind = expectedKind;
                    // Pretend resource 9 had kind `kDrawListKind`: provider must reject `kImageSrcKind`.
                    if (rid != 9u || expectedKind != kDrawListKind.value) return nil;
                    return [NSData dataWithBytes:valid_png_src.data() length:valid_png_src.size()];
                  }];
        auto guard = sess->acquire_scene();
        CHECK(guard.has_value());
        [rec syncFromScene:guard->reader()];
        CHECK(seen_kind == nve::host::kImageSrcKind.value);
        NveAppleImageView *iv = (NveAppleImageView *)[rec nativeViewForNodeId:2u];
        CHECK([iv isKindOfClass:[NveAppleImageView class]] && iv.image == nil);
      }
      // Empty `EncodedBytes` (`[1, 0]`) under `kImageSrcKind` yields nil and never hits URL cache.
      RCTNveMacComponentView *v4 = make_view();
      CHECK([v4.nveHost submitNve2Frame:make_img_scene(nve::host::kImageSrcKind, nve::host::encode_image_bytes({}), false)]);
      CHECK(((NveAppleImageView *)[v4 nativeViewForNodeId:2u]).image == nil);
    }

    // 8. Pairing errors surface the runtime's diagnostics.
    CHECK(![scene connectPairedEndpoint:@"" token:@"" surfaceId:1u error:&err] && err.localizedDescription.length > 0);
    CHECK(!scene.isPaired);

    // 9. `nve.canvas`: CoreGraphics 8-opcode pixel verification, clip overflow balancing (`kMaxClipDepth = 64`),
    //    malformed rejection, live `NveAppleCanvasView` mounting, and dirty-resource in-place redraw.
    {
      using namespace nve::canvas;
      constexpr int kW = 64;
      constexpr int kH = 64;
      auto make_bitmap = [&](std::vector<uint8_t> &buf) -> CGContextRef {
        buf.assign(static_cast<size_t>(kW * kH * 4), 0u);
        CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef ctx = CGBitmapContextCreate(
            buf.data(), kW, kH, 8, kW * 4, cs,
            static_cast<uint32_t>(kCGImageAlphaPremultipliedLast) | static_cast<uint32_t>(kCGBitmapByteOrder32Big));
        CGColorSpaceRelease(cs);
        // Flip top-down so pixel (x, y) matches NSView.isFlipped / UIView coordinates.
        CGContextTranslateCTM(ctx, 0.0, kH);
        CGContextScaleCTM(ctx, 1.0, -1.0);
        return ctx;
      };
      auto pixel = [](const std::vector<uint8_t> &buf, int x, int y) -> std::array<uint8_t, 4> {
        const size_t off = static_cast<size_t>((y * kW + x) * 4);
        return {buf[off], buf[off + 1], buf[off + 2], buf[off + 3]};
      };

      // 9a. All 8 opcodes rendered cleanly on offscreen CGBitmapContext.
      std::vector<uint8_t> pixels;
      CGContextRef ctx = make_bitmap(pixels);
      const DrawPoint poly[] = {{4.0f, 54.0f}, {20.0f, 54.0f}, {20.0f, 60.0f}};
      DrawListEncoder enc;
      enc.clear(DrawColorRgba::rgb(10, 20, 30))
          .fill_rect(2.0f, 2.0f, 12.0f, 12.0f, DrawColorRgba::rgb(255, 0, 0))
          .fill_rounded_rect(20.0f, 2.0f, 16.0f, 16.0f, 4.0f, DrawColorRgba::rgb(0, 255, 0))
          .fill_circle(48.0f, 10.0f, 8.0f, DrawColorRgba::rgb(0, 0, 255))
          .stroke_line(4.0f, 30.0f, 28.0f, 30.0f, 4.0f, DrawColorRgba::rgb(255, 255, 0))
          .stroke_polyline(poly, 4.0f, DrawColorRgba::rgb(0, 255, 255))
          .clip_rect(36.0f, 36.0f, 12.0f, 12.0f)
          .fill_rect(32.0f, 32.0f, 24.0f, 24.0f, DrawColorRgba::rgb(255, 0, 255))
          .restore_clip();
      const std::vector<uint8_t> dl = enc.finish();
      const DrawListDecodeResult r_ok =
          nve::apple::render_draw_list_to_cgcontext(ctx, CGRectMake(0, 0, kW, kH), dl);
      CGContextRelease(ctx);
      CHECK(r_ok.ok() && r_ok.ops_decoded == 9u);
      CHECK(pixel(pixels, 0, 0)[0] == 10 && pixel(pixels, 0, 0)[1] == 20 && pixel(pixels, 0, 0)[2] == 30);
      CHECK(pixel(pixels, 8, 8)[0] > 240 && pixel(pixels, 8, 8)[1] < 10);       // FillRect red
      CHECK(pixel(pixels, 28, 10)[1] > 240 && pixel(pixels, 28, 10)[0] < 10);   // FillRoundedRect green
      CHECK(pixel(pixels, 48, 10)[2] > 240 && pixel(pixels, 48, 10)[0] < 10);   // FillCircle blue
      CHECK(pixel(pixels, 16, 30)[0] > 240 && pixel(pixels, 16, 30)[1] > 240);  // StrokeLine yellow
      CHECK(pixel(pixels, 12, 54)[1] > 240 && pixel(pixels, 12, 54)[2] > 240);  // StrokePolyline cyan
      CHECK(pixel(pixels, 42, 42)[0] > 240 && pixel(pixels, 42, 42)[2] > 240);  // Inside clip: magenta
      CHECK(pixel(pixels, 33, 33)[0] == 10 && pixel(pixels, 33, 33)[2] == 30);  // Outside clip: untouched clear

      // 9b. `kMaxClipDepth = 64` overflow balancing & RAII unwind: 66 clips + 2 restores stay clipped,
      //     and caller's CGContext is cleanly restored afterwards.
      {
        std::vector<uint8_t> clipBuf;
        CGContextRef cctx = make_bitmap(clipBuf);
        DrawListEncoder deep;
        deep.clear(DrawColorRgba::rgb(0, 0, 0));
        for (uint32_t i = 0u; i < kMaxClipDepth + 2u; ++i) {
          deep.clip_rect(10.0f, 10.0f, 10.0f, 10.0f);
        }
        deep.restore_clip().restore_clip();  // pops only the 2 overflowed clips; depth stays at 64
        deep.fill_rect(0.0f, 0.0f, 64.0f, 64.0f, DrawColorRgba::rgb(255, 255, 255));
        const std::vector<uint8_t> deepBytes = deep.finish();
        const DrawListDecodeResult r_deep =
            nve::apple::render_draw_list_to_cgcontext(cctx, CGRectMake(0, 0, kW, kH), deepBytes);
        CHECK(r_deep.ok());
        // Verify caller CGContext is NOT stuck clipped after RAII unwind: draw a red dot at (2, 2).
        CGContextSetRGBFillColor(cctx, 1.0, 0.0, 0.0, 1.0);
        CGContextFillRect(cctx, CGRectMake(0.0, 0.0, 4.0, 4.0));
        CGContextRelease(cctx);
        CHECK(pixel(clipBuf, 15, 15)[0] == 255);  // inside clip
        CHECK(pixel(clipBuf, 40, 40)[0] == 0);    // clipped out during draw list
        CHECK(pixel(clipBuf, 2, 2)[0] > 240 && pixel(clipBuf, 2, 2)[1] < 10);  // caller clip restored
      }

      // 9c. Malformed draw list triggers zero CG calls and returns exact error.
      {
        std::vector<uint8_t> badBuf;
        CGContextRef bctx = make_bitmap(badBuf);
        const std::vector<uint8_t> truncated = {1, 0, 0, 0, 1, 0, 0, 0};  // Clear missing color word
        const DrawListDecodeResult r_bad =
            nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, kW, kH), truncated);
        CGContextRelease(bctx);
        CHECK(!r_bad.ok() && r_bad.error == DrawListError::Truncated);
        CHECK(pixel(badBuf, 0, 0)[3] == 0u);  // untouched
      }

      // 9d. Live `nve.canvas` table mounting, byte-equality dedup, and dirty-resource in-place redraw.
      {
        constexpr uint8_t kCanvasSlot = 28u;
        const std::array<uint32_t, NVE_TABLE_CANVAS_WORDS> cdef(
            {NVE_TABLE_CANVAS_DEFAULTS[0], NVE_TABLE_CANVAS_DEFAULTS[1]});
        const std::array<uint8_t, NVE_TABLE_CANVAS_WORDS> cfx(
            {NVE_TABLE_CANVAS_WORD_EFFECTS[0], NVE_TABLE_CANVAS_WORD_EFFECTS[1]});

        DrawListEncoder dl1, dl2;
        dl1.clear(DrawColorRgba::rgb(255, 0, 0));
        dl2.clear(DrawColorRgba::rgb(0, 255, 0));
        const std::vector<uint8_t> bytes1 = dl1.finish();
        const std::vector<uint8_t> bytes2 = dl2.finish();

        Nve2FrameBuilder f1(1u);
        f1.declare_core_tables();
        f1.declare({Key64{NVE_TABLE_CANVAS_KEY}, Key64{NVE_TABLE_CANVAS_SCHEMA_HASH}, kCanvasSlot, true,
                    static_cast<uint8_t>(NVE_TABLE_CANVAS_FLAGS), cdef, cfx,
                    NVE_TABLE_CANVAS_STR_WORDS, NVE_TABLE_CANVAS_RESOURCE_WORDS, NVE_TABLE_CANVAS_NAME});
        f1.create_nodes({1u, 2u});
        f1.define_resource(42u, kDrawListKind, bytes1);
        std::array<uint32_t, NVE_TABLE_CANVAS_WORDS> crow = cdef;
        f1.patch(kCanvasSlot, 2u,
                 set_field(crow, NVE_TABLE_CANVAS_DRAW_LIST, 42u) | set_field(crow, NVE_TABLE_CANVAS_GENERATION, 1u),
                 crow);
        f1.set_children(1u, {2u});
        const std::vector<uint8_t> raw1 = f1.finish();

        RCTNveMacComponentView *cvHost = make_view();
        CHECK([cvHost.nveHost submitNve2Frame:[NSData dataWithBytes:raw1.data() length:raw1.size()]]);
        NSView *mounted = [cvHost nativeViewForNodeId:2u];
        CHECK([mounted isKindOfClass:[NveAppleCanvasView class]]);
        if ([mounted isKindOfClass:[NveAppleCanvasView class]]) {
          NveAppleCanvasView *canvasView = (NveAppleCanvasView *)mounted;
          CHECK(canvasView.redrawCount == 1u && canvasView.drawListData.length == bytes1.size());

          // Frame 2: touch unrelated visual opacity on node 2 -> bytes unchanged, zero spurious redraws.
          Nve2FrameBuilder f2(2u);
          std::array<uint32_t, NVE_TABLE_VISUAL_WORDS> vis{};
          std::copy(std::begin(NVE_TABLE_VISUAL_DEFAULTS), std::end(NVE_TABLE_VISUAL_DEFAULTS), vis.begin());
          uint32_t op_bits = 0u;
          const float half = 0.75f;
          std::memcpy(&op_bits, &half, sizeof op_bits);
          f2.patch(Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_VISUAL_KEY}), 2u,
                   set_field(vis, NVE_TABLE_VISUAL_OPACITY, op_bits), vis);
          const std::vector<uint8_t> raw2 = f2.finish();
          CHECK([cvHost.nveHost submitNve2Frame:[NSData dataWithBytes:raw2.data() length:raw2.size()]]);
          CHECK(canvasView.redrawCount == 1u);

          // Frame 3: rewrite resource 42 in-place while leaving node 2 clean -> RetainedTree dirty-resource redraw.
          Nve2FrameBuilder f3(3u);
          f3.define_resource(42u, kDrawListKind, bytes2);
          const std::vector<uint8_t> raw3 = f3.finish();
          CHECK([cvHost.nveHost submitNve2Frame:[NSData dataWithBytes:raw3.data() length:raw3.size()]]);
          CHECK([cvHost nativeViewForNodeId:2u] == canvasView);
          CHECK(canvasView.redrawCount == 2u && canvasView.drawListData.length == bytes2.size());
          CHECK(std::memcmp(canvasView.drawListData.bytes, bytes2.data(), bytes2.size()) == 0);
        }
      }
    }

    // 10. Shared typography parity across `native_text_measure` and mounted AppKit labels:
    //     `font_family`, `letter_spacing`, `line_height`, `max_lines`, `text_decoration`.
    {
      const auto measure = [](std::string_view sv, const NveRuntimeTextStyle &st, float max_w) {
        float w = 0.0f, h = 0.0f;
        nve::apple::native_text_measure(nullptr, NveStrSlice{reinterpret_cast<const uint8_t *>(sv.data()), sv.size()},
                                        &st, max_w, &w, &h);
        return std::pair{w, h};
      };
      // CSS font-family fallback list skips nonexistent family and resolves Menlo.
      NSFont *menlo = nve::apple::platform_font(15.0f, 400u, false, "NonexistentNveFont, Menlo, monospace");
      CHECK([menlo.familyName isEqualToString:@"Menlo"]);

      // Letter spacing increases width monotonically.
      NveRuntimeTextStyle tight{};
      tight.font_size = 16.0f;
      tight.font_weight = 400u;
      NveRuntimeTextStyle wide = tight;
      wide.letter_spacing = 6.0f;
      const auto [tw, th] = measure("Typography", tight, INFINITY);
      const auto [ww, wh] = measure("Typography", wide, INFINITY);
      CHECK(ww > tw + 30.0f && th > 0.0f && wh > 0.0f);

      // Line height (points & percent) increases multi-line height; max_lines caps it.
      NveRuntimeTextStyle baseLH{};
      baseLH.font_size = 14.0f;
      baseLH.font_weight = 400u;
      NveRuntimeTextStyle ptsLH = baseLH;
      ptsLH.line_height_unit = NVE_LINE_HEIGHT_POINTS;
      ptsLH.line_height = 40.0f;
      NveRuntimeTextStyle pctLH = baseLH;
      pctLH.line_height_unit = NVE_LINE_HEIGHT_PERCENT;
      pctLH.line_height = 250.0f;
      NveRuntimeTextStyle capLH = baseLH;
      capLH.max_lines = 1u;
      constexpr std::string_view kMulti = "Line one\nLine two\nLine three";
      const auto [_, bh] = measure(kMulti, baseLH, INFINITY);
      const auto [_1, ph] = measure(kMulti, ptsLH, INFINITY);
      const auto [_2, pch] = measure(kMulti, pctLH, INFINITY);
      const auto [_3, ch] = measure(kMulti, capLH, INFINITY);
      (void)_;
      (void)_1;
      (void)_2;
      (void)_3;
      CHECK(ph > bh + 40.0f && pch > bh + 20.0f && ch > 0.0f && ch < bh);

      // Live mounted NSTextField carries matching family, kern, line height, max lines, and decorations.
      auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
      Nve2FrameBuilder tb(1u);
      tb.declare_core_tables().create_nodes({1u, 2u});
      tb.set_string(slot(NVE_TABLE_TEXT_KEY), 2u, NVE_TABLE_TEXT_TEXT.word, "Styled label");
      std::array<uint32_t, NVE_TABLE_TYPOGRAPHY_WORDS> trow{};
      std::copy(std::begin(NVE_TABLE_TYPOGRAPHY_DEFAULTS), std::end(NVE_TABLE_TYPOGRAPHY_DEFAULTS), trow.begin());
      uint32_t sz_bits = 0u, ls_bits = 0u, lh_bits = 0u;
      const float sz = 16.0f, ls = 4.0f, lh = 36.0f;
      std::memcpy(&sz_bits, &sz, sizeof sz_bits);
      std::memcpy(&ls_bits, &ls, sizeof ls_bits);
      std::memcpy(&lh_bits, &lh, sizeof lh_bits);
      const uint32_t mask =
          set_field(trow, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, sz_bits) |
          set_field(trow, NVE_TABLE_TYPOGRAPHY_LETTER_SPACING, ls_bits) |
          set_field(trow, NVE_TABLE_TYPOGRAPHY_LINE_HEIGHT, (lh_bits & ~3u) | static_cast<uint32_t>(NVE_LENGTH_POINTS)) |
          set_field(trow, NVE_TABLE_TYPOGRAPHY_MAX_LINES, 2u) |
          set_field(trow, NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION,
                    NVE_TEXT_DECORATION_UNDERLINE | NVE_TEXT_DECORATION_LINE_THROUGH);
      trow[0] |= uint32_t{1} << NVE_TABLE_TYPOGRAPHY_FONT_FAMILY.explicit_bit;
      tb.patch(slot(NVE_TABLE_TYPOGRAPHY_KEY), 2u, mask | 1u, trow);
      tb.set_string(slot(NVE_TABLE_TYPOGRAPHY_KEY), 2u, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY.word,
                    "NonexistentNveFont, Menlo, monospace");
      tb.set_children(1u, {2u});
      const std::vector<uint8_t> tframe = tb.finish();
      RCTNveMacComponentView *tview = make_view();
      CHECK([tview.nveHost submitNve2Frame:[NSData dataWithBytes:tframe.data() length:tframe.size()]]);
      NSTextField *lbl = (NSTextField *)[tview nativeViewForNodeId:2u];
      CHECK([lbl isKindOfClass:[NSTextField class]]);
      CHECK([lbl.font.familyName isEqualToString:@"Menlo"]);
      CHECK(lbl.maximumNumberOfLines == 2);
      NSAttributedString *as = lbl.attributedStringValue;
      CHECK(as.length > 0u);
      NSDictionary<NSAttributedStringKey, id> *attrs = [as attributesAtIndex:0 effectiveRange:nullptr];
      CHECK(near([attrs[NSKernAttributeName] doubleValue], 4.0));
      NSParagraphStyle *ps = attrs[NSParagraphStyleAttributeName];
      CHECK(ps != nil && near(ps.minimumLineHeight, 36.0));
      CHECK([attrs[NSUnderlineStyleAttributeName] integerValue] != 0);
      CHECK([attrs[NSStrikethroughStyleAttributeName] integerValue] != 0);
    }

    // 11. ABI v6 Frame-0 (`saveFrame0ToPath:`, `bootFrame0FromPath:`, `bootFrame0FromBytes:`):
    //     persistence round-trip, mismatch/corrupt rejection + auto-remove, and attached-app guard.
    {
      constexpr uint64_t kBuild = 0x123456789ABCDEF0ULL;
      constexpr uint64_t kEnv = 0xCAFEBABE00000042ULL;
      const std::filesystem::path tmpPath =
          std::filesystem::temp_directory_path() / "nve_macos_shell_test_frame0.nve2";
      std::error_code ec;
      std::filesystem::remove(tmpPath, ec);
      NSString *nsPath = [NSString stringWithUTF8String:tmpPath.c_str()];

      RCTNveMacComponentView *producer = make_view();
      CHECK([producer attachNveInterface:&DemoApp::get_interface surfaceId:9u]);
      CHECK([producer applyPropsDictionary:@{@"title" : @"Frame0 Saved"}]);
      NSError *f0Err = nil;
      CHECK([producer saveFrame0ToPath:nsPath appBuildId:kBuild envHash:kEnv error:&f0Err]);
      CHECK(std::filesystem::exists(tmpPath));
      NSData *envelope = [NSData dataWithContentsOfFile:nsPath];
      CHECK(envelope != nil && envelope.length > 0u);

      // Cold boot from bytes rejects wrong build/env and corrupt data, succeeds with matching keys.
      RCTNveMacComponentView *coldBytes = make_view();
      CHECK(![coldBytes bootFrame0FromBytes:envelope appBuildId:(kBuild ^ 1ULL) envHash:kEnv error:&f0Err]);
      CHECK(!coldBytes.hasReplayedFrame0);
      CHECK(![coldBytes bootFrame0FromBytes:envelope appBuildId:kBuild envHash:(kEnv ^ 1ULL) error:&f0Err]);
      CHECK(!coldBytes.hasReplayedFrame0);
      NSMutableData *corrupt = [envelope mutableCopy];
      static_cast<uint8_t *>(corrupt.mutableBytes)[corrupt.length - 1u] ^= 0xFFu;
      CHECK(![coldBytes bootFrame0FromBytes:corrupt appBuildId:kBuild envHash:kEnv error:&f0Err]);
      CHECK([coldBytes bootFrame0FromBytes:envelope appBuildId:kBuild envHash:kEnv error:&f0Err]);
      CHECK(coldBytes.hasReplayedFrame0);
      CHECK([label_text(coldBytes, DemoApp::kTitle) isEqualToString:@"Frame0 Saved"]);

      // Cold boot from file succeeds with matching build/env, then rejects and removes on mismatch.
      RCTNveMacComponentView *coldFile = make_view();
      CHECK([coldFile bootFrame0FromPath:nsPath appBuildId:kBuild envHash:kEnv error:&f0Err]);
      CHECK(coldFile.hasReplayedFrame0);
      CHECK([label_text(coldFile, DemoApp::kTitle) isEqualToString:@"Frame0 Saved"]);
      RCTNveMacComponentView *coldMismatch = make_view();
      CHECK(![coldMismatch bootFrame0FromPath:nsPath appBuildId:(kBuild + 1ULL) envHash:kEnv error:&f0Err]);
      CHECK(!coldMismatch.hasReplayedFrame0 && !std::filesystem::exists(tmpPath));

      // Attached app refuses Frame-0 boot.
      CHECK(![producer bootFrame0FromBytes:envelope appBuildId:kBuild envHash:kEnv error:&f0Err]);
      std::filesystem::remove(tmpPath, ec);
    }

    // 12. `--pair-transport` canonicalization and URI rewriting.
    {
      CHECK(nve::apple::canonical_pair_transport("auto") == std::optional<std::string_view>{"auto"});
      CHECK(nve::apple::canonical_pair_transport("happy-eyeballs") == std::optional<std::string_view>{"auto"});
      CHECK(nve::apple::canonical_pair_transport("UDP") == std::optional<std::string_view>{"quic"});
      CHECK(nve::apple::canonical_pair_transport("webtransport") == std::optional<std::string_view>{"webtransport"});
      CHECK(nve::apple::canonical_pair_transport("H3") == std::optional<std::string_view>{"webtransport"});
      CHECK(nve::apple::canonical_pair_transport("tls") == std::optional<std::string_view>{"wss"});
      CHECK(nve::apple::canonical_pair_transport("tcp") == std::optional<std::string_view>{"ws"});
      CHECK(!nve::apple::canonical_pair_transport("bogus").has_value());

      CHECK(nve::apple::apply_pair_transport("nve://pair?host=127.0.0.1&port=9400", "quic") ==
            std::optional<std::string>{"nve://pair?host=127.0.0.1&port=9400&transport=quic"});
      CHECK(nve::apple::apply_pair_transport("nve://pair?host=127.0.0.1&transport=ws&port=9400", "wt") ==
            std::optional<std::string>{"nve://pair?host=127.0.0.1&port=9400&transport=webtransport"});
      CHECK(nve::apple::apply_pair_transport("127.0.0.1:9400", "quic") ==
            std::optional<std::string>{"quic://127.0.0.1:9400"});
      CHECK(nve::apple::apply_pair_transport("ws://127.0.0.1:9400/pair", "wss") ==
            std::optional<std::string>{"wss://127.0.0.1:9400/pair"});
      CHECK(nve::apple::apply_pair_transport("wss://127.0.0.1:9400", "webtransport") ==
            std::optional<std::string>{"wt://127.0.0.1:9400"});
      CHECK(nve::apple::apply_pair_transport("ws://127.0.0.1:9400", "auto") ==
            std::optional<std::string>{"ws://127.0.0.1:9400"});
      CHECK(!nve::apple::apply_pair_transport("ws://127.0.0.1:9400", "invalid").has_value());
    }

    // 13. Real Rust apps (`check-all.sh`: kitchen-sink + studio `cdylib`s) mount, render text, and
    //    reload with state.
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
      const uint32_t before = real.nveNodeCount;
      CHECK([real reloadNveDylibAtPath:@(dylib) error:&err]);
      for (int i = 0; i < 200 && real.nveNodeCount < before; ++i) {
        [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
      }
      CHECK(real.nveNodeCount == before);
      std::printf("  real app %s: %u nodes, %lu labels\n", env, real.nveNodeCount, static_cast<unsigned long>(labels));
    }

    std::printf("macos_shell_test: %s (%d failure(s))\n", g_failures == 0 ? "ok" : "FAILED", g_failures);
  }
  return g_failures == 0 ? 0 : 1;
}