# UI & Framework Maturity Audit (`native-view-engine`)

Clean-slate capability inventory produced via [`.agents/skills/unbiased-feature-brainstorm/SKILL.md`](../.agents/skills/unbiased-feature-brainstorm/SKILL.md) (zero codebase priming, web-search grounded), diffed against NVE at `0335ca0` and cross-referenced with [docs/CHANGELOG.md](./CHANGELOG.md) (shipped) and [docs/BACKLOG.md](./BACKLOG.md) (open roadmap).

Legend: **Done** = production foundation shipped (see [CHANGELOG.md](./CHANGELOG.md)) · **Partial** = core exists, key edge cases or cross-platform parity missing · **Missing** = not yet implemented (tracked in [BACKLOG.md](./BACKLOG.md)).

---

## Tier 1 — Non-Negotiable Table Stakes

Absence of any Tier 1 item makes an app feel broken, non-native, inaccessible, or unshippable on at least one platform.

### 1. Text Rendering, Editing & Input Systems

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases to Preserve / Solve |
| :--- | :---: | :--- | :--- |
| **Full Unicode Shaping, Script Fallback & Color Emoji** | **Partial** | Native OS hosts + Web DOM inherit OS shaping; Skia backend (`packages/backends/skia`) needs verified CJK/Arabic/color-emoji fallback & mixed-baseline stability. Tracked in [BACKLOG.md](./BACKLOG.md#p1--ui--engine-maturity-from-ui-maturity-audit). | Han unification (`zh-CN`/`zh-TW`/`ja`/`ko`), inline emoji baseline stability, alpha-mask vs. RGBA color emoji batching. |
| **Native IME Composition & Dead Keys** | **Done** | Shipped unified `CompositionStart/Update/End` events (`EventMask::COMPOSITION`) across `nve-abi`, `nve-codegen`, `nve-dioxus`, C++ bridges, Web (`compositionstart/update/end` + `focusout`), and `hosts/skia-shell` (`ime.rs` + live `focused_ime_cursor_rect()` candidate-popup tracking on every caret/scroll/resize/DPI change) ([CHANGELOG.md](./CHANGELOG.md)). | Inline pre-edit without cursor desync, HiDPI candidate popup placement, focus blur / `Enter` / `Backspace` mid-composition. |
| **Bidirectional (BiDi) Text Layout & Logical/Visual Caret Mapping** | **Done** | Shipped HarfBuzz + `unicode-bidi` single-line layout runs (`text_bidi.rs`), visual click → logical byte hit-testing, disjoint multi-run selection rects, and visual `Left`/`Right` caret stepping across mixed LTR/RTL runs in `nve-backend-skia` ([CHANGELOG.md](./CHANGELOG.md)). | Visual click → logical grapheme offset across mixed RTL/LTR runs; arrow/Backspace across run boundaries. |
| **Native Text Selection, Caret Navigation & Clipboard** | **Done / Partial** | Shipped UAX #29 grapheme-safe editing/caret/selection/hit-testing (ZWJ emoji, flags, combining marks), double-click word & triple-click paragraph select, `\r\n` paste normalization, and `nve-clipboard` ([CHANGELOG.md](./CHANGELOG.md)). Remaining: mobile touch selection handles/loupes. | ZWJ/emoji grapheme deletion, auto-scroll while dragging handles, `\r\n` / rich-text paste normalization. |
| **System Autofill, Password Managers & OTP SMS Codes** | **Done / Partial** | Shipped zero-serialization `AutofillHint` (16 variants) & `InputMode` (7 variants) in `nve-components` (canonical HTML, Apple `UITextContentType`/`UIKeyboardType`, Android `AUTOFILL_HINT_*`), wired into `nve-dioxus`, `nve-ui::Input`, Web SSR/DOM emission, and delegated `change` + `InputSyncTracker` for zero-keystroke autofill sync ([CHANGELOG.md](./CHANGELOG.md)). Remaining: native Apple/Android host prop wiring. | Syncing form state when OS autofill mutates the field with zero key events. |

### 2. Layout, Constraints & Scrolling

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases to Preserve / Solve |
| :--- | :---: | :--- | :--- |
| **Predictable Constraint & Box Layout (Flexbox + Wrapping + Intrinsic Sizing)** | **Done** | Taffy + Yoga + Web CSS Flexbox, `flex-wrap`, `gap`, `min/max`, `flex-basis`, `aspect-ratio`, `position: relative/absolute/sticky` shipped ([CHANGELOG.md](./CHANGELOG.md)). | `min-width: auto` blowout, `aspect-ratio` + `stretch` cycles, subpixel border snapping at 125%/150% DPI. |
| **Safe Area Insets & Edge-to-Edge Geometry** | **Partial** | `SafeAreaInsets` in `packages/core/interop/src/surface.rs` & iOS/Android hosts; needs unified edge-to-edge bleed + landscape rotation inset updates across all mobile hosts. | Background bleed behind translucent bars while interactive chrome respects insets; cold-start inset flash. |
| **Soft Keyboard Avoidance & Viewport Insets** | **Done / Partial** | Shipped 8B `#[repr(C)]` `KeyboardInsets` built-in trait (`TraitEnvironment` slot 18), zero-double-padding `effective_bottom_inset()`, Web `window.visualViewport` resize/scroll tracking, and `nve_ui::KeyboardAvoidingView` ([CHANGELOG.md](./CHANGELOG.md)). Remaining: iOS/Android native host keyboard observer wiring. | Double-padding (`adjustResize` + manual padding), iOS interactive drag-dismiss sync, bottom home-indicator sum. |
| **Platform-Idiomatic Scroll Physics & Input Handling** | **Partial** | Native scroll views on Apple/Android/Web/Qt/GTK + gesture arbitration (`packages/core/interop/src/gesture.rs`); Skia scroll lacks platform-tuned rubber-band/inertia, `Shift+Wheel`, `overscroll-behavior: contain`, and prepend scroll anchoring. | Scroll chaining across nested carousels/modals; preserving scroll anchor on chat history prepend. |
| **Viewport Clipping, Overflow & Stacking Contexts (Portals / Overlays)** | **Done / Partial** | `overflow: hidden/scroll`, `z_index`, native HTML5 top-layer `<dialog closedby>` + `commandfor` + `[popover]` + `popovertarget` + `<details>`/`<summary>` (`Modal`/`Dialog`/`Popover`/`DropdownMenu`/`Tooltip`/`Disclosure`/`Accordion` in `packages/framework/ui`) shipped ([CHANGELOG.md](./CHANGELOG.md)). Remaining: native desktop/mobile top-layer portal reparenting out of `overflow: hidden` ancestors. | Visual z-order matching hit-test order; backdrop scroll-lock without horizontal shift. |

### 3. Input, Gestures & Focus Management

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases to Preserve / Solve |
| :--- | :---: | :--- | :--- |
| **Unified Pointer, Touch, Mouse & Trackpad Pipeline (Hover + Cursors)** | **Done / Partial** | Shipped zero-serialization `:hover`/`:active`/`:focus`/`:focus-visible`/`:disabled` `StateStyleRule`, `CursorKind`, `InteractionTracker` (sticky-hover clear, drag-off press-cancel, `:focus-visible` modality), zero-JS Web CSS `@media (hover:hover)` pseudo-classes, Headless/Skia resolution, and `winit` cursor syncing ([CHANGELOG.md](./CHANGELOG.md)). Remaining: pointer capture across window exits and native `PointerEnter`/`Leave`/`Move` in `NveDirectEvent` (Wave C1). | Sticky hover after touch tap on hybrid devices; pointer capture when drag exits window; platform touch slop. |
| **Deterministic Keyboard Focus & Tab Traversal** | **Partial** | `FocusScope`, `FocusTrap`, `RovingFocus`, `Escape` dismissal & focus restoration shipped in `packages/framework/ui/src/focus.rs` ([CHANGELOG.md](./CHANGELOG.md)). Missing `:focus-visible` vs `:focus` modality distinction and safe fallback when focused node unmounts. | Keyboard-only focus rings vs mouse click; unmount focus recovery. |
| **OS-Idiomatic Keyboard Shortcuts & Modifiers** | **Partial** | `Cmd`/`Ctrl` shortcuts work in Studio & Docs (`Cmd+K`); missing declarative shortcut registry with physical vs layout-aware matching (AZERTY/QWERTZ/Cyrillic) and automatic IME/input suppression. | Single-key shortcuts (`j`/`k`/`Space`) never firing inside text inputs or active IME. |

### 4. Accessibility (a11y) Foundation

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases to Preserve / Solve |
| :--- | :---: | :--- | :--- |
| **Native OS Accessibility Tree Mapping (Semantics Layer)** | **Partial** | Web ARIA attributes (`accessibility_role`, `accessibility_label`, live regions) shipped; native hosts map basic roles; Skia/desktop needs full AccessKit/UIA/AT-SPI/VoiceOver/TalkBack tree sync with scroll-updated rects and semantic actions. | Bidirectional screen-reader actions (`Press`, `Increment`, `ScrollIntoView`); grouping composite list rows. |
| **Dynamic Type / OS Font Scaling & Layout Reflow** | **Partial** | `TextScale` trait & `text_scale_factor` exist in `packages/core/interop/src/traits`; needs automatic scaling + reflow verification at 150%–300% across `nve-ui` components. | Proportional padding/height scaling; wrapping label+value rows vertically at extreme text scales. |
| **Reduced Motion & System Contrast Preferences** | **Done / Partial** | `ReducedMotion`, `HighContrast`, `ColorScheme` traits + `Sec-CH-Prefers-Reduced-Motion` / `@media (prefers-reduced-motion)` shipped ([CHANGELOG.md](./CHANGELOG.md)); needs Windows Forced Colors border/focus-ring verification. | Instant cuts/crossfades preserving animation-completion callbacks; Forced Colors border visibility. |

### 5. Rendering, Display & Runtime Stability

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases to Preserve / Solve |
| :--- | :---: | :--- | :--- |
| **HiDPI / Fractional Scaling & Multi-Monitor Live DPI Changes** | **Done / Partial** | `ScaleFactor` trait & DPI scaling in `hosts/skia-shell` / Apple / Windows / Android / Web; verify live monitor drag between 1x and 2x / 1.25x / 1.5x displays without coordinate skew. | Physical px vs logical pt separation in hit-testing, texture sizing, and resize events. |
| **Zero Idle CPU/GPU Usage (On-Demand Redraw)** | **Done** | Event-driven mutation & scene diffing (`packages/core/engine`, `hosts/skia-shell`, `packages/backends/web`) redraws only on dirty frames ([CHANGELOG.md](./CHANGELOG.md)). | Clean display-link stop when animations settle; 0-frame wake lag on input/timer/caret blink. |
| **Off-Thread Image Decoding & Bounded Memory Caching** | **Done / Partial** | Byte-budgeted `ImageCache` shipped in `packages/backends/skia/src/image_cache` & `hosts/swiftui-kit` ([CHANGELOG.md](./CHANGELOG.md)); ensure off-thread decode downscaling to target layout dimensions and OS `onTrimMemory` hooks across all mobile hosts. | Decode to target display dimensions (not raw 4000×3000 RGBA); byte-budget LRU eviction. |
| **GPU Context Loss, Window Resize & Sleep/Wake Recovery** | **Partial** | Desktop resize & swapchain recreation work in `hosts/skia-shell` / Windows D3D12; needs explicit GPU context-loss / driver-reset recovery test coverage retaining CPU-side texture/glyph sources. | Recompiling shaders & re-uploading atlases cleanly after Android backgrounding or GPU switch. |
| **Error Boundaries & Graceful Failure** | **Done** | Shipped subtree `ErrorBoundary` in `nve-dioxus` (backed by `dioxus_core::ErrorContext` panic + `throw_ui_error` isolation, `#[track_caller]` source locations, deduplicated `on_error`, recovery `reset()`) and themed `nve_ui::UiErrorBoundary` ([CHANGELOG.md](./CHANGELOG.md)). | Isolating subtree render failures with structured telemetry and recovery reset. |

### 6. Core UI Controls, Navigation & Platform Essentials

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases to Preserve / Solve |
| :--- | :---: | :--- | :--- |
| **Accessible Core Control Set** | **Partial** | `Button`, `IconButton`, `Input`, `Checkbox`, `Switch`, `Card`, `Badge`, `Tabs`, `Dialog`, `Modal`, `Popover`, `DropdownMenu`, `Tooltip`, `Toast`, `Separator`, `VirtualList` shipped in `packages/framework/ui` ([CHANGELOG.md](./CHANGELOG.md)). Missing `RadioGroup`, `Slider`, `Select`, indeterminate checkbox, and built-in pseudo-state visuals (`:hover`/`:active`/`:focus-visible`/`:disabled`). | Press-cancel on pointer drag-off; indeterminate state; tooltips on disabled controls. |
| **Basic Routing, Back Button & URL Sync** | **Done** | Typed `nve-router` (`define_routes!`, stack, deep links, Web History API SPA + native `<a href>` before hydration) shipped ([CHANGELOG.md](./CHANGELOG.md)). | Clean path refresh; duplicate push guard on rapid double-tap. |
| **Web Target Table Stakes** | **Done** | Semantic DOM + zero-CLS SSR + hydration + `<a href>` middle/Cmd-click + `Cmd+F` + `< 350 KB` brotli bundles (`counter` 105 KB br, `studio` 277–307 KB br) shipped ([CHANGELOG.md](./CHANGELOG.md)). | Zero canvas-only web traps; preserves SEO, find-in-page, password managers, and native link UX. |
| **System Dark/Light Mode Tracking** | **Done** | Live OS theme tracking (`nve-system-env`, `TraitColorScheme`, `Sec-CH-Prefers-Color-Scheme`, `@media (prefers-color-scheme)`) shipped ([CHANGELOG.md](./CHANGELOG.md)). | Instant switch without remounting state or wrong-theme cold-start flash. |

---

## Tier 2 — Expected Production Maturity

Capabilities teams hit in months 1–3 of building real consumer or B2B apps.

### 1. Complex Components & Form Ergonomics

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases |
| :--- | :---: | :--- | :--- |
| **Production `Combobox` / Searchable Select & Multi-Select** | **Done** | Shipped in `packages/framework/ui/src/components/combobox*.rs` + typed ARIA attrs in `nve-dioxus` ([CHANGELOG.md](./CHANGELOG.md)). | Virtual highlight (`aria-activedescendant`) while input keeps focus, custom filter, multi-select tags, IME-safe suppression. |
| **Locale-Aware Date, Time & Date-Range Pickers** | **Done** | Shipped zero-dep `DatePicker` & `DateRangePicker` in `packages/framework/ui/src/components/date_picker*.rs` built on `nve-i18n` (`CalendarDate`, CLDR `first_day_of_week`, `DateSegmentOrder`, `TimeOfDay`) and `positioning.rs` collision-flipping geometry ([CHANGELOG.md](./CHANGELOG.md)). | Keyboard segment typing (`MM/DD/YYYY` vs `DD.MM.YYYY`), localized week start, DST/UTC off-by-one bugs. |
| **Smart Anchored Overlays (`Popover`, `ContextMenu`, `Tooltip` Collision Flipping)** | **Done** | Shipped in `packages/framework/ui/src/positioning.rs`, wired into `Popover`/`Tooltip`/`DropdownMenu` ([CHANGELOG.md](./CHANGELOG.md)). | 12 placements, bottom→top / right→left flip, axis shift, `auto_size`, clamped arrow tracking, safe-triangle submenu corridors. |
| **Bottom Sheets, Drawers & Action Sheets (Snap Points + Gesture Handoff)** | **Done** | Shipped in `packages/framework/ui/src/components/bottom_sheet.rs` ([CHANGELOG.md](./CHANGELOG.md)). | Multi-detent (`Peek`/`Half`/`Full`) velocity flick snapping + seamless inner-scroll handoff at `scrollTop == 0`. |
| **Virtualized Lists, Grids & Sticky Headers (Variable-Height)** | **Partial** | `VirtualList` + `FenwickHeightTree` (100k variable-height rows) shipped in `packages/framework/ui` ([CHANGELOG.md](./CHANGELOG.md)). Missing virtualized grids, sticky pushing headers, and prepend anchor preservation. | Lazy measurement for unknown heights, preserving focused inputs in recycled rows, bidirectional chat prepend. |
| **Form Validation, Dirty Tracking & Unsaved-Changes Guards** | **Done** | Shipped in `packages/framework/ui/src/form.rs` ([CHANGELOG.md](./CHANGELOG.md)). | Sync + cross-field + generation-guarded async validation, `first_error_field()`, synchronous `can_pop()` guard. |

### 2. Layout, Animation & Gesture Depth

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases |
| :--- | :---: | :--- | :--- |
| **CSS Grid (`minmax`, Tracks, Spanning) & Container Queries** | **Done / Partial** | Shipped full CSS Grid (`display: grid`, `grid-template-columns`/`rows`, `minmax`, `fr`, `repeat`, `span`) across `LayoutExt`, `StylePatch`, `TypedStyle`, Taffy 0.16 (`nve-backend-headless`) and native Web CSS ([CHANGELOG.md](./CHANGELOG.md)). Remaining: CSS `@container` queries. | `minmax(0, 1fr)` blowout protection, explicit/implicit tracks, subgrid alignment, parent-width container queries. |
| **Formal Gesture Arena & Cooperative Disambiguation** | **Partial** | `GestureArbitrator` (vertical/horizontal scroll handoff & orthogonal pass-through) shipped in `packages/core/interop/src/gesture.rs` ([CHANGELOG.md](./CHANGELOG.md)). Missing multi-touch pinch/zoom/rotate around focal midpoint and velocity flick arena. | Swipe-to-delete inside vertical list inside horizontal pager; canceling press highlight on scroll claim. |
| **Interruptible Physics-Based Spring Animations & Style Transitions** | **Done** | Shipped closed-form `SpringSpec`/`SpringState` + `MotionClock` in `nve-components`, `TypedStyle::transition()` in `nve-dioxus`, platform-native CSS `.nve-t-*` + `@media (prefers-reduced-motion:reduce)` in `nve-backend-web`, and zero-idle-CPU `MotionDriver` in `nve-backend-skia` ([CHANGELOG.md](./CHANGELOG.md)). | Velocity-preserving retargeting on mid-flight interruption; compositor-driven 60/120Hz ticking. |
| **Layout & Shared-Element (Hero) Transitions + Scroll-Driven Animations** | **Missing** | Tracked in [BACKLOG.md](./BACKLOG.md#p1--ui--engine-maturity-from-ui-maturity-audit). | Mid-flight reversal during interactive swipe-back; compositor transforms/opacity without layout thrash. |
| **Rich Graphics Primitives (SVG, Gradients, Shadows, Transforms, Clip/Masks)** | **Partial** | `box-shadow`, `overflow` clip, and typed 2D `Transform2D` (`translate`/`scale`/`rotate` + inverse hit-testing + Skia `PushTransform`/`PopTransform` + Web CSS `transform:`) shipped ([CHANGELOG.md](./CHANGELOG.md)). Remaining: linear/radial gradients in Skia, native Skia SVG rendering, and custom clip-path/masks. Tracked in [BACKLOG.md](./BACKLOG.md#p1--ui--engine-maturity-from-ui-maturity-audit). | Cached compiled SVG display lists, anti-aliased bezier clipping, GPU-friendly multi-shadows. |

### 3. Platform Integration & Native Escape Hatches

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases |
| :--- | :---: | :--- | :--- |
| **Native Platform View Embedding (`WebView`, `Camera`, `Video`, `Blur`, Maps)** | **Done / Partial** | Opt-in `<NativeView>` (`BlurSurface`, `WebView`, `VideoPlayer`, `CameraPreview` + `NVNP`/`NVNC`/`NVCF`) across Web, Apple, Android, Windows, Qt, GTK, Flutter ([CHANGELOG.md](./CHANGELOG.md)). Remaining: thin SwiftUI/Flutter camera wrappers, live emulator QA, and eliminating in-process `NVNP`/`NVNC` serialization in Wave C3. | Z-order overlays above embedded native views; cooperative gesture arbitration; hardware release on unmount. |
| **Mobile Navigation & System Polish** | **Partial** | 15 modular host services (`haptics`, `share`, `secure-store`, `linking`, `web-auth`, `location`, `sensors`, `audio`, `notifications`, `camera`, etc.) shipped ([CHANGELOG.md](./CHANGELOG.md)). Missing Android Predictive Back synchronous `canPop`, iOS interactive edge-swipe back, and cold-start deep-link queue. | Cold-start URI intent queuing across auth gate; process-death route restoration. |
| **Desktop OS Integration** | **Partial** | Multi-window (`WindowManager`), `nve-filesystem`, `nve-notifications`, `nve-clipboard` shipped ([CHANGELOG.md](./CHANGELOG.md)). **Missing**: native OS menu bars, context menus, system tray, OS-to-app / app-to-OS drag-and-drop (file drop), and custom frameless titlebar drag/Snap Layouts. Tracked in [BACKLOG.md](./BACKLOG.md#p1--ui--engine-maturity-from-ui-maturity-audit). | Shared multi-window engine state; Windows 11 Snap Layouts & macOS traffic-light geometry; drag cursors. |
| **Web Integration & SSR / Progressive Loading** | **Done** | Full SSR + generic reusable Web SSG (`prerender_nve_app_frame0`, `render_ssg_app_document`), 0ms instant snapshot & `NVB1` binary state restore (`WebSnapshotStore`, `889 B` boot script), zero-CLS `@media`, pre-hydration event replay, 3-way `StateReconciler` hydration, `Sec-CH-*` Client Hints, and content-hashed immutable bundles shipped ([CHANGELOG.md](./CHANGELOG.md)). | Zero hydration mismatches across locale/viewport/theme. |

### 4. Internationalization (i18n / l10n) & Theming Systems

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases |
| :--- | :---: | :--- | :--- |
| **Full RTL Mirroring & Logical Layout Properties** | **Done** | Shipped inheritable `LayoutDirection::{Ltr, Rtl}`, logical inline edges (`padding_inline_start/end`, `margin_inline_start/end`) across Taffy, Yoga (`YGDirection`/`YGEdgeStart`), and Web CSS (`dir="rtl"`, `padding-inline-*`), plus selective `IconMirrorPolicy` in `nve-i18n` ([CHANGELOG.md](./CHANGELOG.md)). | Flip directional chevrons/back arrows; never flip checkmarks, clocks, search, `×`, or logos. |
| **CLDR Pluralization, Formatting & Pseudo-Localization** | **Done** | Shipped platform-first `packages/framework/i18n` (`nve-i18n`: `WebIntlBackend` `Intl.*` on `wasm32` + opt-outable `BuiltinI18nBackend` CLDR v45+, ICU MessageFormat, `PseudoLocalizer`, `IconMirrorPolicy`) ([CHANGELOG.md](./CHANGELOG.md)). | 6 Arabic/Welsh plural categories, ICU MessageFormat, currency/number/date/relative-time, CI pseudo-l10n expansion checks. |
| **Design Token Architecture & Adaptive Theming** | **Done** | Typed `UiTheme` / `UiPalette` / `SpacingScale` / `RadiiScale` / `TypographyScale`, `UiDensity`, `ThemeProvider`, and scoped `TraitOverrides` shipped ([CHANGELOG.md](./CHANGELOG.md)). | Instant theme switching without state loss; dark subtree inside light app. |

### 5. Developer Experience (DX), Testing & Observability

| Capability | Status | NVE Evidence / Tracking | Key Edge Cases |
| :--- | :---: | :--- | :--- |
| **Sub-Second Stateful Hot Reload** | **Done** | 4-tier hot reload (Tier 1 RSX `NVP1`, Tier 2 `cdylib` + `signal_archive` 3-way `StateReconciler`, Tier 3/4 incremental) shipped ([CHANGELOG.md](./CHANGELOG.md)). | Preserving inputs, scroll, nav stack, and open dialogs across edits. |
| **Actionable Layout & Recomposition Diagnostics** | **Partial** | `HeadlessBackend` ASCII tree + computed rects & Chrome/Perfetto traces exist; missing live visual overlay inspector (click-to-inspect box model/tokens/semantics) & live recomposition highlighter. Tracked in [BACKLOG.md](./BACKLOG.md#p1--ui--engine-maturity-from-ui-maturity-audit). | Visual overflow badges, zero-size collapse diagnostics, per-phase frame budget attribution. |
| **Semantic UI Testing & Deterministic Golden Testing** | **Done / Partial** | Fast `< 50ms` `HeadlessBackend` tests, Chrome E2E, and visual screenshot QA exist; add ergonomic `find_by_role(Role, label)` semantic query helpers & deterministic cross-OS golden diff harness. | Tolerance masking for dynamic regions; role/label queries resilient to styling refactors. |
| **Predictable Packaging & Brownfield ("Add-to-App") Embedding** | **Done** | `nve-codegen` multi-target exporters (RN Fabric, Flutter, SwiftUI, Compose, Web Component), opt-in modular `packages/modules/*`, thin host shells, and `InteropRuntime` islands shipped ([CHANGELOG.md](./CHANGELOG.md)). | Shared engine across multiple native ViewControllers/Activities; lazy pre-warming. |

---

## Tier 3 — Delighters & Power-User Capabilities

Tracked as long-term Flagship / Vision capabilities in [BACKLOG.md](./BACKLOG.md#flagship--vision-remaining-work-only):

- **Next-Gen Visuals & Motion**: Display P3 / wide-gamut / HDR (`RGBA16F`), 120Hz ProMotion/LTPO dynamic VRR + input prediction, analytical SDF squircles (native Mica/Acrylic/Vibrancy already shipped in `packages/modules/native-views`), variable fonts (`opsz`/`tnum`), Lottie/Rive vector motion & custom runtime shaders.
- **Pro-App & Enterprise Workstation Primitives**: Spreadsheet-grade dual-axis virtualized Data Grid (frozen columns, multi-cell copy/paste), Docking Layouts & nested `SplitPane` + virtualized `TreeView`, first-class `UndoRedoStack` *(active now)* + `Cmd+K` Command Palette *(done in `nve-docs` & `studio-workspace`)*, Infinite Pan/Zoom Canvas + Marquee Selection, Rope/Piece-Table Rich Text & Code Editor.
- **Advanced Hardware & Form Factors**: 240Hz stylus/pen input (pressure/tilt/palm rejection), foldables/dual-screen hinge avoidance, TV/Watch/Spatial 10-foot UI *(Phase 1–2 done, Phase 3 in flight)*, Document Pagination & Native Printing / Vector PDF Export.
- **Best-in-Class Engineering Ergonomics**: Fine-grained signals *(done via `nve-dioxus`)*, Headless/SSR/Offscreen pipeline *(done)*, Compile-time safety & zero-cost `#[derive(NveAbi)]` FFI *(done in Wave C0.5)*, Interactive Component Workbench & Time-Travel State-Replay Inspector *(foundation in `studio-workspace` + `signal_archive`)*.

---

## Scorecard: The 5 Architectural Traps That Kill Cross-Platform UI Frameworks

| Trap | NVE Architectural Position |
| :--- | :--- |
| **1. Underestimating Text & Input** | **Action Needed** — Native OS hosts & Web DOM inherit system text quality, but Skia needs full BiDi, unified IME composition events, grapheme-safe selection, and cross-platform autofill/OTP hints. |
| **2. The Canvas-Only Web Trap** | **Avoided** — `nve-backend-web` renders real semantic DOM + content-addressed CSS + zero-CLS SSR + hydration + 987B pre-hydration event replay (no canvas-only web). |
| **3. Treating Accessibility as a Bolt-On** | **Partially Avoided** — Focus management (`FocusScope`/`FocusTrap`/`RovingFocus`), ARIA attributes, and `TraitEnvironment` are built in; next step is native desktop/Skia AccessKit/UIA/AT-SPI tree parity & `:focus-visible`. |
| **4. Ignoring Native Escape Hatches** | **Avoided** — First-class `<NativeView>` (`BlurSurface`, `WebView`, `VideoPlayer`, `CameraPreview`), opt-in `packages/modules/*`, `InteropRuntime` islands, and `GestureArbitrator` are core engine primitives. |
| **5. Opaque Performance & Layout Diagnostics** | **Partially Avoided** — Fast headless ASCII/rect inspector, Perfetto traces, and 4-tier stateful hot reload exist; add a live visual layout/recomposition overlay inspector. |
