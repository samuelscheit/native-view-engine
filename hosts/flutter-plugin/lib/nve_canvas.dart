/// Direct Flutter `RenderBox` + Impeller/Skia `Canvas` host for Native View Engine (NVE).
///
/// Re-exports the core `nve_flutter` FFI bindings alongside the zero-PlatformView
/// [NveCanvasView] widget, [NveCanvasController], zero-copy binary [NveRetainedTree]
/// decoder, and Flutter [NveFlexLayoutSolver].
library nve_canvas;

export 'nve_flutter.dart';
export 'src/command_decoder.dart';
export 'src/flex_layout.dart';
export 'src/nve_canvas_host.dart';
