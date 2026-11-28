//! State-preserving DOM moves (`wasm32`). `ParentNode.moveBefore()` keeps a connected subtree's
//! imperative state (top layer `<dialog>` / `[popover]`, focus, `<iframe>` documents) across a
//! reorder or retag; the `insertBefore()` / `appendChild()` fallback runs the removing steps, so
//! the caller re-syncs every plugin node of the moved subtree.

use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(inline_js = "
function nveMove(parent, child, before) {
  if (typeof parent.moveBefore === 'function' && parent.isConnected && child.isConnected) {
    try { parent.moveBefore(child, before); return true; } catch (_) {}
  }
  return false;
}
export function nve_place_child(parent, child, before) {
  if (nveMove(parent, child, before)) return true;
  try { parent.insertBefore(child, before); } catch (_) {}
  return false;
}
export function nve_retag_element(oldEl, freshEl) {
  const parent = oldEl.parentNode;
  let preserved = false;
  if (parent && oldEl.isConnected) {
    parent.insertBefore(freshEl, oldEl);
    preserved = true;
    while (preserved && oldEl.firstChild) preserved = nveMove(freshEl, oldEl.firstChild, null);
  }
  while (oldEl.firstChild) freshEl.appendChild(oldEl.firstChild);
  if (freshEl.parentNode) oldEl.remove(); else oldEl.replaceWith(freshEl);
  return preserved;
}
")]
extern "C" {
    /// Places `child` before `before` (append when `None`) inside `parent`; `true` when a
    /// connected `child` moved with `moveBefore()` (state preserved).
    #[wasm_bindgen(js_name = nve_place_child)]
    pub(crate) fn place_child(
        parent: &web_sys::Element,
        child: &web_sys::Element,
        before: Option<&web_sys::Node>,
    ) -> bool;

    /// Replaces `old` with `fresh` (a retag), moving `old`'s children into `fresh`; `true` when
    /// every child moved with `moveBefore()` (descendant state preserved).
    #[wasm_bindgen(js_name = nve_retag_element)]
    pub(crate) fn retag_element(old: &web_sys::Element, fresh: &web_sys::Element) -> bool;
}
