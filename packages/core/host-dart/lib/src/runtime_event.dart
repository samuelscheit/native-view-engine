/// Uniform pointer-free 64-byte `NveEvent` value representation (web- and FFI-safe).
library;

import 'dart:typed_data';

import 'generated/nve_abi_const.generated.dart';
import 'key64.dart';

const int _modMask = 0x1F;

/// One uniform 64-byte `NveEvent` record with its decoded payload (web- and FFI-safe).
final class RuntimeEvent {
  const RuntimeEvent({
    required this.table,
    required this.event,
    this.surfaceId = 0,
    this.nodeId = 0,
    this.timestampUs = 0,
    Key64? timestamp,
    this.flags = 0,
    this.x = 0,
    this.y = 0,
    this.width = 0,
    this.height = 0,
    this.pointerId = 0,
    this.keyCode = 0,
    this.key = '',
    this.code = '',
    this.text = '',
    this.name = '',
    this.data,
    this.value,
    this.rawPayload,
  }) : _timestamp = timestamp;

  /// App-defined named custom event (`nve.custom` + `CUSTOM_NAMED`).
  ///
  /// When dispatching, [data] (if provided) is sent verbatim; otherwise [value] (when non-null) is
  /// encoded as `NveBinValue`. When drained, [data] holds the raw bytes and [value] holds the
  /// decoded Dart object (or `null` when [data] is empty or not valid `NveBinValue`).
  const RuntimeEvent.custom(
    int nodeId,
    String name, {
    Object? value,
    Uint8List? data,
    int surfaceId = 0,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: nveTableCustom,
          event: 0,
          surfaceId: surfaceId,
          nodeId: nodeId,
          timestampUs: timestampUs,
          timestamp: timestamp,
          flags: nveEventFlagCustomNamed,
          name: name,
          data: data,
          value: value,
        );

  const RuntimeEvent.press(int nodeId, double x, double y, {int surfaceId = 0, int timestampUs = 0, Key64? timestamp})
      : this(
          table: nveTableInteraction,
          event: 0, // NveInteractionEvent.press
          surfaceId: surfaceId,
          nodeId: nodeId,
          x: x,
          y: y,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  const RuntimeEvent.longPress(int nodeId, double x, double y, {int surfaceId = 0, int timestampUs = 0, Key64? timestamp})
      : this(
          table: nveTableInteraction,
          event: 1, // NveInteractionEvent.longPress
          surfaceId: surfaceId,
          nodeId: nodeId,
          x: x,
          y: y,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  const RuntimeEvent.focus(int nodeId, {bool focused = true, int surfaceId = 0, int timestampUs = 0, Key64? timestamp})
      : this(
          table: nveTableInteraction,
          event: focused ? 2 : 3, // NveInteractionEvent.focus / blur
          surfaceId: surfaceId,
          nodeId: nodeId,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  /// Pointer down / move / up / enter / leave; [button] is `0..=7` (bits 8..=10 of `flags`).
  RuntimeEvent.pointer(
    NveInteractionEvent kind,
    int nodeId,
    double x,
    double y, {
    int pointerId = 0,
    int button = 0,
    int modifiers = 0,
    bool capture = false,
    int surfaceId = 0,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: nveTableInteraction,
          event: kind.code,
          surfaceId: surfaceId,
          nodeId: nodeId,
          x: x,
          y: y,
          pointerId: pointerId,
          flags: (modifiers & _modMask) |
              (((button & 7) << nveEventFlagButtonShift) & nveEventFlagButtonMask) |
              (capture ? nveEventFlagCapture : 0),
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  const RuntimeEvent.wheel(
    int nodeId,
    double dx,
    double dy, {
    int modifiers = 0,
    int surfaceId = 0,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: nveTableInteraction,
          event: 11, // NveInteractionEvent.wheel
          surfaceId: surfaceId,
          nodeId: nodeId,
          x: dx,
          y: dy,
          flags: modifiers & _modMask,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  /// `keyDown` / `keyUp`: [key] is the W3C key value (`<= 16 B` UTF-8 packed inline in `payload[2..6]`,
  /// longer keys split in the byte arena), [code] is the physical key code.
  RuntimeEvent.key(
    NveInteractionEvent kind,
    int nodeId,
    String key,
    String code, {
    int modifiers = 0,
    int keyCode = 0,
    bool composing = false,
    int surfaceId = 0,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: nveTableInteraction,
          event: kind.code,
          surfaceId: surfaceId,
          nodeId: nodeId,
          key: key,
          code: code,
          keyCode: keyCode,
          flags: (modifiers & _modMask) | (composing ? nveEventFlagComposing : 0),
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  const RuntimeEvent.textChange(int nodeId, String text, {int surfaceId = 0, int timestampUs = 0, Key64? timestamp})
      : this(
          table: nveTableTextInput,
          event: 0, // NveTextInputEvent.change
          surfaceId: surfaceId,
          nodeId: nodeId,
          text: text,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  const RuntimeEvent.submit(int nodeId, String text, {int surfaceId = 0, int timestampUs = 0, Key64? timestamp})
      : this(
          table: nveTableTextInput,
          event: 1, // NveTextInputEvent.submit
          surfaceId: surfaceId,
          nodeId: nodeId,
          text: text,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  RuntimeEvent.composition(
    NveTextInputEvent kind,
    int nodeId,
    String text, {
    int surfaceId = 0,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: nveTableTextInput,
          event: kind.code,
          surfaceId: surfaceId,
          nodeId: nodeId,
          text: text,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  /// Scroll position report ([x], [y] = offset; [contentWidth] / [contentHeight] = extent).
  const RuntimeEvent.scroll(
    int nodeId,
    double x,
    double y,
    double contentWidth,
    double contentHeight, {
    int surfaceId = 0,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: nveTableScroll,
          event: 0, // NveScrollEvent.scroll
          surfaceId: surfaceId,
          nodeId: nodeId,
          x: x,
          y: y,
          width: contentWidth,
          height: contentHeight,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  const RuntimeEvent.layout(
    int nodeId,
    double x,
    double y,
    double width,
    double height, {
    int surfaceId = 0,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: nveTableLayout,
          event: 0, // NveLayoutEvent.layout
          surfaceId: surfaceId,
          nodeId: nodeId,
          x: x,
          y: y,
          width: width,
          height: height,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  /// Generic table event for any module-defined `EventEnum`.
  const RuntimeEvent.table(
    Key64 table,
    int event, {
    int surfaceId = 0,
    int nodeId = 0,
    int flags = 0,
    List<int>? payload,
    Uint8List? data,
    int timestampUs = 0,
    Key64? timestamp,
  }) : this(
          table: table,
          event: event,
          surfaceId: surfaceId,
          nodeId: nodeId,
          flags: flags,
          rawPayload: payload,
          data: data,
          timestampUs: timestampUs,
          timestamp: timestamp,
        );

  final Key64 table;
  final int event;
  final int surfaceId;
  final int nodeId;
  final int timestampUs;
  final Key64? _timestamp;
  final int flags;
  final double x, y, width, height;
  final int pointerId;
  final int keyCode;
  final String key;
  final String code;
  final String text;
  final String name;
  final Uint8List? data;
  final Object? value;
  final List<int>? rawPayload;

  /// Exact 64-bit timestamp in microseconds (safe above 2^53 on Dart-web).
  Key64 get timestamp => _timestamp ?? Key64.fromInt(timestampUs);

  NveInteractionEvent? get interaction => table == nveTableInteraction ? NveInteractionEvent.fromCode(event) : null;
  NveTextInputEvent? get textInput => table == nveTableTextInput ? NveTextInputEvent.fromCode(event) : null;
  NveScrollEvent? get scrollEvent => table == nveTableScroll ? NveScrollEvent.fromCode(event) : null;
  NveLayoutEvent? get layoutEvent => table == nveTableLayout ? NveLayoutEvent.fromCode(event) : null;
  bool get isCustom => table == nveTableCustom && (flags & nveEventFlagCustomNamed) != 0;

  int get modifiers => flags & _modMask;
  int get pointerButton => (flags & nveEventFlagButtonMask) >>> nveEventFlagButtonShift;
  bool get isCapture => (flags & nveEventFlagCapture) != 0;
  bool get isComposing => (flags & nveEventFlagComposing) != 0;
}
