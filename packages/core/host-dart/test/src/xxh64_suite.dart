import 'dart:convert' show utf8;

import 'package:nve_host/nve_host.dart';
import 'package:nve_host/src/key64.dart' show write32LE;

import 'check.dart';

const Key64 prime32 = Key64(0x9E3779B1, 0);
const Key64 wireSeed = Key64(0x49524532, 0x4E564557);

List<int> seq(int n) => List<int>.generate(n, (i) => (i * 31 + 7) & 0xFF);

/// Official Cyan4973 xxHash sanity buffer (BigInt is exact on the VM and in JS).
List<int> cyan(int n) {
  final mask = (BigInt.one << 64) - BigInt.one;
  final p1 = BigInt.parse('9E3779B185EBCA87', radix: 16);
  var gen = BigInt.from(0x9E3779B1);
  return List<int>.generate(n, (_) {
    final b = (gen >> 56).toInt() & 0xFF;
    gen = (gen * p1) & mask;
    return b;
  });
}

/// Golden vectors computed by the canonical Rust `nve-hash` crate: (label, bytes, seed, expected).
final List<(String, List<int>, Key64, Key64)> vectors = [
  ('empty', <int>[], Key64.zero, const Key64(0x51D8E999, 0xEF46DB37)),
  ('a', utf8.encode('a'), Key64.zero, const Key64(0xA98C6E5B, 0xD24EC4F1)),
  ('abc', utf8.encode('abc'), Key64.zero, const Key64(0xAD770999, 0x44BC2CF5)),
  ('nve.visual', utf8.encode('nve.visual'), Key64.zero, const Key64(0x985A4FD6, 0x33E73115)),
  ('high_bits', <int>[0x00, 0x01, 0x7F, 0x80, 0xFF], Key64.zero, const Key64(0x99A141EA, 0x8BABCAB4)),
  ('utf8', utf8.encode('héllo wörld — 日本語 🚀'), Key64.zero, const Key64(0xF213F592, 0x72AC442F)),
  ('seq101', seq(101), Key64.zero, const Key64(0x1A53BC7F, 0xFBFDF3FA)),
  ('seq101_wire', seq(101), wireSeed, const Key64(0x8C3DB1BC, 0x277E092A)),
  ('seq37_prime1', seq(37), Xxh64Primes.prime1, const Key64(0x85FC58A4, 0x33DDB9A8)),
  ('seq1024', seq(1024), Key64.zero, const Key64(0x72CDAE00, 0x149AA449)),
  ('cyan1', cyan(1), Key64.zero, const Key64(0xDB052768, 0xE934A84A)),
  ('cyan4', cyan(4), Key64.zero, const Key64(0xF81E29E4, 0xE244DDBC)),
  ('cyan14', cyan(14), Key64.zero, const Key64(0xE0BD0A0A, 0xB89B3598)),
  ('cyan222', cyan(222), Key64.zero, const Key64(0x30BCEC3A, 0x06CC5BD9)),
  ('cyan1024', cyan(1024), Key64.zero, const Key64(0xE063991A, 0x2E307F2F)),
  ('cyan1_p32', cyan(1), prime32, const Key64(0x43A9B4C3, 0x50146076)),
  ('cyan222_p32', cyan(222), prime32, const Key64(0xDF89AF66, 0x0B105469)),
  ('cyan32_p1', cyan(32), Xxh64Primes.prime1, const Key64(0xB7E278ED, 0x6D0242B0)),
  ('cyan1024_p1', cyan(1024), Xxh64Primes.prime1, const Key64(0x9A39421C, 0xF464C3F6)),
];

void runXxh64(Check c) {
  for (final (label, bytes, seed, expected) in vectors) {
    c.eq(xxh64(bytes, seed), expected, label);
    for (final step in const [1, 3, 7, 13, 24, 31, 32, 33, 64, 100]) {
      final h = Xxh64Hasher(seed);
      for (var at = 0; at < bytes.length; at += step) {
        h.update(bytes, at, at + step < bytes.length ? at + step : bytes.length);
      }
      c.eq(h.digest(), expected, '$label step $step');
    }
  }
  c.throwsA<RangeError>(() => Xxh64Hasher().update(<int>[1, 2], 1, 3), 'range past end');

  final h = Xxh64Hasher(const Key64(7, 0))..updateU32(0x04030201);
  c.eq(h.digest(), h.digest(), 'digest is non-consuming');
  h.updateU64(const Key64(0x08070605, 0x0C0B0A09));
  c.eq(h.digest(), xxh64(<int>[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], const Key64(7, 0)), 'typed feeds');

  // Key64 surface.
  final k = Key64.of('nve.visual');
  c.eq(k, const Key64(0x985A4FD6, 0x33E73115), 'Key64.of');
  c.eq(k.toHex(), '33e73115985a4fd6', 'toHex');
  c.eq(constId('abc'), const Key64(0xAD770999, 0x44BC2CF5), 'constId');
  c.eq(constId('héllo wörld — 日本語 🚀'), const Key64(0xF213F592, 0x72AC442F), 'constId encodes pairs in place');
  for (final lone in const ['\uD800', 'x\uDC00', '\uD83D', '\uDE80\uD83D']) {
    c.throwsA<BinCodecException>(() => constId(lone), 'constId lone surrogate');
  }
  // A rejected string feeds nothing: the hasher state is unchanged.
  final atomic = Xxh64Hasher()..updateUtf8('ab');
  final before = atomic.digest();
  c.throwsA<BinCodecException>(() => atomic.updateUtf8('abc\uD800'), 'updateUtf8 lone surrogate');
  c.eq(atomic.digest(), before, 'updateUtf8 is atomic');
  atomic.updateUtf8('c');
  c.eq(atomic.digest(), constId('abc'), 'hasher still usable after a rejection');
  final le = List<int>.filled(9, 0);
  k.writeLE(le, 1);
  c.eq(Key64.readLE(le, 1), k, 'LE round trip at odd offset');
  c.eq(le[1], 0xD6, 'LE low byte first');
  // A key that only half fits throws before writing anything.
  final zeroed = List<int>.filled(9, 0);
  c.throwsA<RangeError>(() => k.writeLE(zeroed, 2), 'writeLE past the end');
  c.throwsA<RangeError>(() => k.writeLE(zeroed, -1), 'writeLE before the start');
  c.throwsA<RangeError>(() => write32LE(zeroed, 6, 0xFFFFFFFF), 'write32LE past the end');
  c.that(zeroed.every((b) => b == 0), 'rejected writes leave every byte untouched');
  c.that(const Key64(0, 1).compareTo(const Key64(0xFFFFFFFF, 0)) > 0, 'compare by hi first');
  c.eq(const Key64(5, 0).toUnsignedInt(), 5, 'small unsigned');
  c.eq(const Key64(0, 0x1FFFFF).toUnsignedInt(), 0x1FFFFF * 0x100000000, 'below 2^53 is exact everywhere');
  c.eq(const Key64(0xFFFFFFFF, 0xFFFFFFFF).toSignedInt(), -1, 'i64 -1');
  c.eq(const Key64(0, 0xFFE00000).toSignedInt(), -0x200000 * 0x100000000, 'i64 -2^53 exact everywhere');
  c.eq(Key64.fromInt(-1), const Key64(0xFFFFFFFF, 0xFFFFFFFF), 'fromInt -1');
  c.eq(Key64.fromInt(0x1FFFFFFFFFFFFF), const Key64(0xFFFFFFFF, 0x1FFFFF), 'fromInt 2^53-1');
  c.eq(Key64.fromInt(-0x1FFFFFFFFFFFFF), const Key64(1, 0xFFE00000), 'fromInt -(2^53-1)');
  const isWeb = identical(0, 0.0);
  if (isWeb) {
    c.throwsA<StateError>(() => const Key64(0, 0x200000).toUnsignedInt(), '2^53 not exact on web');
    c.throwsA<StateError>(() => const Key64(0xFFFFFFFF, 0xFFDFFFFF).toSignedInt(), '< -2^53 not exact on web');
  } else {
    c.eq(const Key64(0, 0x200000).toUnsignedInt(), 0x20000000000000, '2^53 exact on VM');
    c.throwsA<StateError>(() => const Key64(0, 0x80000000).toUnsignedInt(), '2^63 not an int on VM');
  }
}
