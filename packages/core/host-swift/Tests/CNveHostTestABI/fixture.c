#include "nve_host_test_abi.h"

/* key = Key64::of("nve.visual") = 0x33E73115985A4FD6. Row layout matches TableSceneTests.swift. */
static const uint32_t kTableRows[12] = {
    200u | (0xFFDu << 8) | (1u << 20), 0x42480001u, 0x80402010u, 0u, 3u, 0x3FC00000u,
    2047u << 8,                         2u,          0u,          3u, 0u, 0x80000000u};
static const NveTestTableView kTables[1] = {
    {0x985A4FD6u, 0x33E73115u, 5u, 6u, 2u, 0u, kTableRows, NULL, NULL}};
static const NveTestNodeRow kRows[4] = {
    {{0u, 0u, 0u, 0u}, 0u, 0u, 0u, 0u, 0u, 0u, 0u},
    {{1u << 5, 0u, 0u, 0u}, 0u, 0u, 2u, 0u, 0xFFu, 0u, 1u},
    {{1u << 5, 1u << 1, 0u, 0x80000000u}, 1u, 2u, 0u, 1u, 5u, 4u, 1u},
    {{0u, 0u, 0u, 0u}, 0u, 0u, 0u, 2u, 0xFFu, 0u, 0u}};
static const NveTestLayoutRect kRects[4] = {
    {0.0f, 0.0f, 0.0f, 0.0f}, {0.0f, 0.0f, 320.0f, 240.0f}, {10.0f, 20.0f, 100.0f, 50.0f}, {0.0f, 0.0f, 0.0f, 0.0f}};
static const uint8_t kText[7] = {'h', 0xC3, 0xA9, 0xF0, 0x9F, 0x9A, 0x80};
static const uint32_t kChildren[2] = {0x01000002u, 0x02000003u};
static const uint32_t kDirty[3] = {1u, 2u, 99u};
static const uint32_t kRemoved[1] = {0x02000003u};

static const NveTestScrollOffset kScroll[1] = {{1u, 0.0f, 12.5f}};

static const uint32_t kDirtyResources[2] = {3u, 9u};

static const NveTestRuntimeSceneView kScene = {
    UINT64_C(0xF00DFACEC0FFEE11), 7u, 4u, 3u, 1u, 7u, 2u, 1u, 1u, 5u, 2u, 0u,
    kRows, kRects, kText, kChildren, kDirty, kRemoved, kTables, kScroll, kDirtyResources};

const NveTestRuntimeSceneView* nve_test_scene(void) { return &kScene; }