// macOS host suite over the real `libnve_runtime`: demo-app mount + styling, props, press ->
// outbound events, resize, calls re-entering from reconcile, runtime-loaded dylib error codes,
// Tier-2 reload with state, core-table scenes (text input, image, scroll, accessibility, rewritten
// image resources), min-content measurement, and pairing errors.
#import <AppKit/AppKit.h>
#import "NveAppleComponentHost.h"
#import "RCTNveMacComponentView.h"

#include "NveAppleDemoApp.hpp"
#include "NveAppleValueBridge.h"

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

/// `nve.res.image_src` bytes: `tag` (0: URL, 1: encoded image), little-endian length, payload.
std::vector<uint8_t> image_src(std::string_view payload, uint8_t tag = 0u) {
  std::vector<uint8_t> out{tag};
  const auto n = static_cast<uint32_t>(payload.size());
  for (uint32_t s = 0; s < 32u; s += 8u) out.push_back(static_cast<uint8_t>((n >> s) & 0xFFu));
  out.insert(out.end(), payload.begin(), payload.end());
  return out;
}

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
/// row 5 to 100pt (row 6 only moves).
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
    b.define_resource(9u, Key64::of("nve.res.image_src"),
                      image_src(std::string_view(static_cast<const char *>(bytes.bytes), bytes.length), 1u));
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
  } else {
    b.set_children(1u, {2u, 4u});
    height(b, 5u, 0x42C80000u);
  }
  const std::vector<uint8_t> frame = b.finish();
  return [NSData dataWithBytes:frame.data() length:frame.size()];
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
  b.patch(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, set_field(input, NVE_TABLE_TEXT_INPUT_SECURE, 1u), input);
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, NVE_TABLE_TEXT_INPUT_VALUE.word, "hunter2");
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, NVE_TABLE_TEXT_INPUT_PLACEHOLDER.word, "Password");
  std::array<uint32_t, NVE_TABLE_IMAGE_WORDS> img{};
  std::copy(std::begin(NVE_TABLE_IMAGE_DEFAULTS), std::end(NVE_TABLE_IMAGE_DEFAULTS), img.begin());
  b.patch(slot(NVE_TABLE_IMAGE_KEY), 3u, set_field(img, NVE_TABLE_IMAGE_SRC, 9u), img);
  std::array<uint32_t, NVE_TABLE_ACCESSIBILITY_WORDS> a11y{};
  std::copy(std::begin(NVE_TABLE_ACCESSIBILITY_DEFAULTS), std::end(NVE_TABLE_ACCESSIBILITY_DEFAULTS), a11y.begin());
  b.patch(slot(NVE_TABLE_ACCESSIBILITY_KEY), 3u, set_field(a11y, NVE_TABLE_ACCESSIBILITY_ROLE, NVE_ARIA_ROLE_IMAGE), a11y);
  b.set_string(slot(NVE_TABLE_ACCESSIBILITY_KEY), 3u, NVE_TABLE_ACCESSIBILITY_LABEL.word, "Brand logo");
  std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> lay{};
  std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 3u, set_field(lay, NVE_TABLE_LAYOUT_WIDTH, 0x42000000u) | set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x42000000u), lay);
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 4u, set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x42C80000u), lay);  // 100pt
  std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
  b.patch(slot(NVE_TABLE_LAYOUT_KEY), 5u, set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x43C80000u) | set_field(lay, NVE_TABLE_LAYOUT_FLEX_SHRINK, 0u), lay);
  std::array<uint32_t, NVE_TABLE_SCROLL_WORDS> scroll{};
  std::copy(std::begin(NVE_TABLE_SCROLL_DEFAULTS), std::end(NVE_TABLE_SCROLL_DEFAULTS), scroll.begin());
  b.patch(slot(NVE_TABLE_SCROLL_KEY), 4u, 0u, scroll);
  b.set_children(1u, {2u, 3u, 4u});
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
            set_field(row, NVE_TABLE_TYPOGRAPHY_SET, NVE_TYPOGRAPHY_SET_FONT_SIZE) |
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
  b.define_resource(9u, Key64::of("nve.res.image_src"), image_src(png_data_uri(8)));
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
    // A clean image node whose resource bytes were rewritten re-decodes; nothing else updates.
    NSImage *before4 = ((NveAppleImageView *)image).image;
    CHECK(near(before4.size.width, 4.0));
    CHECK([scene.nveHost replayFrame0:rewritten_image_scene()]);
    CHECK([scene nativeViewForNodeId:3u] == image && near(((NveAppleImageView *)image).image.size.width, 8.0));
    CHECK(scene.nveLastReconciledDirtyCount == 1u && scene.nveLastReconciledRemovedCount == 0u);

    // 7'. Inline images decode once per resource; a still-live detached child leaves its parent;
    //     scroll content follows a resized child and its moved sibling.
    {
      RCTNveMacComponentView *det = make_view();
      CHECK([det.nveHost replayFrame0:detach_scene(1u)]);
      NveAppleImageView *a = (NveAppleImageView *)[det nativeViewForNodeId:2u];
      NveAppleImageView *b = (NveAppleImageView *)[det nativeViewForNodeId:3u];
      CHECK([a isKindOfClass:[NveAppleImageView class]] && [b isKindOfClass:[NveAppleImageView class]]);
      CHECK(a.image != nil && a.image == b.image);  // one decode shared by both nodes
      NSScrollView *sv = (NSScrollView *)[det nativeViewForNodeId:4u];
      CHECK([sv isKindOfClass:[NSScrollView class]] && near(sv.documentView.frame.size.height, 600.0));
      NSView *parent = b.superview;
      CHECK(parent != nil && [parent.subviews containsObject:b]);
      CHECK([det.nveHost replayFrame0:detach_scene(2u)]);
      CHECK([det nativeViewForNodeId:3u] == b);  // still live, so still realized ...
      CHECK(b.superview == nil && ![parent.subviews containsObject:b]);  // ... but detached
      CHECK(a.superview == parent && sv.superview == parent);
      CHECK(near([det nativeViewForNodeId:6u].frame.origin.y, 100.0));
      CHECK(near(sv.documentView.frame.size.height, 300.0));
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
    CHECK([typo.nveHost replayFrame0:typography_scene(1u)]);
    CHECK(near(font_size(typo, 4u), 30.0) && near(font_size(typo, kRecycledLeaf), 30.0));
    CHECK([typo.nveHost replayFrame0:typography_scene(2u)]);
    CHECK(near(font_size(typo, 4u), 20.0));             // through the dirty `mid`
    CHECK(near(font_size(typo, kRecycledLeaf), 20.0));  // raw NodeId resolved to its slot
    CHECK([typo.nveHost replayFrame0:typography_scene(3u)]);
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

    // 8. Pairing errors surface the runtime's diagnostics.
    CHECK(![scene connectPairedEndpoint:@"" token:@"" surfaceId:1u error:&err] && err.localizedDescription.length > 0);
    CHECK(!scene.isPaired);

    // 9. Real Rust apps (`check-all.sh`: kitchen-sink + studio `cdylib`s) mount, render text, and
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