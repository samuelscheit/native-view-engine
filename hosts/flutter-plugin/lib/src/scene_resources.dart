/// Resource access seam for painters plus the decoders of core resource payloads.
///
/// `NveCanvasController.resources` serves `resource_payload` of the live runtime; tests pass
/// synthetic [NveSceneResources].
library;

import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

/// Canonical `FlatResource` kind keys (`Key64`).
final Key64 imageSrcResourceKind = constId('nve.res.image_src');
final Key64 drawListResourceKind = constId('nve.res.draw_list');

/// Canonical payload bytes of published resources (`Resource::encode`), valid while the scene
/// that referenced them is held.
abstract interface class NveSceneResources {
  /// Payload of [resourceId] when its kind matches [expectedKind], or `null` when unknown,
  /// mismatched, or not exposed.
  Uint8List? payload(int resourceId, {required Key64 expectedKind});

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

/// Decodes `FlatResource` `nve.res.image_src` (`u32 tag, u32 byte_len, payload zero-padded to 4 B`);
/// `null` for an unknown tag, invalid UTF-8 URL, non-zero padding, unaligned buffer, truncation, or
/// trailing bytes.
NveImageSource? decodeImageSource(Uint8List payload) {
  if (payload.length < 8 || (payload.length & 3) != 0) {
    return null;
  }
  final r = BinReader(payload);
  try {
    final tag = r.u32();
    final len = r.u32();
    final maxLen = switch (tag) {
      0 => 256 * 1024,
      1 => 32 * 1024 * 1024,
      _ => -1,
    };
    if (maxLen < 0 || len > maxLen) {
      return null;
    }
    final padded = (len + 3) & ~3;
    final raw = r.take(padded);
    if (!r.isAtEnd) {
      return null;
    }
    for (var i = len; i < padded; i++) {
      if (raw[i] != 0) {
        return null;
      }
    }
    final bytes = Uint8List.sublistView(raw, 0, len);
    return switch (tag) {
      0 => isValidUtf8(bytes) ? NveImageUrl(utf8.decode(bytes)) : null,
      1 => NveImageBytes(bytes),
      _ => null,
    };
  } on BinCodecException {
    return null;
  }
}
