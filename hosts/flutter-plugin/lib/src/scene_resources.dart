/// Resource access seam for painters plus the decoders of core resource payloads.
///
/// `NveCanvasController.resources` serves `resource_payload` of the live runtime; tests pass
/// synthetic [NveSceneResources].
library;

import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

/// Canonical payload bytes of published resources (`Resource::encode`), valid while the scene
/// that referenced them is held.
abstract interface class NveSceneResources {
  /// Payload of [resourceId], or `null` when unknown / not exposed.
  Uint8List? payload(int resourceId);

  /// Content versions of this source's resources. Owned by the source (one per runtime
  /// session), never by a painter: a painter shared across sessions, or a controller swap,
  /// never answers one session's resource id with another's decoded image.
  ResourceVersions get versions;
}

/// Decoded `nve.res.image_src` payload (Rust `ImageSrc`).
sealed class NveImageSource {
  const NveImageSource();
}

/// `ImageSrc::Url` (tag 0): fetched / decoded through `NveSceneImageCache.decodeImageUri`.
final class NveImageUrl extends NveImageSource {
  const NveImageUrl(this.uri);
  final String uri;
}

/// `ImageSrc::EncodedBytes` (tag 1): an encoded image decoded directly. [bytes] borrows the
/// scene's resource memory: copy before it outlives the scene read.
final class NveImageBytes extends NveImageSource {
  const NveImageBytes(this.bytes);
  final Uint8List bytes;
}

/// Decodes `tag u8, len u32, bytes`; `null` for an unknown tag, invalid UTF-8 URL, truncation,
/// or trailing bytes.
NveImageSource? decodeImageSource(Uint8List payload) {
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
      0 => isValidUtf8(bytes) ? NveImageUrl(utf8.decode(bytes)) : null,
      1 => bytes.isEmpty ? null : NveImageBytes(bytes),
      _ => null,
    };
  } on BinCodecException {
    return null;
  }
}
