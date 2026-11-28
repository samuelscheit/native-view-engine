#ifndef NVE_A11Y_WIN_H
#define NVE_A11Y_WIN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct NveSceneView;
typedef struct NveA11yWinHandle NveA11yWinHandle;

typedef enum NveA11yWinActionKind {
  NVE_A11Y_WIN_ACTION_CLICK = 1,
  NVE_A11Y_WIN_ACTION_FOCUS = 2,
  NVE_A11Y_WIN_ACTION_BLUR = 3,
  NVE_A11Y_WIN_ACTION_SET_VALUE = 4,
  NVE_A11Y_WIN_ACTION_SET_NUMERIC_VALUE = 5,
  NVE_A11Y_WIN_ACTION_REPLACE_SELECTED_TEXT = 6,
  NVE_A11Y_WIN_ACTION_SET_TEXT_SELECTION = 7,
  NVE_A11Y_WIN_ACTION_SCROLL_INTO_VIEW = 8,
  NVE_A11Y_WIN_ACTION_SCROLL_BY = 9,
  NVE_A11Y_WIN_ACTION_SET_SCROLL_OFFSET = 10,
  NVE_A11Y_WIN_ACTION_INCREMENT = 11,
  NVE_A11Y_WIN_ACTION_DECREMENT = 12,
  NVE_A11Y_WIN_ACTION_EXPAND = 13,
  NVE_A11Y_WIN_ACTION_COLLAPSE = 14,
  NVE_A11Y_WIN_ACTION_SHOW_CONTEXT_MENU = 15,
} NveA11yWinActionKind;

typedef struct NveA11yWinAction {
  uint32_t node_id;
  uint32_t kind;
  float x;
  float y;
  double numeric_value;
  size_t anchor_char;
  size_t focus_char;
  const uint8_t* text_ptr;
  size_t text_len;
} NveA11yWinAction;

/* Invoked while the internal callback-teardown lock is held (and potentially from a COM MTA worker
 * thread on Windows). Implementations MUST copy `text_ptr[0..text_len]` and enqueue/post to the UI
 * thread; they MUST NOT synchronously call back into any `nve_a11y_win_*` function or destroy the
 * owning window/handle. */
typedef void (*NveA11yWinActionCallback)(void* ctx, const NveA11yWinAction* action);

/* Borrows resource bytes from the caller's live scene guard for the duration of `sync_scene`. */
typedef int (*NveA11yResourceLookupFn)(void* ctx,
                                       uint32_t resource_id,
                                       uint64_t expected_kind,
                                       const uint8_t** out_ptr,
                                       size_t* out_len);

/* Must be called right after `CreateWindowExW` (never inside `WM_GETOBJECT`). */
NveA11yWinHandle* nve_a11y_win_create(void* hwnd,
                                      int is_window_focused,
                                      NveA11yWinActionCallback on_action,
                                      void* action_ctx);

/* Detaches the action callback atomically before freeing the UIA adapter; safe on NULL. */
void nve_a11y_win_destroy(NveA11yWinHandle* handle);

/* Returns 1 if `lparam` targets the client/UIA root and should be routed through this adapter. */
int nve_a11y_win_wants_wm_getobject(intptr_t lparam);

/* Returns 1 once an assistive technology has requested the tree (or `activate` was called). */
int nve_a11y_win_is_active(const NveA11yWinHandle* handle);

/* Marks the bridge active so the next `sync_scene` mirrors the scene ahead of `handle_wm_getobject`. */
void nve_a11y_win_activate(NveA11yWinHandle* handle);

/* Mirrors `scene` when active and pushes any incremental UIA update. Returns 1 if an update fired.
 * Preconditions (guaranteed by `nve-runtime`): `scene->scroll_offsets` is sorted ascending by
 * `node_id` and `scene->dirty_resources` is sorted ascending by resource id. */
int nve_a11y_win_sync_scene(NveA11yWinHandle* handle,
                            const struct NveSceneView* scene,
                            NveA11yResourceLookupFn res_lookup,
                            void* res_ctx,
                            uint32_t focused_node_id,
                            float scale_factor,
                            const char* window_title_utf8);

/* Updates top-level HWND focus state (call from `WM_SETFOCUS` / `WM_KILLFOCUS`). */
void nve_a11y_win_set_window_focus(NveA11yWinHandle* handle, int is_focused);

/* Handles `WM_GETOBJECT`. Returns 1 and writes `*out_lresult` when handled, 0 for `DefWindowProc`. */
int nve_a11y_win_handle_wm_getobject(NveA11yWinHandle* handle,
                                     uintptr_t wparam,
                                     intptr_t lparam,
                                     intptr_t* out_lresult);

#ifdef __cplusplus
}
#endif

#endif /* NVE_A11Y_WIN_H */
