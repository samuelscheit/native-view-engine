import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';

/// Cross-platform image scaling mode matching `ImageResizeMode` in `canvas_ir.rs`.
enum NveImageResizeMode {
  cover(0),
  contain(1),
  stretch(2),
  center(3);

  const NveImageResizeMode(this.code);
  final int code;

  static NveImageResizeMode parse(String? raw) {
    if (raw == null || raw.isEmpty) return NveImageResizeMode.cover;
    return switch (raw.trim().toLowerCase()) {
      'contain' => NveImageResizeMode.contain,
      'stretch' => NveImageResizeMode.stretch,
      'center' => NveImageResizeMode.center,
      _ => NveImageResizeMode.cover,
    };
  }

  static NveImageResizeMode fromCode(int code) {
    return switch (code) {
      1 => NveImageResizeMode.contain,
      2 => NveImageResizeMode.stretch,
      3 => NveImageResizeMode.center,
      _ => NveImageResizeMode.cover,
    };
  }
}

/// Exact source-crop and destination placement for painting an image into a box.
class NveImagePlacement {
  const NveImagePlacement({
    required this.srcRect,
    required this.dstRect,
  });

  final Rect srcRect;
  final Rect dstRect;

  /// Computes source and destination rectangles for `(imgW, imgH)` inside `dstBounds`.
  static NveImagePlacement? compute(
    Rect dstBounds,
    double imgW,
    double imgH,
    NveImageResizeMode mode,
  ) {
    if (dstBounds.width <= 0.0 || dstBounds.height <= 0.0 || imgW <= 0.0 || imgH <= 0.0) {
      return null;
    }
    final dw = dstBounds.width;
    final dh = dstBounds.height;

    switch (mode) {
      case NveImageResizeMode.stretch:
        return NveImagePlacement(
          srcRect: Rect.fromLTWH(0.0, 0.0, imgW, imgH),
          dstRect: dstBounds,
        );

      case NveImageResizeMode.contain:
        final scale = math.min(dw / imgW, dh / imgH);
        final outW = math.max(0.0, imgW * scale);
        final outH = math.max(0.0, imgH * scale);
        return NveImagePlacement(
          srcRect: Rect.fromLTWH(0.0, 0.0, imgW, imgH),
          dstRect: Rect.fromLTWH(
            dstBounds.left + (dw - outW) * 0.5,
            dstBounds.top + (dh - outH) * 0.5,
            outW,
            outH,
          ),
        );

      case NveImageResizeMode.cover:
        final scale = math.max(dw / imgW, dh / imgH);
        final srcW = (dw / scale).clamp(0.0, imgW).toDouble();
        final srcH = (dh / scale).clamp(0.0, imgH).toDouble();
        return NveImagePlacement(
          srcRect: Rect.fromLTWH(
            (imgW - srcW) * 0.5,
            (imgH - srcH) * 0.5,
            srcW,
            srcH,
          ),
          dstRect: dstBounds,
        );

      case NveImageResizeMode.center:
        final unclippedLeft = dstBounds.left + (dw - imgW) * 0.5;
        final unclippedTop = dstBounds.top + (dh - imgH) * 0.5;
        final clipLeft = math.max(unclippedLeft, dstBounds.left);
        final clipTop = math.max(unclippedTop, dstBounds.top);
        final clipRight = math.min(unclippedLeft + imgW, dstBounds.right);
        final clipBottom = math.min(unclippedTop + imgH, dstBounds.bottom);
        final visW = clipRight - clipLeft;
        final visH = clipBottom - clipTop;
        if (visW <= 0.0 || visH <= 0.0) {
          return null;
        }
        return NveImagePlacement(
          srcRect: Rect.fromLTWH(
            (clipLeft - unclippedLeft).clamp(0.0, imgW).toDouble(),
            (clipTop - unclippedTop).clamp(0.0, imgH).toDouble(),
            math.min(visW, imgW),
            math.min(visH, imgH),
          ),
          dstRect: Rect.fromLTWH(clipLeft, clipTop, visW, visH),
        );
    }
  }
}

/// Bounded-LRU async `ui.Image` decoder & cache supporting `data:image/png;base64,...`,
/// `data:image/rgba;width=W;height=H;base64,...`, and `file://...` URIs.
class NveSceneImageCache {
  NveSceneImageCache({this.maxEntries = 128}) : assert(maxEntries > 0);

  static const int maxImageDimension = 16384;
  static const int maxRawRgbaBytes = 64 * 1024 * 1024;

  final int maxEntries;
  final LinkedHashMap<String, ui.Image> _entries = LinkedHashMap<String, ui.Image>();
  final Map<String, Future<ui.Image?>> _inFlight = <String, Future<ui.Image?>>{};
  final LinkedHashSet<String> _failedUris = LinkedHashSet<String>();
  bool _disposed = false;

  int get length => _entries.length;
  bool containsKey(String uri) => _entries.containsKey(uri);
  bool isInFlight(String uri) => _inFlight.containsKey(uri);

  /// Returns a cached [ui.Image] synchronously if available (updating LRU recency).
  ui.Image? getCached(String uri) {
    final existing = _entries.remove(uri);
    if (existing != null) {
      _entries[uri] = existing;
    }
    return existing;
  }

  /// Synchronously fetches the cached [ui.Image] for [key] or starts one deduplicated async
  /// decode (default: [key] is an image URI; [decode] supplies other sources, e.g. inline encoded
  /// bytes). [onDecoded] fires once, when the decode that this call started succeeds, so a
  /// per-paint caller never piles up callbacks while a decode is in flight.
  ui.Image? resolveOrRequest(String key, {VoidCallback? onDecoded, Future<ui.Image?> Function()? decode}) {
    if (_disposed || key.isEmpty) return null;
    final cached = getCached(key);
    if (cached != null) return cached;
    if (_failedUris.contains(key) || _inFlight.containsKey(key)) return null;
    final future = load(key, decode: decode);
    if (onDecoded != null) {
      future.then((ui.Image? img) {
        if (img != null && !_disposed) {
          onDecoded();
        }
      });
    }
    return null;
  }

  /// Asynchronously decodes and caches [key] (see [resolveOrRequest]), deduplicating requests.
  Future<ui.Image?> load(String key, {Future<ui.Image?> Function()? decode}) {
    if (_disposed || key.isEmpty) return Future<ui.Image?>.value(null);
    final cached = getCached(key);
    if (cached != null) return Future<ui.Image?>.value(cached);
    final inflight = _inFlight[key];
    if (inflight != null) return inflight;

    final future = _decodeAndStore(key, decode ?? () => decodeImageUri(key));
    _inFlight[key] = future;
    return future;
  }

  Future<ui.Image?> _decodeAndStore(String uri, Future<ui.Image?> Function() decode) async {
    try {
      final image = await decode();
      if (_disposed) {
        image?.dispose();
        return null;
      }
      if (image == null) {
        _markFailed(uri);
        return null;
      }
      _failedUris.remove(uri);
      _insert(uri, image);
      return image;
    } catch (_) {
      _markFailed(uri);
      return null;
    } finally {
      _inFlight.remove(uri);
    }
  }

  void _insert(String uri, ui.Image image) {
    final prev = _entries.remove(uri);
    if (prev != null && !identical(prev, image)) {
      prev.dispose();
    }
    while (_entries.length >= maxEntries) {
      final oldestKey = _entries.keys.first;
      final evicted = _entries.remove(oldestKey);
      evicted?.dispose();
    }
    _entries[uri] = image;
  }

  void _markFailed(String uri) {
    if (_failedUris.length >= maxEntries * 2) {
      _failedUris.remove(_failedUris.first);
    }
    _failedUris.add(uri);
  }

  /// Clears all cached [ui.Image] entries and disposes their GPU/memory handles.
  void clear() {
    for (final img in _entries.values) {
      img.dispose();
    }
    _entries.clear();
    _failedUris.clear();
  }

  void dispose() {
    _disposed = true;
    clear();
    _inFlight.clear();
  }

  /// Decodes `data:image/png;base64,...`, `data:image/rgba;width=W;height=H;base64,...`,
  /// or `file://...` into a Flutter [ui.Image].
  static Future<ui.Image?> decodeImageUri(String rawUri) async {
    final trimmed = rawUri.trim();
    if (trimmed.isEmpty) return null;
    final lower = trimmed.toLowerCase();

    if (lower.startsWith('data:')) {
      final comma = trimmed.indexOf(',');
      if (comma <= 5 || comma + 1 >= trimmed.length) return null;
      final meta = trimmed.substring(5, comma);
      final payload = trimmed.substring(comma + 1);
      final parts = meta.split(';');
      final mime = parts.first.trim().toLowerCase();

      var isBase64 = false;
      int? rawWidth;
      int? rawHeight;
      for (var i = 1; i < parts.length; i++) {
        final token = parts[i].trim();
        final tokenLower = token.toLowerCase();
        if (tokenLower == 'base64') {
          isBase64 = true;
        } else {
          final eq = token.indexOf('=');
          if (eq > 0) {
            final k = token.substring(0, eq).trim().toLowerCase();
            final v = token.substring(eq + 1).trim();
            if (k == 'width' || k == 'w') {
              rawWidth = int.tryParse(v);
            } else if (k == 'height' || k == 'h') {
              rawHeight = int.tryParse(v);
            }
          }
        }
      }
      if (!isBase64) return null;

      final Uint8List decodedBytes;
      try {
        final cleaned = payload.replaceAll(RegExp(r'\s+'), '');
        if (cleaned.isEmpty) return null;
        decodedBytes = base64.decode(base64.normalize(cleaned));
      } catch (_) {
        return null;
      }

      if (mime == 'image/rgba') {
        final w = rawWidth;
        final h = rawHeight;
        if (w == null || h == null || w <= 0 || h <= 0 || w > maxImageDimension || h > maxImageDimension) {
          return null;
        }
        final expectedBytes = w * h * 4;
        if (expectedBytes <= 0 || expectedBytes > maxRawRgbaBytes || decodedBytes.length != expectedBytes) {
          return null;
        }
        final completer = Completer<ui.Image>();
        ui.decodeImageFromPixels(
          decodedBytes,
          w,
          h,
          ui.PixelFormat.rgba8888,
          completer.complete,
        );
        return completer.future;
      }

      if (mime.startsWith('image/')) {
        return decodeEncodedBytes(decodedBytes);
      }
      return null;
    }

    if (lower.startsWith('file://')) {
      File? file;
      try {
        final parsed = Uri.parse(trimmed);
        if (parsed.isScheme('file')) {
          file = File.fromUri(parsed);
        }
      } catch (_) {
        // Fallback below for unescaped spaces.
      }
      file ??= File(Uri.decodeFull(trimmed.substring(7)));
      if (!await file.exists()) return null;
      final bytes = await file.readAsBytes();
      if (bytes.isEmpty) return null;
      return decodeEncodedBytes(bytes);
    }

    return null;
  }

  /// Decodes one encoded image (PNG, JPEG, WebP, GIF first frame, … as the engine supports).
  static Future<ui.Image?> decodeEncodedBytes(Uint8List bytes) async {
    if (bytes.isEmpty) return null;
    ui.Codec? codec;
    try {
      codec = await ui.instantiateImageCodec(bytes);
      final frame = await codec.getNextFrame();
      return frame.image;
    } catch (_) {
      return null;
    } finally {
      codec?.dispose();
    }
  }
}
