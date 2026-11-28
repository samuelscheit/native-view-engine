// `nve codegen abi` output against this package's static runtime: every generated table key is
// `xxh64(name)`, every field spec fits its row, the wasm32 `SceneLayout` matches the generated
// struct layouts, and a `TableReader` reads a generated default row with a generated field spec.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  Key64,
  AccessibilityTable,
  LayoutTable,
  NVE_LAYOUT_RECT_LAYOUT,
  NVE_NODE_ROW_LAYOUT,
  NVE_SCENE_LAYOUT_WASM32,
  NVE_SCENE_VIEW_LAYOUT,
  NVE_TABLE_VIEW_LAYOUT,
  NVE_TABLES,
  TableReader,
} from '../src/index.ts';

test('generated tables agree with the runtime', () => {
  assert.ok(NVE_TABLES.length > 0);
  for (const t of NVE_TABLES) {
    assert.ok(t.key.equals(Key64.of(t.name)), `${t.name} key is xxh64(name)`);
    assert.ok(t.words >= 1 && t.words <= 32, `${t.name} word count`);
    for (const f of Object.values(t.fields)) assert.ok(f.fits(t.words), `${t.name} field fits its row`);
  }
});

test('wasm32 scene layout matches the generated struct layouts', () => {
  const { size: _viewSize, ...viewFields } = NVE_SCENE_VIEW_LAYOUT;
  const { tableView, ...sceneFields } = NVE_SCENE_LAYOUT_WASM32;
  assert.deepEqual(sceneFields, viewFields);
  const { size: tableViewSize, ...tableFields } = NVE_TABLE_VIEW_LAYOUT;
  const { stride, ...tableViewFields } = tableView;
  assert.equal(stride, tableViewSize);
  assert.deepEqual(tableViewFields, tableFields);
  assert.equal(NVE_NODE_ROW_LAYOUT.size, 32);
  assert.equal(NVE_LAYOUT_RECT_LAYOUT.size, 16);
});

test('a generated field spec reads a generated default row', () => {
  const rows = new DataView(new ArrayBuffer(LayoutTable.words * 4));
  LayoutTable.defaults.forEach((w, i) => rows.setUint32(i * 4, w, true));
  const layout = new TableReader({ key: LayoutTable.key, slot: 0, words: LayoutTable.words, rowCount: 1, rows });
  assert.equal(layout.f32(0, LayoutTable.fields.flexShrink), 1);
});

test('a generated two-word Str spec reads its span', () => {
  const label = AccessibilityTable.fields.label;
  const rows = new DataView(new ArrayBuffer(AccessibilityTable.words * 4));
  rows.setUint32(label.word * 4, 5, true);
  rows.setUint32((label.word + 1) * 4, 7, true);
  const a11y = new TableReader({ key: AccessibilityTable.key, slot: 0, words: AccessibilityTable.words, rowCount: 1, rows });
  assert.deepEqual(a11y.str(0, label), { offset: 5, len: 7 });
});
