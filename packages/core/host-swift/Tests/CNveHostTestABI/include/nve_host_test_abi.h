/* Test-only C mirror of the generated `NveTableView` / `NveSceneView` shapes (Rust `offset_of!`
 * layout), proving the C-imported structs satisfy `TableViewABI` / `SceneViewABI` unchanged. */
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

typedef struct NveTestNode {
  uint32_t node_id;
  uint32_t text_offset;
  uint16_t text_len;
  uint16_t child_count;
  uint32_t child_offset;
} NveTestNode;

typedef struct NveTestSceneView {
  uint64_t schema_hash;
  uint64_t frame_id;
  uint32_t node_count;
  uint32_t dirty_count;
  uint32_t removed_count;
  uint32_t text_slab_len;
  uint32_t children_slab_len;
  uint32_t table_count;
  const NveTestNode* nodes;
  const uint8_t* text_slab;
  const uint32_t* children_slab;
  const uint32_t* dirty_indices;
  const uint32_t* removed_ids;
  const NveTestTableView* tables;
} NveTestSceneView;

_Static_assert(offsetof(NveTestSceneView, table_count) == 36, "NveSceneView.table_count @36");
_Static_assert(offsetof(NveTestSceneView, nodes) == 40, "NveSceneView.nodes @40");
_Static_assert(offsetof(NveTestTableView, rows) == 24, "NveTableView.rows @24");

/* Static fixture frame published from C memory (schema 0xF00DFACEC0FFEE11, frame 7). */
const NveTestSceneView* nve_test_scene(void);

#endif /* NVE_HOST_TEST_ABI_H */
