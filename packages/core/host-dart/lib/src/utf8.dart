/// Strict UTF-8 shared by the codec, hashing, and scene text (Rust `core::str::from_utf8`
/// acceptance), identical on the Dart VM and Dart-web.
library;

import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'bin_codec.dart' show BinCodecException, BinErrorKind;

/// Strict validation of `b[start ..< end]` without decoding. Elements outside `0..=255` are invalid.
bool isValidUtf8(List<int> b, [int start = 0, int? end]) {
  final stop = RangeError.checkValidRange(start, end, b.length);
  var i = start;
  while (i < stop) {
    final b0 = b[i];
    if (b0 >= 0 && b0 < 0x80) {
      i++;
      continue;
    }
    int need;
    var lo = 0x80;
    var hi = 0xBF;
    if (b0 >= 0xC2 && b0 <= 0xDF) {
      need = 1;
    } else if (b0 >= 0xE0 && b0 <= 0xEF) {
      need = 2;
      if (b0 == 0xE0) {
        lo = 0xA0;
      }
      if (b0 == 0xED) {
        hi = 0x9F;
      }
    } else if (b0 >= 0xF0 && b0 <= 0xF4) {
      need = 3;
      if (b0 == 0xF0) {
        lo = 0x90;
      }
      if (b0 == 0xF4) {
        hi = 0x8F;
      }
    } else {
      return false;
    }
    if (need >= stop - i || b[i + 1] < lo || b[i + 1] > hi) {
      return false;
    }
    for (var k = 2; k <= need; k++) {
      final c = b[i + k];
      if (c < 0x80 || c > 0xBF) {
        return false;
      }
    }
    i += need + 1;
  }
  return true;
}

/// Decodes already-validated UTF-8, keeping a leading U+FEFF: `dart:convert` strips one leading
/// BOM on the VM and on the web, which would silently change text that Rust preserves.
String decodeUtf8PreserveBom(Uint8List bytes) {
  final hasBom = bytes.length >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF;
  // Decode everything (slicing first would expose a second BOM to the same stripping), then restore
  // exactly the one leading U+FEFF the decoder dropped.
  final text = utf8.decode(bytes);
  return hasBom ? '\uFEFF$text' : text;
}

BinCodecException _loneSurrogate(int index) =>
    BinCodecException(BinErrorKind.invalidUtf8, 'lone surrogate at index $index');

/// UTF-8 byte length of [s]; throws [BinCodecException] (`invalidUtf8`) on a lone surrogate, which
/// a Rust `&str` can never hold (never encoded as U+FFFD).
int utf8EncodedLength(String s) {
  var len = 0;
  for (var i = 0; i < s.length; i++) {
    final c = s.codeUnitAt(i);
    if (c < 0x80) {
      len += 1;
    } else if (c < 0x800) {
      len += 2;
    } else if (c >= 0xD800 && c <= 0xDFFF) {
      _requirePair(s, i);
      len += 4;
      i++;
    } else {
      len += 3;
    }
  }
  return len;
}

/// Encodes the code point at `s[i]` (a surrogate pair consumes `s[i + 1]` too) into `out[o ..]`
/// and returns the end offset; throws on a lone surrogate.
int encodeUtf8CodePointAt(String s, int i, List<int> out, int o) {
  final c = s.codeUnitAt(i);
  if (c < 0x80) {
    out[o] = c;
    return o + 1;
  }
  if (c < 0x800) {
    out[o] = 0xC0 | (c >>> 6);
    out[o + 1] = 0x80 | (c & 0x3F);
    return o + 2;
  }
  if (c >= 0xD800 && c <= 0xDFFF) {
    _requirePair(s, i);
    final cp = 0x10000 + ((c - 0xD800) << 10) + (s.codeUnitAt(i + 1) - 0xDC00);
    out[o] = 0xF0 | (cp >>> 18);
    out[o + 1] = 0x80 | ((cp >>> 12) & 0x3F);
    out[o + 2] = 0x80 | ((cp >>> 6) & 0x3F);
    out[o + 3] = 0x80 | (cp & 0x3F);
    return o + 4;
  }
  out[o] = 0xE0 | (c >>> 12);
  out[o + 1] = 0x80 | ((c >>> 6) & 0x3F);
  out[o + 2] = 0x80 | (c & 0x3F);
  return o + 3;
}

/// Code units the code point at `s[i]` occupies (2 for a surrogate pair).
int utf16UnitsAt(String s, int i) {
  final c = s.codeUnitAt(i);
  return c >= 0xD800 && c <= 0xDBFF ? 2 : 1;
}

void _requirePair(String s, int i) {
  final c = s.codeUnitAt(i);
  final ok = c <= 0xDBFF && i + 1 < s.length && (s.codeUnitAt(i + 1) & 0xFC00) == 0xDC00;
  if (!ok) {
    throw _loneSurrogate(i);
  }
}
