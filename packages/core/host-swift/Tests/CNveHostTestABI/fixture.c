#include "nve_host_test_abi.h"

/* key = Key64::of("nve.visual") = 0x33E73115985A4FD6. Row layout matches NveHostTests.swift. */
static const uint32_t kRows[12] = {
    200u | (0xFFDu << 8) | (1u << 20), 0x42480001u, 0x80402010u, 0u, 3u, 0x3FC00000u,
    2047u << 8,                         2u,          0u,          3u, 0u, 0x80000000u};
static const NveTestTableView kTables[1] = {
    {0x985A4FD6u, 0x33E73115u, 0u, 6u, 2u, 0u, kRows, NULL, NULL}};
static const NveTestNode kNodes[3] = {{1u, 0u, 3u, 2u, 0u}, {2u, 3u, 4u, 0u, 2u}, {3u, 7u, 0u, 0u, 2u}};
static const uint8_t kText[7] = {'h', 0xC3, 0xA9, 0xF0, 0x9F, 0x9A, 0x80};
static const uint32_t kChildren[2] = {2u, 3u};
static const uint32_t kDirty[3] = {0u, 2u, 99u};
static const uint32_t kRemoved[1] = {42u};

static const NveTestSceneView kScene = {
    UINT64_C(0xF00DFACEC0FFEE11), 7u, 3u, 3u, 1u, 7u, 2u, 1u, kNodes, kText, kChildren, kDirty, kRemoved, kTables};

const NveTestSceneView* nve_test_scene(void) { return &kScene; }
