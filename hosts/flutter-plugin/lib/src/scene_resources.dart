/// Resource access seam for painters plus the decoders of core resource payloads.
///
/// `NveRuntimeApi` v1 publishes no resource view, so [NveCanvasController.resources] is `null`
/// until the runtime exposes `resource_payload` (Slice 2B-0); painters then fetch image sources and
/// module payloads (canvas draw lists) through [NveSceneResources] with no other change.
library;

import 'dart:convert' show base64, utf8;
import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

/// Canonical payload bytes of published resources (`Resource::encode`), valid while the scene
/// that referenced them is held.
abstract interface class NveSceneResources {
  /// Payload of [resourceId], or `null` when unknown / not exposed.
  Uint8List? payload(int resourceId);
}

/// Image URI of an `nve.res.image_src` payload: the URL itself (`tag 0, u32 len, utf8`) or a
/// `data:` URI for inline encoded bytes (`tag 1, u32 len, bytes`); `null` when malformed.
String? decodeImageSourceUri(Uint8List payload) {
  if (payload.length < 5) {
    return null;
  }
  final r = BinReader(payload);
  try {
    final tag = r.u8();
    final bytes = r.take(r.u32());
    if (!r.isAtEnd) {
      return null;
    }
    return switch (tag) {
      0 => isValidUtf8(bytes) ? utf8.decode(bytes) : null,
      1 => 'data:application/octet-stream;base64,${base64.encode(bytes)}',
      _ => null,
    };
  } on BinCodecException {
    return null;
  }
}
