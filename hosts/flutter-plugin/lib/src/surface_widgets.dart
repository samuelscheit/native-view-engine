import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'app_instance.dart';
import 'channel_client.dart';
import 'event_codec.dart';
import 'nve_bin_value.dart';

/// Rendering strategy for embedding an NVE component inside Flutter.
enum NveRenderMode {
  /// Zero-copy 120fps Skia GPU texture via `FlutterTextureRegistry` (`Texture` widget).
  texture,

  /// Native React Native Fabric view via `UiKitView` / `AndroidView`.
  platformView,
}

int _nextSurfaceId = 1000;

/// Embeds an NVE Rust component into Flutter using zero-copy `Texture(textureId: ...)`
/// compositing at up to 120fps via `FlutterTextureRegistry` (Skia / Metal / Vulkan / Direct3D).
///
/// Works identically across iOS, Android, macOS, Windows, and Linux without platform-view
/// thread-merging penalties.
class NveTextureHostWidget extends StatefulWidget {
  const NveTextureHostWidget({
    super.key,
    required this.componentName,
    this.vtableSymbol = 'nve_get_interface',
    this.libraryPath,
    this.props = const <String, Object?>{},
    this.controller,
    this.onEvent,
    this.surfaceId,
    this.channelClient,
    this.width,
    this.height,
  });

  final String componentName;
  final String vtableSymbol;
  final String? libraryPath;
  final Map<String, Object?> props;
  final NveSurfaceController? controller;
  final NveEventCallback? onEvent;
  final int? surfaceId;
  final NveChannelClient? channelClient;
  final double? width;
  final double? height;

  @override
  State<NveTextureHostWidget> createState() => _NveTextureHostWidgetState();
}

class _NveTextureHostWidgetState extends State<NveTextureHostWidget> {
  static const MethodChannel _textureChannel = MethodChannel('dev.nve/flutter_texture');
  static const EventChannel _eventChannel = EventChannel('dev.nve/flutter_events');

  late final int _surfaceId = widget.surfaceId ?? (_nextSurfaceId++);
  int? _textureId;
  NveAppInstance? _ffiInstance;
  StreamSubscription<dynamic>? _eventSub;
  Size _lastReportedSize = Size.zero;

  @override
  void initState() {
    super.initState();
    _attachController();
    _initSurface();
  }

  @override
  void didUpdateWidget(covariant NveTextureHostWidget oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      oldWidget.controller?.detachHandlers();
      _attachController();
    }
    if (!mapEquals(oldWidget.props, widget.props)) {
      _pushProps(widget.props);
    }
  }

  @override
  void dispose() {
    widget.controller?.detachHandlers();
    _eventSub?.cancel();
    _ffiInstance?.dispose();
    if (_textureId != null) {
      _textureChannel.invokeMethod<void>('destroyTextureSurface', <String, dynamic>{
        'surfaceId': _surfaceId,
        'textureId': _textureId,
      }).catchError((_) {});
    }
    super.dispose();
  }

  void _attachController() {
    widget.controller?.attachHandlers(
      onCommand: _dispatchCommand,
      onRender: _requestRender,
      onSnapshot: () => _ffiInstance?.snapshotState(),
    );
  }

  Future<void> _initSurface() async {
    if (widget.libraryPath != null && widget.libraryPath!.isNotEmpty) {
      try {
        final instance = NveAppInstance.open(
          surfaceId: _surfaceId,
          vtableSymbol: widget.vtableSymbol,
          libraryPath: widget.libraryPath,
        );
        instance.setPropsDirect(widget.props);
        instance.renderNextFrame();
        _ffiInstance = instance;
        _drainFfiEvents();
      } catch (_) {
        // Fallback to MethodChannel-hosted texture registry.
      }
    }

    try {
      final textureId = await _textureChannel.invokeMethod<int>(
        'createTextureSurface',
        <String, dynamic>{
          'surfaceId': _surfaceId,
          'componentName': widget.componentName,
          'vtableSymbol': widget.vtableSymbol,
          'libraryPath': widget.libraryPath,
          'propsBin': NveBinValueEncoder.encode(widget.props),
        },
      );
      if (mounted && textureId != null) {
        setState(() => _textureId = textureId);
      }
      _eventSub = _eventChannel.receiveBroadcastStream(_surfaceId).listen(
        (dynamic payload) {
          if (payload is Map && widget.onEvent != null) {
            widget.onEvent!(NveEvent.fromPlatformMap(payload));
          }
        },
        onError: (_) {},
      );
    } on MissingPluginException {
      // Running in headless widget test or pure-Dart environment.
    }
  }

  void _pushProps(Map<String, Object?> props) {
    final ffiInst = _ffiInstance;
    if (ffiInst != null && !ffiInst.isDisposed) {
      if (ffiInst.setPropsDirect(props)) {
        ffiInst.renderNextFrame();
      }
      _drainFfiEvents();
    }
    _textureChannel.invokeMethod<void>('setProps', <String, dynamic>{
      'surfaceId': _surfaceId,
      'propsBin': NveBinValueEncoder.encode(props),
    }).catchError((_) {});
  }

  void _dispatchCommand(String name, Map<String, Object?> args) {
    final ffiInst = _ffiInstance;
    if (ffiInst != null && !ffiInst.isDisposed) {
      final ev = NveEvent.custom(
        surfaceId: _surfaceId,
        nodeId: 1,
        name: name,
        value: args,
      );
      if (ffiInst.dispatchEventsDirect(<NveEvent>[ev])) {
        ffiInst.renderNextFrame();
      }
      _drainFfiEvents();
    }
    _textureChannel.invokeMethod<void>('dispatchCommand', <String, dynamic>{
      'surfaceId': _surfaceId,
      'command': name,
      'argsBin': NveBinValueEncoder.encode(args),
    }).catchError((_) {});
  }

  void _requestRender() {
    _ffiInstance?.renderNextFrame();
    _textureChannel.invokeMethod<void>('requestRender', <String, dynamic>{
      'surfaceId': _surfaceId,
    }).catchError((_) {});
  }

  void _drainFfiEvents() {
    final cb = widget.onEvent;
    final ffiInst = _ffiInstance;
    if (cb == null || ffiInst == null || ffiInst.isDisposed) return;
    for (final ev in ffiInst.drainEmittedEvents()) {
      cb(ev);
    }
  }

  void _notifyResize(Size size, double dpr) {
    if (size == _lastReportedSize || size.isEmpty) return;
    _lastReportedSize = size;
    _textureChannel.invokeMethod<void>('resizeSurface', <String, dynamic>{
      'surfaceId': _surfaceId,
      'width': size.width,
      'height': size.height,
      'scaleFactor': dpr,
    }).catchError((_) {});
  }

  void _forwardPointer(PointerEvent event, int phase) {
    _textureChannel.invokeMethod<void>('pointerEvent', <String, dynamic>{
      'surfaceId': _surfaceId,
      'pointerId': event.pointer,
      'phase': phase,
      'x': event.localPosition.dx,
      'y': event.localPosition.dy,
      'deltaX': event.delta.dx,
      'deltaY': event.delta.dy,
      'timestampUs': event.timeStamp.inMicroseconds,
    }).catchError((_) {});
  }

  @override
  Widget build(BuildContext context) {
    final dpr = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1.0;

    return SizedBox(
      width: widget.width,
      height: widget.height,
      child: LayoutBuilder(
        builder: (BuildContext context, BoxConstraints constraints) {
          final intrinsic = _ffiInstance?.measure(constraints);
          final resolved = Size(
            widget.width ??
                intrinsic?.width ??
                (constraints.hasBoundedWidth ? constraints.maxWidth : 0.0),
            widget.height ??
                intrinsic?.height ??
                (constraints.hasBoundedHeight ? constraints.maxHeight : 0.0),
          );
          _notifyResize(resolved, dpr);

          final Widget surface = _textureId != null
              ? Texture(textureId: _textureId!, filterQuality: FilterQuality.low)
              : const SizedBox.expand();

          return Listener(
            behavior: HitTestBehavior.opaque,
            onPointerDown: (PointerDownEvent e) => _forwardPointer(e, 1),
            onPointerMove: (PointerMoveEvent e) => _forwardPointer(e, 2),
            onPointerUp: (PointerUpEvent e) => _forwardPointer(e, 3),
            onPointerCancel: (PointerCancelEvent e) => _forwardPointer(e, 4),
            child: SizedBox(
              width: resolved.width > 0 ? resolved.width : null,
              height: resolved.height > 0 ? resolved.height : null,
              child: surface,
            ),
          );
        },
      ),
    );
  }
}

/// Embeds an NVE component rendered via React Native Fabric native views (`UiKitView` on iOS,
/// `AndroidView` on Android), automatically falling back to [NveTextureHostWidget] on desktop.
class NvePlatformViewWidget extends StatefulWidget {
  const NvePlatformViewWidget({
    super.key,
    required this.componentName,
    this.vtableSymbol = 'nve_get_interface',
    this.libraryPath,
    this.props = const <String, Object?>{},
    this.controller,
    this.onEvent,
    this.surfaceId,
    this.channelClient,
    this.width,
    this.height,
  });

  final String componentName;
  final String vtableSymbol;
  final String? libraryPath;
  final Map<String, Object?> props;
  final NveSurfaceController? controller;
  final NveEventCallback? onEvent;
  final int? surfaceId;
  final NveChannelClient? channelClient;
  final double? width;
  final double? height;

  @override
  State<NvePlatformViewWidget> createState() => _NvePlatformViewWidgetState();
}

class _NvePlatformViewWidgetState extends State<NvePlatformViewWidget> {
  static const String _viewType = 'dev.nve/fabric_view';

  late final int _surfaceId = widget.surfaceId ?? (_nextSurfaceId++);
  MethodChannel? _viewChannel;

  @override
  void didUpdateWidget(covariant NvePlatformViewWidget oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      oldWidget.controller?.detachHandlers();
      _attachController();
    }
    if (!mapEquals(oldWidget.props, widget.props)) {
      _viewChannel?.invokeMethod<void>('setProps', <String, dynamic>{
        'propsBin': NveBinValueEncoder.encode(widget.props),
      }).catchError((_) {});
    }
  }

  @override
  void dispose() {
    widget.controller?.detachHandlers();
    _viewChannel?.setMethodCallHandler(null);
    super.dispose();
  }

  void _attachController() {
    widget.controller?.attachHandlers(
      onCommand: (String name, Map<String, Object?> args) {
        _viewChannel?.invokeMethod<void>('dispatchCommand', <String, dynamic>{
          'command': name,
          'argsBin': NveBinValueEncoder.encode(args),
        }).catchError((_) {});
      },
      onRender: () {
        _viewChannel?.invokeMethod<void>('requestRender').catchError((_) {});
      },
      onSnapshot: () => null,
    );
  }

  void _onPlatformViewCreated(int viewId) {
    final channel = MethodChannel('dev.nve/fabric_view_$viewId');
    channel.setMethodCallHandler((MethodCall call) async {
      if (call.method == 'onEvent' && call.arguments is Map && widget.onEvent != null) {
        widget.onEvent!(NveEvent.fromPlatformMap(call.arguments as Map<Object?, Object?>));
      }
      return null;
    });
    _viewChannel = channel;
    _attachController();
  }

  @override
  Widget build(BuildContext context) {
    final creationParams = <String, dynamic>{
      'surfaceId': _surfaceId,
      'componentName': widget.componentName,
      'vtableSymbol': widget.vtableSymbol,
      'libraryPath': widget.libraryPath,
      'propsBin': NveBinValueEncoder.encode(widget.props),
    };

    final Widget child;
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      child = UiKitView(
        viewType: _viewType,
        creationParams: creationParams,
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: _onPlatformViewCreated,
        gestureRecognizers: const <Factory<OneSequenceGestureRecognizer>>{},
      );
    } else if (defaultTargetPlatform == TargetPlatform.android) {
      child = AndroidView(
        viewType: _viewType,
        creationParams: creationParams,
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: _onPlatformViewCreated,
        hitTestBehavior: PlatformViewHitTestBehavior.opaque,
        gestureRecognizers: const <Factory<OneSequenceGestureRecognizer>>{},
      );
    } else {
      return NveTextureHostWidget(
        componentName: widget.componentName,
        vtableSymbol: widget.vtableSymbol,
        libraryPath: widget.libraryPath,
        props: widget.props,
        controller: widget.controller,
        onEvent: widget.onEvent,
        surfaceId: _surfaceId,
        channelClient: widget.channelClient,
        width: widget.width,
        height: widget.height,
      );
    }

    return SizedBox(
      width: widget.width,
      height: widget.height,
      child: child,
    );
  }
}
