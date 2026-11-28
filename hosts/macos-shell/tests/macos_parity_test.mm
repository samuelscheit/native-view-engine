// Focused Track 1B-2/3B Apple Native Parity suite (`nve_macos_parity_test`):
// - `nve.canvas` CoreGraphics 8-opcode + 1-pt polyline + 66-clip overflow + malformed (NaN/Inf/
//   negative/unaligned/trailing/non-finite bounds) + live `NveAppleCanvasView` `drawRect:` & dirty
//   resource redraws + zero-alloc `hasDrawList`.
// - Shared typography: bold/italic named family (`Menlo`), cached `platform_font`, `white_space: pre`
//   multi-line preservation on `NSTextField`, `overline` pixels, `letter_spacing`, `line_height`
//   (points & percent), and `max_lines`.
// - ABI v6 Frame-0: persistence, cold boot (file & bytes), mismatch auto-remove, corrupt/missing/
//   empty/attached errors with `NveAppleRuntimeStatusErrorKey`.
// - `--pair-transport` & `parse_pair_url` edge cases (canonical `nve://pair?...cert_hash=...`,
//   `auto` stripping `transport=`, non-`nve://` query stripping, `NVE://` case folding, scheme rejection).
// - `TextInput` parity: decoupled `read_only` vs `disabled`, `ControlledText` echo suppression +
//   `A -> B -> A` race + IME `Composition*` + blur composition end.
#import <AppKit/AppKit.h>
#import "NveAppleCanvasView.h"
#import "NveAppleComponentHost.h"
#import "RCTNveMacComponentView.h"

#include "NveAppleDemoApp.hpp"

#include <nve/canvas/canvas.hpp>
#include <nve/canvas/draw_list_encoder.hpp>
#include <nve/host/nve2_builder.hpp>

#include <array>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <limits>
#include <string>
#include <vector>

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

void patch_u32(std::vector<uint8_t> &buf, size_t word, uint32_t v) {
  for (unsigned i = 0; i < 4u; ++i) buf[word * 4u + i] = static_cast<uint8_t>((v >> (8u * i)) & 0xFFu);
}

void patch_f32(std::vector<uint8_t> &buf, size_t word, float v) {
  uint32_t bits = 0u;
  std::memcpy(&bits, &v, sizeof bits);
  patch_u32(buf, word, bits);
}

void test_canvas_parity() {
  using namespace nve::canvas;
  constexpr int kW = 64, kH = 64;
  auto make_bitmap = [&](std::vector<uint8_t> &buf) -> CGContextRef {
    buf.assign(static_cast<size_t>(kW * kH * 4), 0u);
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef ctx = CGBitmapContextCreate(
        buf.data(), kW, kH, 8, kW * 4, cs,
        static_cast<uint32_t>(kCGImageAlphaPremultipliedLast) | static_cast<uint32_t>(kCGBitmapByteOrder32Big));
    CGColorSpaceRelease(cs);
    CGContextTranslateCTM(ctx, 0.0, kH);
    CGContextScaleCTM(ctx, 1.0, -1.0);
    return ctx;
  };
  auto pixel = [](const std::vector<uint8_t> &buf, int x, int y) -> std::array<uint8_t, 4> {
    const size_t off = static_cast<size_t>((y * kW + x) * 4);
    return {buf[off], buf[off + 1], buf[off + 2], buf[off + 3]};
  };

  // All 8 opcodes + 1-point polyline round dot.
  std::vector<uint8_t> pixels;
  CGContextRef ctx = make_bitmap(pixels);
  const DrawPoint poly[] = {{4.0f, 54.0f}, {20.0f, 54.0f}, {20.0f, 60.0f}};
  const DrawPoint singleDot[] = {{56.0f, 56.0f}};
  DrawListEncoder enc;
  enc.clear(DrawColorRgba::rgb(10, 20, 30))
      .fill_rect(2.0f, 2.0f, 12.0f, 12.0f, DrawColorRgba::rgb(255, 0, 0))
      .fill_rounded_rect(20.0f, 2.0f, 16.0f, 16.0f, 4.0f, DrawColorRgba::rgb(0, 255, 0))
      .fill_circle(48.0f, 10.0f, 8.0f, DrawColorRgba::rgb(0, 0, 255))
      .stroke_line(4.0f, 30.0f, 28.0f, 30.0f, 4.0f, DrawColorRgba::rgb(255, 255, 0))
      .stroke_polyline(poly, 4.0f, DrawColorRgba::rgb(0, 255, 255))
      .stroke_polyline(singleDot, 6.0f, DrawColorRgba::rgb(255, 128, 0))
      .clip_rect(36.0f, 36.0f, 12.0f, 12.0f)
      .fill_rect(32.0f, 32.0f, 24.0f, 24.0f, DrawColorRgba::rgb(255, 0, 255))
      .restore_clip();
  const std::vector<uint8_t> dl = enc.finish();
  const DrawListDecodeResult r_ok = nve::apple::render_draw_list_to_cgcontext(ctx, CGRectMake(0, 0, kW, kH), dl);
  CGContextRelease(ctx);
  CHECK(r_ok.ok() && r_ok.ops_decoded == 10u);
  CHECK(pixel(pixels, 0, 0)[0] == 10 && pixel(pixels, 0, 0)[1] == 20 && pixel(pixels, 0, 0)[2] == 30);
  CHECK(pixel(pixels, 8, 8)[0] > 240 && pixel(pixels, 8, 8)[1] < 10);
  CHECK(pixel(pixels, 28, 10)[1] > 240 && pixel(pixels, 28, 10)[0] < 10);
  CHECK(pixel(pixels, 48, 10)[2] > 240 && pixel(pixels, 48, 10)[0] < 10);
  CHECK(pixel(pixels, 16, 30)[0] > 240 && pixel(pixels, 16, 30)[1] > 240);
  CHECK(pixel(pixels, 12, 54)[1] > 240 && pixel(pixels, 12, 54)[2] > 240);
  CHECK(pixel(pixels, 56, 56)[0] > 240 && pixel(pixels, 56, 56)[1] > 110 && pixel(pixels, 56, 56)[2] < 10);
  CHECK(pixel(pixels, 42, 42)[0] > 240 && pixel(pixels, 42, 42)[2] > 240);
  CHECK(pixel(pixels, 33, 33)[0] == 10 && pixel(pixels, 33, 33)[2] == 30);

  // `kMaxClipDepth = 64` overflow balancing + caller gstate restoration.
  {
    std::vector<uint8_t> clipBuf;
    CGContextRef cctx = make_bitmap(clipBuf);
    DrawListEncoder deep;
    deep.clear(DrawColorRgba::rgb(0, 0, 0));
    for (uint32_t i = 0u; i < kMaxClipDepth + 2u; ++i) deep.clip_rect(10.0f, 10.0f, 10.0f, 10.0f);
    deep.restore_clip().restore_clip();
    deep.fill_rect(0.0f, 0.0f, 64.0f, 64.0f, DrawColorRgba::rgb(255, 255, 255));
    CHECK(nve::apple::render_draw_list_to_cgcontext(cctx, CGRectMake(0, 0, kW, kH), deep.finish()).ok());
    CGContextSetRGBFillColor(cctx, 1.0, 0.0, 0.0, 1.0);
    CGContextFillRect(cctx, CGRectMake(0.0, 0.0, 4.0, 4.0));
    CGContextRelease(cctx);
    CHECK(pixel(clipBuf, 15, 15)[0] == 255 && pixel(clipBuf, 40, 40)[0] == 0);
    CHECK(pixel(clipBuf, 2, 2)[0] > 240 && pixel(clipBuf, 2, 2)[1] < 10);
  }

  // Malformed payloads (Truncated, Unaligned, TrailingBytes, NonFiniteFloat, NegativeFloat) & non-finite bounds.
  {
    std::vector<uint8_t> badBuf;
    CGContextRef bctx = make_bitmap(badBuf);
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, kW, kH), std::vector<uint8_t>{1, 0, 0, 0, 1, 0, 0, 0})
              .error == DrawListError::Truncated);
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, kW, kH), std::vector<uint8_t>{0, 0, 0, 0, 1})
              .error == DrawListError::Unaligned);
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, kW, kH), std::vector<uint8_t>{0, 0, 0, 0, 0, 0, 0, 0})
              .error == DrawListError::TrailingBytes);

    DrawListEncoder rectEnc;
    rectEnc.fill_rect(1.0f, 2.0f, 10.0f, 10.0f, DrawColorRgba::rgb(255, 0, 0));
    std::vector<uint8_t> nanRect = rectEnc.finish();
    patch_f32(nanRect, 2u, std::numeric_limits<float>::quiet_NaN());
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, kW, kH), nanRect).error == DrawListError::NonFiniteFloat);
    std::vector<uint8_t> infRect = rectEnc.finish();
    patch_f32(infRect, 4u, std::numeric_limits<float>::infinity());
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, kW, kH), infRect).error == DrawListError::NonFiniteFloat);
    DrawListEncoder circleEnc;
    circleEnc.fill_circle(10.0f, 10.0f, 5.0f, DrawColorRgba::rgb(255, 0, 0));
    std::vector<uint8_t> negCircle = circleEnc.finish();
    patch_f32(negCircle, 4u, -5.0f);
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, kW, kH), negCircle).error == DrawListError::NegativeFloat);

    // Non-finite bounds guard: returns Ok validation but performs zero CG drawing.
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(0, 0, INFINITY, kH), dl).ok());
    CHECK(nve::apple::render_draw_list_to_cgcontext(bctx, CGRectMake(NAN, 0, kW, kH), dl).ok());
    CGContextRelease(bctx);
    CHECK(pixel(badBuf, 8, 8)[3] == 0u);
  }

  // Live `nve.canvas` table mounting, `hasDrawList`, `drawRect:` bitmap rendering, dedup & dirty-resource redraw.
  {
    constexpr uint8_t kCanvasSlot = 28u;
    const std::array<uint32_t, NVE_TABLE_CANVAS_WORDS> cdef{NVE_TABLE_CANVAS_DEFAULTS[0], NVE_TABLE_CANVAS_DEFAULTS[1]};
    const std::array<uint8_t, NVE_TABLE_CANVAS_WORDS> cfx{NVE_TABLE_CANVAS_WORD_EFFECTS[0], NVE_TABLE_CANVAS_WORD_EFFECTS[1]};
    DrawListEncoder dl1, dl2;
    dl1.clear(DrawColorRgba::rgb(240, 10, 10));
    dl2.clear(DrawColorRgba::rgb(10, 240, 10));
    const std::vector<uint8_t> bytes1 = dl1.finish(), bytes2 = dl2.finish();

    Nve2FrameBuilder f1(1u);
    f1.declare_core_tables();
    f1.declare({Key64{NVE_TABLE_CANVAS_KEY}, Key64{NVE_TABLE_CANVAS_SCHEMA_HASH}, kCanvasSlot, true,
                static_cast<uint8_t>(NVE_TABLE_CANVAS_FLAGS), cdef, cfx,
                NVE_TABLE_CANVAS_STR_WORDS, NVE_TABLE_CANVAS_RESOURCE_WORDS, NVE_TABLE_CANVAS_NAME});
    f1.create_nodes({1u, 2u});
    f1.define_resource(42u, kDrawListKind, bytes1);
    std::array<uint32_t, NVE_TABLE_CANVAS_WORDS> crow = cdef;
    f1.patch(kCanvasSlot, 2u, set_field(crow, NVE_TABLE_CANVAS_DRAW_LIST, 42u) | set_field(crow, NVE_TABLE_CANVAS_GENERATION, 1u), crow);
    std::array<uint32_t, NVE_TABLE_LAYOUT_WORDS> lay{};
    std::copy(std::begin(NVE_TABLE_LAYOUT_DEFAULTS), std::end(NVE_TABLE_LAYOUT_DEFAULTS), lay.begin());
    f1.patch(Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_LAYOUT_KEY}), 2u,
             set_field(lay, NVE_TABLE_LAYOUT_WIDTH, 0x42000000u) | set_field(lay, NVE_TABLE_LAYOUT_HEIGHT, 0x42000000u), lay);
    f1.set_children(1u, {2u});
    const std::vector<uint8_t> raw1 = f1.finish();

    RCTNveMacComponentView *cvHost = make_view();
    CHECK([cvHost.nveHost submitNve2Frame:[NSData dataWithBytes:raw1.data() length:raw1.size()]]);
    NSView *mounted = [cvHost nativeViewForNodeId:2u];
    CHECK([mounted isKindOfClass:[NveAppleCanvasView class]]);
    if ([mounted isKindOfClass:[NveAppleCanvasView class]]) {
      NveAppleCanvasView *canvasView = (NveAppleCanvasView *)mounted;
      CHECK(canvasView.hasDrawList && canvasView.redrawCount == 1u && canvasView.drawListData.length == bytes1.size());

      // Exercise real AppKit `drawRect:` onto an NSBitmapImageRep.
      NSBitmapImageRep *rep = [canvasView bitmapImageRepForCachingDisplayInRect:canvasView.bounds];
      [canvasView cacheDisplayInRect:canvasView.bounds toBitmapImageRep:rep];
      NSColor *c1 = [[rep colorAtX:8 y:8] colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
      CHECK(c1 != nil && c1.redComponent > 0.85 && c1.greenComponent < 0.15);

      // Frame 2: touch unrelated visual opacity -> zero spurious redraws.
      Nve2FrameBuilder f2(2u);
      std::array<uint32_t, NVE_TABLE_VISUAL_WORDS> vis{};
      std::copy(std::begin(NVE_TABLE_VISUAL_DEFAULTS), std::end(NVE_TABLE_VISUAL_DEFAULTS), vis.begin());
      uint32_t op_bits = 0u;
      const float half = 0.75f;
      std::memcpy(&op_bits, &half, sizeof op_bits);
      f2.patch(Nve2FrameBuilder::core_slot(Key64{NVE_TABLE_VISUAL_KEY}), 2u, set_field(vis, NVE_TABLE_VISUAL_OPACITY, op_bits), vis);
      const std::vector<uint8_t> raw2 = f2.finish();
      CHECK([cvHost.nveHost submitNve2Frame:[NSData dataWithBytes:raw2.data() length:raw2.size()]]);
      CHECK(canvasView.redrawCount == 1u);

      // Frame 3: rewrite resource 42 in-place while node 2 is clean -> dirty-resource redraw.
      Nve2FrameBuilder f3(3u);
      f3.define_resource(42u, kDrawListKind, bytes2);
      const std::vector<uint8_t> raw3 = f3.finish();
      CHECK([cvHost.nveHost submitNve2Frame:[NSData dataWithBytes:raw3.data() length:raw3.size()]]);
      CHECK([cvHost nativeViewForNodeId:2u] == canvasView && canvasView.redrawCount == 2u);
      NSBitmapImageRep *rep2 = [canvasView bitmapImageRepForCachingDisplayInRect:canvasView.bounds];
      [canvasView cacheDisplayInRect:canvasView.bounds toBitmapImageRep:rep2];
      NSColor *c2 = [[rep2 colorAtX:8 y:8] colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
      CHECK(c2 != nil && c2.greenComponent > 0.85 && c2.redComponent < 0.15);
    }
  }
}

void test_typography_parity() {
  const auto measure = [](std::string_view sv, const NveRuntimeTextStyle &st, float max_w) {
    float w = 0.0f, h = 0.0f;
    nve::apple::native_text_measure(nullptr, NveStrSlice{reinterpret_cast<const uint8_t *>(sv.data()), sv.size()}, &st, max_w, &w, &h);
    return std::pair{w, h};
  };

  // Named family resolution + weight 700 bold trait + font caching.
  NSFont *menloReg = nve::apple::platform_font(15.0f, 400u, false, "NonexistentNveFont, Menlo, monospace");
  NSFont *menloReg2 = nve::apple::platform_font(15.0f, 400u, false, "NonexistentNveFont, Menlo, monospace");
  NSFont *menloBold = nve::apple::platform_font(15.0f, 700u, false, "NonexistentNveFont, Menlo, monospace");
  NSFont *menloBoldItalic = nve::apple::platform_font(15.0f, 700u, true, "Menlo");
  CHECK([menloReg.familyName isEqualToString:@"Menlo"] && menloReg == menloReg2);  // cached instance
  CHECK([menloBold.familyName isEqualToString:@"Menlo"]);
  CHECK(![menloBold.fontName isEqualToString:menloReg.fontName]);
  CHECK((menloBold.fontDescriptor.symbolicTraits & NSFontDescriptorTraitBold) != 0u);
  CHECK((menloBoldItalic.fontDescriptor.symbolicTraits & NSFontDescriptorTraitBold) != 0u);
  CHECK((menloBoldItalic.fontDescriptor.symbolicTraits & NSFontDescriptorTraitItalic) != 0u);

  // Letter spacing, line height (points & percent), and max_lines.
  NveRuntimeTextStyle base{};
  base.font_size = 14.0f;
  base.font_weight = 400u;
  NveRuntimeTextStyle wide = base;
  wide.letter_spacing = 6.0f;
  CHECK(measure("Typography", wide, INFINITY).first > measure("Typography", base, INFINITY).first + 30.0f);

  NveRuntimeTextStyle ptsLH = base;
  ptsLH.line_height_unit = NVE_LINE_HEIGHT_POINTS;
  ptsLH.line_height = 40.0f;
  NveRuntimeTextStyle pctLH = base;
  pctLH.line_height_unit = NVE_LINE_HEIGHT_PERCENT;
  pctLH.line_height = 250.0f;
  NveRuntimeTextStyle capLH = base;
  capLH.max_lines = 1u;
  constexpr std::string_view kMulti = "Line one\nLine two\nLine three";
  const float bh = measure(kMulti, base, INFINITY).second;
  CHECK(measure(kMulti, ptsLH, INFINITY).second > bh + 40.0f);
  CHECK(measure(kMulti, pctLH, INFINITY).second > bh + 20.0f);
  CHECK(measure(kMulti, capLH, INFINITY).second > 0.0f && measure(kMulti, capLH, INFINITY).second < bh);

  // Live mounted NSTextField: bold Menlo, kern, line_height, max_lines, underline + strikethrough + overline,
  // AND a second label testing `white_space: pre` (must preserve multiple lines, NOT collapse to 1!).
  auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
  Nve2FrameBuilder tb(1u);
  tb.declare_core_tables().create_nodes({1u, 2u, 3u});
  tb.set_string(slot(NVE_TABLE_TEXT_KEY), 2u, NVE_TABLE_TEXT_TEXT.word, "Styled label");
  tb.set_string(slot(NVE_TABLE_TEXT_KEY), 3u, NVE_TABLE_TEXT_TEXT.word, "Pre line 1\nPre line 2\nPre line 3");
  std::array<uint32_t, NVE_TABLE_TYPOGRAPHY_WORDS> trow{};
  std::copy(std::begin(NVE_TABLE_TYPOGRAPHY_DEFAULTS), std::end(NVE_TABLE_TYPOGRAPHY_DEFAULTS), trow.begin());
  uint32_t sz_bits = 0u, ls_bits = 0u, lh_bits = 0u;
  const float sz = 16.0f, ls = 4.0f, lh = 36.0f;
  std::memcpy(&sz_bits, &sz, sizeof sz_bits);
  std::memcpy(&ls_bits, &ls, sizeof ls_bits);
  std::memcpy(&lh_bits, &lh, sizeof lh_bits);
  const uint32_t mask =
      set_field(trow, NVE_TABLE_TYPOGRAPHY_FONT_SIZE, sz_bits) |
      set_field(trow, NVE_TABLE_TYPOGRAPHY_FONT_WEIGHT, 700u) |
      set_field(trow, NVE_TABLE_TYPOGRAPHY_LETTER_SPACING, ls_bits) |
      set_field(trow, NVE_TABLE_TYPOGRAPHY_LINE_HEIGHT, (lh_bits & ~3u) | static_cast<uint32_t>(NVE_LENGTH_POINTS)) |
      set_field(trow, NVE_TABLE_TYPOGRAPHY_MAX_LINES, 2u) |
      set_field(trow, NVE_TABLE_TYPOGRAPHY_TEXT_DECORATION,
                NVE_TEXT_DECORATION_UNDERLINE | NVE_TEXT_DECORATION_LINE_THROUGH | NVE_TEXT_DECORATION_OVERLINE);
  trow[0] |= uint32_t{1} << NVE_TABLE_TYPOGRAPHY_FONT_FAMILY.explicit_bit;
  tb.patch(slot(NVE_TABLE_TYPOGRAPHY_KEY), 2u, mask | 1u, trow);
  tb.set_string(slot(NVE_TABLE_TYPOGRAPHY_KEY), 2u, NVE_TABLE_TYPOGRAPHY_FONT_FAMILY.word, "NonexistentNveFont, Menlo, monospace");

  std::array<uint32_t, NVE_TABLE_TYPOGRAPHY_WORDS> preRow{};
  std::copy(std::begin(NVE_TABLE_TYPOGRAPHY_DEFAULTS), std::end(NVE_TABLE_TYPOGRAPHY_DEFAULTS), preRow.begin());
  tb.patch(slot(NVE_TABLE_TYPOGRAPHY_KEY), 3u, set_field(preRow, NVE_TABLE_TYPOGRAPHY_WHITE_SPACE, NVE_WHITE_SPACE_PRE), preRow);
  tb.set_children(1u, {2u, 3u});
  const std::vector<uint8_t> tframe = tb.finish();

  RCTNveMacComponentView *tview = make_view();
  CHECK([tview.nveHost submitNve2Frame:[NSData dataWithBytes:tframe.data() length:tframe.size()]]);
  NSTextField *lbl = (NSTextField *)[tview nativeViewForNodeId:2u];
  CHECK([lbl isKindOfClass:[NSTextField class]]);
  CHECK([lbl.font.familyName isEqualToString:@"Menlo"]);
  CHECK((lbl.font.fontDescriptor.symbolicTraits & NSFontDescriptorTraitBold) != 0u);
  CHECK(lbl.maximumNumberOfLines == 2);
  NSDictionary<NSAttributedStringKey, id> *attrs = [lbl.attributedStringValue attributesAtIndex:0 effectiveRange:nullptr];
  CHECK(near([attrs[NSKernAttributeName] doubleValue], 4.0));
  NSParagraphStyle *ps = attrs[NSParagraphStyleAttributeName];
  CHECK(ps != nil && near(ps.minimumLineHeight, 36.0));
  CHECK([attrs[NSUnderlineStyleAttributeName] integerValue] != 0);
  CHECK([attrs[NSStrikethroughStyleAttributeName] integerValue] != 0);
  // Verify overline actually paints dark pixels on row y=0..1 of the label bitmap.
  NSBitmapImageRep *lblRep = [lbl bitmapImageRepForCachingDisplayInRect:lbl.bounds];
  [lbl cacheDisplayInRect:lbl.bounds toBitmapImageRep:lblRep];
  int overlineInk = 0;
  for (NSInteger x = 4; x < std::min<NSInteger>(lblRep.pixelsWide, 60); ++x) {
    NSColor *c = [[lblRep colorAtX:x y:0] colorUsingColorSpace:NSColorSpace.sRGBColorSpace];
    if (c != nil && c.alphaComponent > 0.2) ++overlineInk;
  }
  CHECK(overlineInk > 10);

  // `white_space: pre` must NOT force single-line mode or 1-line cap!
  NSTextField *preLbl = (NSTextField *)[tview nativeViewForNodeId:3u];
  CHECK([preLbl isKindOfClass:[NSTextField class]]);
  CHECK(!preLbl.usesSingleLineMode && preLbl.maximumNumberOfLines == 0);
  CHECK(preLbl.frame.size.height > 40.0);  // 3 lines tall, not collapsed to 1 line
}

void test_frame0_parity() {
  constexpr uint64_t kBuild = 0x123456789ABCDEF0ULL, kEnv = 0xCAFEBABE00000042ULL;
  const std::filesystem::path tmpPath = std::filesystem::temp_directory_path() / "nve_macos_parity_frame0.nve2";
  std::error_code ec;
  std::filesystem::remove(tmpPath, ec);
  NSString *nsPath = [NSString stringWithUTF8String:tmpPath.c_str()];

  RCTNveMacComponentView *producer = make_view();
  CHECK([producer attachNveInterface:&DemoApp::get_interface surfaceId:9u]);
  CHECK([producer applyPropsDictionary:@{@"title" : @"Frame0 Saved"}]);
  NSError *f0Err = nil;
  CHECK([producer saveFrame0ToPath:nsPath appBuildId:kBuild envHash:kEnv error:&f0Err]);
  NSData *envelope = [NSData dataWithContentsOfFile:nsPath];
  CHECK(envelope != nil && envelope.length > 0u);

  // Missing file & empty bytes report `NVE_RUNTIME_NOT_FOUND` / `NVE_RUNTIME_INVALID_ARG` clearly.
  RCTNveMacComponentView *coldBytes = make_view();
  CHECK(![coldBytes bootFrame0FromPath:@"/nonexistent/frame0.nve2" appBuildId:kBuild envHash:kEnv error:&f0Err]);
  CHECK(f0Err != nil && f0Err.code == static_cast<NSInteger>(NVE_RUNTIME_NOT_FOUND));
  CHECK([f0Err.userInfo[NveAppleRuntimeStatusErrorKey] integerValue] == static_cast<NSInteger>(NVE_RUNTIME_NOT_FOUND));
  CHECK(![coldBytes bootFrame0FromBytes:[NSData data] appBuildId:kBuild envHash:kEnv error:&f0Err]);
  CHECK(f0Err != nil && f0Err.code == static_cast<NSInteger>(NVE_RUNTIME_INVALID_ARG));

  // Build/env mismatch & corrupt bytes fail cleanly; matching bytes succeed.
  CHECK(![coldBytes bootFrame0FromBytes:envelope appBuildId:(kBuild ^ 1ULL) envHash:kEnv error:&f0Err]);
  CHECK(!coldBytes.hasReplayedFrame0);
  CHECK(![coldBytes bootFrame0FromBytes:envelope appBuildId:kBuild envHash:(kEnv ^ 1ULL) error:&f0Err]);
  NSMutableData *corrupt = [envelope mutableCopy];
  static_cast<uint8_t *>(corrupt.mutableBytes)[corrupt.length - 1u] ^= 0xFFu;
  CHECK(![coldBytes bootFrame0FromBytes:corrupt appBuildId:kBuild envHash:kEnv error:&f0Err]);
  CHECK([coldBytes bootFrame0FromBytes:envelope appBuildId:kBuild envHash:kEnv error:&f0Err]);
  CHECK(coldBytes.hasReplayedFrame0 && [label_text(coldBytes, DemoApp::kTitle) isEqualToString:@"Frame0 Saved"]);

  // File boot succeeds, then mismatch auto-removes the stale file.
  RCTNveMacComponentView *coldFile = make_view();
  CHECK([coldFile bootFrame0FromPath:nsPath appBuildId:kBuild envHash:kEnv error:&f0Err]);
  CHECK(coldFile.hasReplayedFrame0 && [label_text(coldFile, DemoApp::kTitle) isEqualToString:@"Frame0 Saved"]);
  RCTNveMacComponentView *coldMismatch = make_view();
  CHECK(![coldMismatch bootFrame0FromPath:nsPath appBuildId:(kBuild + 1ULL) envHash:kEnv error:&f0Err]);
  CHECK(f0Err.code == static_cast<NSInteger>(NVE_RUNTIME_NOT_FOUND));
  CHECK(!coldMismatch.hasReplayedFrame0 && !std::filesystem::exists(tmpPath));

  // Attached app refuses Frame-0 boot with `NVE_RUNTIME_BUSY`.
  CHECK(![producer bootFrame0FromBytes:envelope appBuildId:kBuild envHash:kEnv error:&f0Err]);
  CHECK(f0Err.code == static_cast<NSInteger>(NVE_RUNTIME_BUSY));
  std::filesystem::remove(tmpPath, ec);
}

void test_pair_transport_and_deep_links() {
  CHECK(nve::apple::canonical_pair_transport("auto") == std::optional<std::string_view>{"auto"});
  CHECK(nve::apple::canonical_pair_transport("happy-eyeballs") == std::optional<std::string_view>{"auto"});
  CHECK(nve::apple::canonical_pair_transport("UDP") == std::optional<std::string_view>{"quic"});
  CHECK(nve::apple::canonical_pair_transport("webtransport") == std::optional<std::string_view>{"webtransport"});
  CHECK(nve::apple::canonical_pair_transport("H3") == std::optional<std::string_view>{"webtransport"});
  CHECK(nve::apple::canonical_pair_transport("tls") == std::optional<std::string_view>{"wss"});
  CHECK(nve::apple::canonical_pair_transport("tcp") == std::optional<std::string_view>{"ws"});
  CHECK(!nve::apple::canonical_pair_transport("bogus").has_value());

  // `nve://` query rewrite + `auto` stripping existing `transport=` + uppercase `NVE://` folding.
  CHECK(nve::apple::apply_pair_transport("nve://pair?host=127.0.0.1&port=9400", "quic") ==
        std::optional<std::string>{"nve://pair?host=127.0.0.1&port=9400&transport=quic"});
  CHECK(nve::apple::apply_pair_transport("NVE://pair?host=127.0.0.1&transport=ws&port=9400", "wt") ==
        std::optional<std::string>{"nve://pair?host=127.0.0.1&port=9400&transport=webtransport"});
  CHECK(nve::apple::apply_pair_transport("nve://pair?host=127.0.0.1&transport=ws&port=9400", "auto") ==
        std::optional<std::string>{"nve://pair?host=127.0.0.1&port=9400"});

  // Non-`nve://` scheme rewrites + `?transport=` query stripping + unrecognized scheme rejection.
  CHECK(nve::apple::apply_pair_transport("127.0.0.1:9400", "quic") ==
        std::optional<std::string>{"quic://127.0.0.1:9400"});
  CHECK(nve::apple::apply_pair_transport("ws://127.0.0.1:9400/pair?transport=ws&k=1", "quic") ==
        std::optional<std::string>{"quic://127.0.0.1:9400/pair?k=1"});
  CHECK(nve::apple::apply_pair_transport("wss://127.0.0.1:9400?transport=wss", "auto") ==
        std::optional<std::string>{"wss://127.0.0.1:9400"});
  CHECK(!nve::apple::apply_pair_transport("http://127.0.0.1:9400", "ws").has_value());
  CHECK(!nve::apple::apply_pair_transport("ftp://127.0.0.1:9400", "quic").has_value());
  CHECK(!nve::apple::apply_pair_transport("ws://127.0.0.1:9400", "invalid").has_value());

  // Canonical `format_wire_uri()` iOS deep link preserves `quic`, `wss`, `cert_hash`, and `token` intact!
  const auto canonical = nve::apple::parse_pair_url(
      "NVE://pair?host=192.168.1.5&quic=8091&wss=8092&token=secret42&cert_hash=deadbeef0123");
  CHECK(canonical.has_value());
  CHECK(canonical->endpoint == "nve://pair?host=192.168.1.5&quic=8091&wss=8092&token=secret42&cert_hash=deadbeef0123");
  CHECK(canonical->token == "secret42");
  CHECK(canonical->transport.empty());
  CHECK(!nve::apple::parse_pair_url("nve://pair").has_value());
  CHECK(!nve::apple::parse_pair_url("https://example.com").has_value());
}

void test_text_input_parity() {
  // Decoupled `read_only` vs `disabled` on mounted AppKit NSTextField.
  auto slot = [](uint64_t key) { return Nve2FrameBuilder::core_slot(Key64{key}); };
  Nve2FrameBuilder b(1u);
  b.declare_core_tables().create_nodes({1u, 2u, 3u});
  std::array<uint32_t, NVE_TABLE_TEXT_INPUT_WORDS> ro{};
  std::copy(std::begin(NVE_TABLE_TEXT_INPUT_DEFAULTS), std::end(NVE_TABLE_TEXT_INPUT_DEFAULTS), ro.begin());
  b.patch(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, set_field(ro, NVE_TABLE_TEXT_INPUT_READ_ONLY, 1u), ro);
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 2u, NVE_TABLE_TEXT_INPUT_VALUE.word, "readonly-val");
  std::array<uint32_t, NVE_TABLE_TEXT_INPUT_WORDS> rw{};
  std::copy(std::begin(NVE_TABLE_TEXT_INPUT_DEFAULTS), std::end(NVE_TABLE_TEXT_INPUT_DEFAULTS), rw.begin());
  b.patch(slot(NVE_TABLE_TEXT_INPUT_KEY), 3u, 0u, rw);
  b.set_string(slot(NVE_TABLE_TEXT_INPUT_KEY), 3u, NVE_TABLE_TEXT_INPUT_VALUE.word, "a");
  b.set_children(1u, {2u, 3u});
  const std::vector<uint8_t> raw = b.finish();

  RCTNveMacComponentView *v = make_view();
  CHECK([v.nveHost submitNve2Frame:[NSData dataWithBytes:raw.data() length:raw.size()]]);
  NSTextField *roField = (NSTextField *)[v nativeViewForNodeId:2u];
  CHECK([roField isKindOfClass:[NSTextField class]]);
  CHECK(roField.enabled && !roField.editable && roField.selectable);  // read-only != disabled

  // ControlledText `A -> B -> A` echo suppression + IME guard + deferred pendingAppValue.
  NveAppleComponentHost *host = v.nveHost;
  CHECK(![host shouldApplyAppText:@"a" toNodeId:3u shownText:@"a" hasMarkedText:NO]);
  __block NSString *written = nil;
  [host handleTextInputEditOnNodeId:3u text:@"ab" hasMarkedText:NO markedText:nil writeBack:^(NSString *t) { written = t; }];
  [host handleTextInputEditOnNodeId:3u text:@"a" hasMarkedText:NO markedText:nil writeBack:^(NSString *t) { written = t; }];
  // Unrelated frame still carrying "a" MUST NOT evict "ab" from in_flight_!
  CHECK(![host shouldApplyAppText:@"a" toNodeId:3u shownText:@"a" hasMarkedText:NO]);
  // Delayed echo "ab" is absorbed without overwriting user's "a"!
  CHECK(![host shouldApplyAppText:@"ab" toNodeId:3u shownText:@"a" hasMarkedText:NO]);
  CHECK(![host shouldApplyAppText:@"a" toNodeId:3u shownText:@"a" hasMarkedText:NO]);

  // IME composition guard & deferred app reset flush on cancel.
  [host handleTextInputEditOnNodeId:3u text:@"ani" hasMarkedText:YES markedText:@"ni" writeBack:^(NSString *t) { written = t; }];
  CHECK(![host shouldApplyAppText:@"reset" toNodeId:3u shownText:@"ani" hasMarkedText:YES]);
  CHECK(![host handleTextInputSubmitOnNodeId:3u text:@"ani" hasMarkedText:YES]);  // Return suppressed during IME
  // Cancel IME back to "a" -> deferred "reset" is flushed cleanly via writeBack!
  written = nil;
  [host handleTextInputEditOnNodeId:3u text:@"a" hasMarkedText:NO markedText:nil writeBack:^(NSString *t) { written = t; }];
  CHECK([written isEqualToString:@"reset"]);
}

}  // namespace

int main() {
  @autoreleasepool {
    [NSApplication sharedApplication];
    test_canvas_parity();
    test_typography_parity();
    test_frame0_parity();
    test_pair_transport_and_deep_links();
    test_text_input_parity();
    std::printf("macos_parity_test: %s (%d failure(s))\n", g_failures == 0 ? "ok" : "FAILED", g_failures);
  }
  return g_failures == 0 ? 0 : 1;
}
