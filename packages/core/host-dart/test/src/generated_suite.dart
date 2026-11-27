import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'check.dart';

/// `nve codegen abi` output against this package's static runtime: every generated table key is
/// `xxh64(name)`, every field spec fits its row, the wasm32 `SceneLayout` matches the generated
/// struct layouts, and a `TableReader` reads a generated default row with a generated field spec.
void runGenerated(Check c) {
  for (final t in nveTables) {
    c.eq(t.key, Key64.of(t.name), '${t.name} key is xxh64(name)');
    c.that(t.words >= 1 && t.words <= maxTableWords, '${t.name} word count');
    for (final f in t.fields) {
      c.that(f.fits(t.words), '${t.name} field fits its row');
    }
  }
  c.eq(nveSceneLayoutWasm32.nodeStride, NveRenderNodeLayout.size, 'node stride');
  c.eq(nveSceneLayoutWasm32.tables, NveSceneViewLayout.tables, 'tables offset');
  c.eq(nveSceneLayoutWasm32.tableView.stride, NveTableViewLayout.size, 'table view stride');

  final rows = ByteData(LayoutTable.words * 4);
  for (var i = 0; i < LayoutTable.words; i++) {
    rows.setUint32(i * 4, LayoutTable.defaults[i], Endian.little);
  }
  final layout = TableReader(key: LayoutTable.key, slot: 0, words: LayoutTable.words, rowCount: 1, rows: rows);
  c.eq(layout.f32(0, LayoutTable.flexShrink), 1.0, 'Layout.flex_shrink default');

  // A generated two-word `str` spec reads its (offset, len) span.
  final label = AccessibilityTable.label;
  final a11yRows = ByteData(AccessibilityTable.words * 4)
    ..setUint32(label.word * 4, 5, Endian.little)
    ..setUint32((label.word + 1) * 4, 7, Endian.little);
  final a11y = TableReader(key: AccessibilityTable.key, slot: 0, words: AccessibilityTable.words, rowCount: 1, rows: a11yRows);
  c.eq(a11y.str(0, label), (5, 7), 'Accessibility.label span');
}
