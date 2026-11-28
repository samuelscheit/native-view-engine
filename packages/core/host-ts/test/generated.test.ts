// `nve codegen abi` output against this package's static runtime: every generated table key is
// `xxh64(name)`, every field spec fits its row, generated `EventSet` / `inherited` metadata is
// coherent, the wasm32 `SceneLayout` / `NveEvent` layouts match the generated struct layouts, and
// a `TableReader` reads generated default rows with generated field specs.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  AccessibilityTable,
  GridItemTable,
  GridLineKind,
  ImageTable,
  InteractionTable,
  Key64,
  LayoutTable,
  NO_VAL_WORD,
  NVE_EVENT_BATCH_LAYOUT,
  NVE_EVENT_INLINE_KEY_CAP,
  NVE_EVENT_LAYOUT,
  NVE_EVENT_SIZE,
  NVE_LAYOUT_RECT_LAYOUT,
  NVE_NODE_ROW_LAYOUT,
  NVE_SCENE_LAYOUT_WASM32,
  NVE_SCENE_VIEW_LAYOUT,
  NVE_TABLE_VIEW_LAYOUT,
  NVE_TABLES,
  RAW_EVENT_BYTES_LAYOUT,
  ScrollTable,
  TableReader,
  TextInputTable,
  TypographyTable,
} from '../src/index.ts';

function defaultReader(t: { readonly key: Key64; readonly words: number; readonly defaults: readonly number[] }): TableReader {
  const rows = new DataView(new ArrayBuffer(t.words * 4));
  t.defaults.forEach((w, i) => rows.setUint32(i * 4, w >>> 0, true));
  return new TableReader({ key: t.key, slot: 0, words: t.words, rowCount: 1, rows });
}

test('generated tables agree with the runtime', () => {
  assert.ok(NVE_TABLES.length > 0);
  for (const t of NVE_TABLES) {
    assert.ok(t.key.equals(Key64.of(t.name)), `${t.name} key is xxh64(name)`);
    assert.ok(t.words >= 1 && t.words <= 32, `${t.name} word count`);
    assert.ok(
      t.eventsWord === NO_VAL_WORD || (t.eventsWord >= 0 && t.eventsWord < t.words),
      `${t.name} eventsWord in range`,
    );
    if (t.inherited) assert.equal(t.eventsWord, NO_VAL_WORD, `${t.name} cannot combine inherited and events`);
    for (const f of Object.values(t.fields)) {
      assert.ok(f.fits(t.words), `${t.name} field fits its row`);
      if (t.inherited) {
        assert.ok(f.explicitBit >= 0 && f.explicitBit < 32, `${t.name} inherited field has explicitBit`);
      } else {
        assert.equal(f.explicitBit, NO_VAL_WORD, `${t.name} non-inherited field has no explicitBit`);
        assert.equal(f.inherits, false, `${t.name} non-inherited field does not inherit`);
      }
    }
  }
});

test('wasm32 scene and event layouts match the generated struct layouts', () => {
  const { size: _viewSize, ...viewFields } = NVE_SCENE_VIEW_LAYOUT;
  const { tableView, ...sceneFields } = NVE_SCENE_LAYOUT_WASM32;
  assert.deepEqual(sceneFields, viewFields);
  const { size: tableViewSize, ...tableFields } = NVE_TABLE_VIEW_LAYOUT;
  const { stride, ...tableViewFields } = tableView;
  assert.equal(stride, tableViewSize);
  assert.deepEqual(tableViewFields, tableFields);
  assert.equal(NVE_NODE_ROW_LAYOUT.size, 32);
  assert.equal(NVE_LAYOUT_RECT_LAYOUT.size, 16);
  assert.equal(NVE_EVENT_SIZE, 64);
  assert.equal(NVE_EVENT_INLINE_KEY_CAP, 16);
  assert.deepEqual(NVE_EVENT_LAYOUT, {
    size: 64,
    timestamp: 0,
    table: 8,
    surface: 16,
    node: 20,
    event: 24,
    flags: 28,
    payload: 32,
    bytes: 56,
  });
  assert.deepEqual(RAW_EVENT_BYTES_LAYOUT, { size: 8, offset: 0, len: 4 });
  assert.deepEqual(NVE_EVENT_BATCH_LAYOUT, { size: 16, events: 0, bytes: 4, eventCount: 8, bytesLen: 12 });
});

test('generated field specs read generated default rows (optional, GridLine, EventSet, inherited Typography)', () => {
  const layout = defaultReader(LayoutTable);
  assert.equal(layout.f32(0, LayoutTable.fields.flexShrink), 1);
  assert.equal(layout.optF32(0, LayoutTable.fields.aspectRatio), null);
  assert.equal(LayoutTable.eventsWord, 0);
  assert.equal(layout.eventMask(0, LayoutTable.eventsWord), 0);

  const gridItem = defaultReader(GridItemTable);
  assert.deepEqual(gridItem.gridLine(0, GridItemTable.fields.columnStart), { kind: GridLineKind.Auto, value: 0 });
  assert.deepEqual(gridItem.gridLine(0, GridItemTable.fields.rowEnd), { kind: GridLineKind.Auto, value: 0 });

  const typo = defaultReader(TypographyTable);
  assert.equal(TypographyTable.inherited, true);
  assert.equal(typo.color(0, TypographyTable.fields.color), 0xff00_0000);
  assert.equal(typo.f32(0, TypographyTable.fields.fontSize), 16);
  assert.equal(typo.optU32(0, TypographyTable.fields.maxLines), null);
  assert.equal(typo.isExplicit(0, TypographyTable.fields.color), false);

  const interaction = defaultReader(InteractionTable);
  assert.equal(InteractionTable.eventsWord, 0);
  assert.equal(interaction.optI32(0, InteractionTable.fields.tabIndex), null);
  assert.equal(ScrollTable.eventsWord, 0);
  assert.equal(TextInputTable.eventsWord, 0);

  const a11y = defaultReader(AccessibilityTable);
  assert.equal(a11y.optEnum(0, AccessibilityTable.fields.role), null);
  assert.equal(a11y.optU32(0, AccessibilityTable.fields.level), null);
  assert.equal(a11y.optF32(0, AccessibilityTable.fields.valueNow), null);

  const image = defaultReader(ImageTable);
  assert.equal(image.optColor(0, ImageTable.fields.tint), null);
  assert.equal(image.optU32(0, ImageTable.fields.intrinsicWidth), null);
});

test('a generated two-word Str spec reads its span', () => {
  const label = AccessibilityTable.fields.label;
  const rows = new DataView(new ArrayBuffer(AccessibilityTable.words * 4));
  rows.setUint32(label.word * 4, 5, true);
  rows.setUint32((label.word + 1) * 4, 7, true);
  const a11y = new TableReader({ key: AccessibilityTable.key, slot: 0, words: AccessibilityTable.words, rowCount: 1, rows });
  assert.deepEqual(a11y.str(0, label), { offset: 5, len: 7 });
});
