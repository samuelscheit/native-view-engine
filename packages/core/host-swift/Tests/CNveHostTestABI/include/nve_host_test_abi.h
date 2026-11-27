/* Test-only C mirror of the generated `NveTableView` / `NveNodeRow` / `NveLayoutRect` /
 * `NveRuntimeSceneView` shapes (Rust `offset_of!` layout), proving the C-imported structs satisfy
 * `TableViewABI` / `NodeRowABI` / `LayoutRectABI` / `RuntimeSceneViewABI` unchanged. */
#ifndef NVE_HOST_TEST_ABI_H
#define NVE_HOST_TEST_ABI_H

#include <stddef.h>
#include <stdint.h>

typedef struct NveTestTableView {
  uint32_t key_lo;
  uint32_t key_hi;
  uint32_t slot;
  uint32_t words;
  uint32_t row_count;
  uint32_t dirty_count;
  const uint32_t* rows;
  const uint32_t* owners;
  const uint32_t* dirty_rows;
} NveTestTableView;

typedef struct NveTestNodeRow {
  uint32_t table_set[4];
  uint32_t parent;
  uint32_t child_off;
  uint32_t child_len;
  uint8_t generation;
  uint8_t primary_slot;
  uint8_t effects;
  uint8_t flags;
} NveTestNodeRow;

typedef struct NveTestLayoutRect {
  float x;
  float y;
  float width;
  float height;
} NveTestLayoutRect;

typedef struct NveTestScrollOffset {
  uint32_t node_id;
  float x;
  float y;
} NveTestScrollOffset;

typedef struct NveTestRuntimeSceneView {
  uint64_t schema_hash;
  uint64_t frame_id;
  uint32_t node_capacity;
  uint32_t dirty_count;
  uint32_t removed_count;
  uint32_t text_slab_len;
  uint32_t children_slab_len;
  uint32_t table_count;
  uint32_t scroll_count;
  uint32_t _reserved;
  const NveTestNodeRow* rows;
  const NveTestLayoutRect* rects;
  const uint8_t* text_slab;
  const uint32_t* children_slab;
  const uint32_t* dirty_indices;
  const uint32_t* removed_ids;
  const NveTestTableView* tables;
  const NveTestScrollOffset* scroll_offsets;
} NveTestRuntimeSceneView;

_Static_assert(sizeof(NveTestNodeRow) == 32, "NveNodeRow is 32 bytes");
_Static_assert(offsetof(NveTestNodeRow, parent) == 16, "NveNodeRow.parent @16");
_Static_assert(offsetof(NveTestNodeRow, flags) == 31, "NveNodeRow.flags @31");
_Static_assert(sizeof(NveTestLayoutRect) == 16, "NveLayoutRect is 16 bytes");
_Static_assert(offsetof(NveTestRuntimeSceneView, table_count) == 36, "NveRuntimeSceneView.table_count @36");
_Static_assert(offsetof(NveTestRuntimeSceneView, rows) == 48, "NveRuntimeSceneView.rows @48");
_Static_assert(offsetof(NveTestRuntimeSceneView, tables) == 96, "NveRuntimeSceneView.tables @96");
_Static_assert(offsetof(NveTestRuntimeSceneView, scroll_offsets) == 104, "NveRuntimeSceneView.scroll_offsets @104");
_Static_assert(sizeof(NveTestRuntimeSceneView) == 112, "NveRuntimeSceneView is 112 bytes");
_Static_assert(offsetof(NveTestTableView, rows) == 24, "NveTableView.rows @24");

/* Static fixture frame published from C memory (schema 0xF00DFACEC0FFEE11, frame 7): slot 0 null,
 * slot 1 live root (children 2v1, 3v2), slot 2 live leaf generation 1, slot 3 dead generation 2. */
const NveTestRuntimeSceneView* nve_test_scene(void);

/* Counts every malloc-family allocation made by the calling thread between begin and end (freed
 * temporaries included). `begin` returns 0 where the platform has no hook; `end` then returns -1. */
int nve_test_alloc_count_begin(void);
int64_t nve_test_alloc_count_end(void);

#endif /* NVE_HOST_TEST_ABI_H */
