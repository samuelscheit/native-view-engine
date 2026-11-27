// `[T; N]` (Dart `List<T>`) over the generated `dart:ffi` bindings, against the live Rust provider
// (`args.first` is its cdylib): the `NveBinValue` codec (lossless; wrong lengths rejected on encode
// with `RangeError` and on decode with `truncated` / `tooLarge`, inner rows too), POD
// `ffi.Array`s (nested ones via `@ffi.Array.multi`, `N = 1` too) through `toAbi` / `fromAbi`,
// `Option<[T; N]>` arguments and returns, `Option<PodStruct>`, `Result<Option<[T; N]>, String>`,
// and `[T; N]` view props.
import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:nve_fixed_array/fixed_codec_hybrid.generated.dart';
import 'package:nve_host/nve_host.dart';

void expect(bool cond, String what) {
  if (!cond) {
    throw StateError('FAILED: $what');
  }
}

/// Deep equality of nested lists and of the generated classes (compared by their encoding).
bool same(Object? a, Object? b) {
  if (a is List && b is List) {
    if (a.length != b.length) {
      return false;
    }
    for (var i = 0; i < a.length; i++) {
      if (!same(a[i], b[i])) {
        return false;
      }
    }
    return true;
  }
  if (a is FixedPoint && b is FixedPoint) {
    return a.x == b.x && a.y == b.y;
  }
  if (a is FixedPod && b is FixedPod) {
    return same(a.v, b.v) &&
        same(a.points, b.points) &&
        same(a.kinds, b.kinds) &&
        same(a.grid, b.grid) &&
        same(a.flags, b.flags) &&
        same(a.one, b.one) &&
        same(a.cells, b.cells);
  }
  return a == b;
}

void expectThrows<T>(String what, void Function() body) {
  try {
    body();
  } on Object catch (e) {
    expect(e is T, '$what threw ${e.runtimeType}: $e, expected $T');
    return;
  }
  throw StateError('FAILED: $what did not throw');
}

void expectBinError(BinErrorKind kind, String what, void Function() body) {
  try {
    body();
  } on BinCodecException catch (e) {
    expect(e.kind == kind, '$what failed with ${e.kind}, expected $kind');
    return;
  }
  throw StateError('FAILED: $what did not throw');
}

FixedBundle sampleBundle() => const FixedBundle(
      label: 'fixed',
      coords: <int>[1, 0xFFFFFFFF, 0, 42],
      points: <FixedPoint>[
        FixedPoint(x: -7, y: 8),
        FixedPoint(x: 2147483647, y: -2147483648),
      ],
      matrix: <List<int>>[
        <int>[1, 2],
        <int>[3, 4],
        <int>[5, 6],
      ],
      tag: <int>[9, 0, 255],
    );

FixedPod samplePod({
  List<double> v = const <double>[1.5, -2.0, 0.0, 3.25],
  List<List<int>> grid = const <List<int>>[
    <int>[10, 11],
    <int>[12, 13],
    <int>[14, 15],
  ],
  List<List<FixedPoint>> cells = const <List<FixedPoint>>[
    <FixedPoint>[FixedPoint(x: 5, y: 6)],
    <FixedPoint>[FixedPoint(x: 7, y: 8)],
  ],
}) {
  final flags = List<List<bool>>.generate(2, (_) => List<bool>.filled(33, false));
  flags[0][0] = true;
  flags[0][32] = true;
  flags[1][5] = true;
  return FixedPod(
    v: v,
    points: const <FixedPoint>[FixedPoint(x: 1, y: 2), FixedPoint(x: 3, y: 4)],
    kinds: const <FixedKind>[FixedKind.gamma, FixedKind.beta],
    grid: grid,
    flags: flags,
    one: const <int>[21],
    cells: cells,
  );
}

/// `{"<key>": [rows]}` where `rows[i]` has `lens[i]` integers (or is one integer when `lens[i]`
/// is negative).
Uint8List bundleWith(String key, List<int> lens) {
  final w = BinWriter();
  final map = w.beginMap(1);
  w.writeMapKey(key);
  final rows = w.beginArray(lens.length);
  for (final len in lens) {
    if (len < 0) {
      w.writeI64(0);
      continue;
    }
    final cols = w.beginArray(len);
    for (var i = 0; i < len; i++) {
      w.writeI64(i);
    }
    w.end(cols);
  }
  w
    ..end(rows)
    ..end(map);
  return w.toBytes();
}

void codecChecks() {
  final bundle = sampleBundle();
  final w = BinWriter();
  bundle.encodeBin(w);
  final r = BinReader(w.toBytes());
  final back = FixedBundle.decodeBin(r);
  r.requireDone();
  expect(back.label == 'fixed' && same(back.coords, bundle.coords), 'round trip coords');
  expect(same(back.points, bundle.points) && same(back.matrix, bundle.matrix), 'round trip nested');
  expect(same(back.tag, bundle.tag), 'round trip tag');

  final empty = BinWriter();
  empty.end(empty.beginMap(0));
  final defaults = FixedBundle.decodeBin(BinReader(empty.toBytes()));
  expect(same(defaults.matrix, const <List<int>>[<int>[0, 0], <int>[0, 0], <int>[0, 0]]), 'defaults');

  for (final (key, lens, kind) in const <(String, List<int>, BinErrorKind?)>[
    ('coords', <int>[-1, -1, -1, -1], null),
    ('coords', <int>[-1, -1, -1], BinErrorKind.truncated),
    ('coords', <int>[-1, -1, -1, -1, -1], BinErrorKind.tooLarge),
    ('matrix', <int>[2, 2, 2], null),
    ('matrix', <int>[2, 1, 2], BinErrorKind.truncated),
    ('matrix', <int>[2, 3, 2], BinErrorKind.tooLarge),
    ('matrix', <int>[2, 2], BinErrorKind.truncated),
    ('tag', <int>[-1, -1, -1, -1], BinErrorKind.tooLarge),
  ]) {
    final bytes = bundleWith(key, lens);
    if (kind == null) {
      FixedBundle.decodeBin(BinReader(bytes));
    } else {
      expectBinError(kind, 'decoding $key $lens', () => FixedBundle.decodeBin(BinReader(bytes)));
    }
  }
  expectThrows<RangeError>('encoding a short matrix row', () {
    FixedBundle(
      label: '',
      coords: bundle.coords,
      points: bundle.points,
      matrix: const <List<int>>[<int>[1, 2], <int>[3], <int>[5, 6]],
      tag: null,
    ).encodeBin(BinWriter());
  });

  // POD `ffi.Array`s: element by element, nested and `N = 1` dimensions included.
  final pod = samplePod();
  final raw = pod.toAbi();
  expect(raw.v[3] == 3.25 && raw.points[1].y == 4 && raw.grid[2][1] == 15, 'toAbi copies elements');
  expect(raw.kinds[0] == FixedKind.gamma.rawValue && raw.flags[0][32] && !raw.flags[1][4], 'toAbi nested');
  expect(raw.one[0] == 21 && raw.cells[0][0].y == 6 && raw.cells[1][0].x == 7, 'toAbi N = 1');
  expect(same(FixedPod.fromAbi(raw), pod), 'fromAbi restores every element');
  expectThrows<RangeError>('toAbi of a 1-element v', () => samplePod(v: const <double>[1.0]).toAbi());
  expectThrows<RangeError>(
    'toAbi of a 3-element grid row',
    () => samplePod(grid: const <List<int>>[<int>[1, 2], <int>[3, 4], <int>[5, 6, 7]]).toAbi(),
  );
  expectThrows<RangeError>(
    'toAbi of an empty cells row',
    () => samplePod(cells: const <List<FixedPoint>>[<FixedPoint>[FixedPoint(x: 0, y: 0)], <FixedPoint>[]]).toAbi(),
  );
}

void liveChecks(ffi.DynamicLibrary lib) {
  final codec = FixedCodecHybridClient.resolve(lib);
  final echoed = codec.echo(sampleBundle());
  expect(echoed.label == 'fixed!' && same(echoed.coords, const <int>[42, 0, 0xFFFFFFFF, 1]), 'echo coords');
  expect(same(echoed.matrix, const <List<int>>[<int>[2, 1], <int>[4, 3], <int>[6, 5]]), 'echo matrix');
  expect(same(echoed.tag, const <int>[255, 0, 9]), 'echo tag');
  expect(same(codec.swap(const <int>[1, 2, 3, 4]), const <int>[4, 3, 2, 1]), 'swap');
  expectThrows<RangeError>('swap with 3 coords', () => codec.swap(const <int>[1, 2, 3]));
  expect(same(codec.tag(true), const <int>[7, 8, 255]) && codec.tag(false) == null, 'tag');
  expect(same(codec.tryTag(2), const <int>[1, 2, 3]) && codec.tryTag(1) == null, 'tryTag ok');
  var remote = false;
  try {
    codec.tryTag(0);
  } on HybridException catch (e) {
    remote = e.message == 'no tag';
  }
  expect(remote, 'tryTag error');
  expect(same(codec.retag(const <int>[1, 2, 255]), const <int>[0, 2, 1]) && codec.retag(null) == null, 'retag');

  final mirrored = codec.mirror(samplePod());
  expect(same(mirrored.v, const <double>[-1.5, 2.0, -0.0, -3.25]), 'mirror v');
  expect(same(mirrored.kinds, const <FixedKind>[FixedKind.beta, FixedKind.gamma]), 'mirror kinds');
  expect(same(mirrored.grid, const <List<int>>[<int>[11, 12], <int>[13, 14], <int>[15, 16]]), 'mirror grid');
  expect(!mirrored.flags[0][0] && mirrored.flags[0][1] && !mirrored.flags[1][5], 'mirror flags');
  expect(same(mirrored.one, const <int>[42]) && mirrored.cells[0][0].x == 7, 'mirror N = 1');
  expect(same(codec.maybeMirror(samplePod()), mirrored) && codec.maybeMirror(null) == null, 'maybeMirror');

  final view = FixedViewHybridClient.resolve(lib);
  view.setProps(FixedViewProps(coords: const <int>[5, 6, 7, 8], tag: const <int>[4, 5, 6], pod: samplePod()));
  final summary = view.summary();
  expect(summary.label == 'view:3' && same(summary.coords, const <int>[5, 6, 7, 8]), 'view coords');
  expect(same(summary.matrix, samplePod().grid) && same(summary.tag, const <int>[4, 5, 6]), 'view pod');
  view.setProps(FixedViewProps(coords: const <int>[5, 6, 7, 8], pod: samplePod()));
  expect(view.summary().tag == null, 'view absent tag');
  expectThrows<RangeError>('setProps with 1 coord', () => view.setProps(FixedViewProps(coords: const <int>[1])));
  codec.dispose();
  view.dispose();
}

void main(List<String> args) {
  codecChecks();
  liveChecks(ffi.DynamicLibrary.open(args.first));
  print('dart fixed array ok');
}
