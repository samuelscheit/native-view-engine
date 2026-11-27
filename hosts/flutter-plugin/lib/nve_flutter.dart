/// Native View Engine Flutter host: a `libnve_runtime` surface rendered on Flutter's own
/// [Canvas] (zero PlatformViews) through `package:nve_host`'s `dart:ffi` runtime session.
///
/// - [NveCanvasController] owns one `RuntimeSession` (attach an app, pair, or replay `NVE2`).
/// - [NveCanvasView] lays the surface out, paints it, routes input, and exposes semantics.
/// - [NvePairedSurface] pairs with an `nve dev` server.
/// - [NveScenePainter] / [NveTablePainter] paint scene tables (extend with module painters).
library;

export 'package:nve_host/nve_host.dart';
export 'package:nve_host/nve_host_ffi.dart';

export 'src/canvas_semantics.dart';
export 'src/draw_list_painter.dart';
export 'src/nve_canvas_controller.dart';
export 'src/nve_canvas_view.dart';
export 'src/nve_paired_surface.dart';
export 'src/scene_image_cache.dart';
export 'src/scene_painter.dart';
export 'src/scene_resources.dart';
export 'src/scene_style.dart';
export 'src/scene_tree.dart';
