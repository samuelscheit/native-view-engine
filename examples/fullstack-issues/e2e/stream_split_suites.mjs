/**
 * Suites 3 + 4 of the fullstack-issues Chrome E2E: byte-exact TCP splits and truncations of the
 * streaming SSR document through `stream_proxy.mjs`.
 *
 *  3. Mid-slot splits at every critical boundary of the `stats` slot chunk
 *     (`[NVEQ script][chunk <style>][<div slot>][ack]`), held while WASM boots, then released:
 *     never adopted early, 0 mismatches, exactly one `stats-total`, 0 RPCs.
 *  4. Truncations: after `seq=0` + `boot.js`, inside the chunk `<style>`, and inside the slot
 *     `<div>` (payload complete but never acked). Stream-pending queries fall back to exactly one
 *     client RPC each, and the stats panel is fully styled (no truncated CSS seeded).
 */

import {
  COMMENTS_RPC,
  STATS_RPC,
  assert,
  deepCountJs,
  evalJs,
  navigateDocument,
  sleep,
  visibleJs,
  waitFor,
  waitForEvent,
  waitUntil,
} from "./harness.mjs";

const STATS_SLOT = '<div slot="nve-slot-stats" data-nve-slot="stats"';
const STATS_STYLE = '<style data-nve-ssr-chunk="stats"';
const STATS_PAYLOAD = 'data-nve-slot-for="stats">';

/** Cuts inside each part of the stats slot chunk, in wire order. */
const SPLIT_CUTS = [
  { name: "inside-nveq-script", plan: { marker: STATS_PAYLOAD } },
  { name: "inside-chunk-style", plan: { marker: STATS_STYLE, cutAfter: "{flex-grow:1;" }, styleIncomplete: true },
  // Inside the `stats-total` attribute value: the parser holds an unfinished start tag.
  { name: "inside-slot-div", plan: { marker: STATS_SLOT, cutAfter: 'data-testid="stats-to' } },
  // The slot `<div>` is closed but its `data-nve-slot-ack` has not arrived.
  { name: "closed-but-unacked", plan: { marker: STATS_SLOT, cutBefore: '<template data-nve-slot-ack="stats">' } },
];

const TRUNCATE_CUTS = [
  { name: "inside-chunk-style", plan: { marker: STATS_STYLE, cutAfter: "{flex-grow:1;" } },
  { name: "inside-slot-div", plan: { marker: STATS_SLOT, cutAfter: 'data-testid="stats-to' } },
];

const statsTotalCount = deepCountJs('[data-testid="stats-total"]');

/** `stats-total` elements adopted into the live app (outside the hidden DSD host). */
const adoptedStatsTotalJs = `Array.from(document.querySelectorAll('[data-testid="stats-total"]'))
  .filter((el) => !el.closest('[data-nve-dsd-host]')).length`;

/** Computed box styles of the adopted `stats-total` tile (its chunk CSS must be whole). */
const statsTileStyleJs = `(() => {
  const el = Array.from(document.querySelectorAll('[data-testid="stats-total"]')).find((e) => !e.closest('[data-nve-dsd-host]'));
  if (!el) return null;
  const cs = getComputedStyle(el);
  return { border: cs.borderTopWidth + " " + cs.borderTopStyle, radius: cs.borderTopLeftRadius,
    bg: cs.backgroundColor, basis: cs.flexBasis, padding: cs.paddingTop + " " + cs.paddingLeft };
})()`;

function assertStyledStatsTile(tile, where) {
  const want = { border: "1px solid", radius: "10px", bg: "rgb(15, 23, 42)", basis: "110px", padding: "12px 14px" };
  assert(tile, `${where}: no adopted stats-total tile`);
  for (const [k, v] of Object.entries(want)) {
    assert(tile[k] === v, `${where}: stats-total ${k}=${tile[k]} (want ${v}) ${JSON.stringify(tile)}`);
  }
}

export async function runStreamSplitSuites(ctx) {
  const { cdp, proxy, control, commentsWaiting, rpcSince, rpcRequests, controller, waitMounted, assertCleanHydration, assertStrictCsp, shot } = ctx;

  // ---------------------------------------------------------------------------
  // 3. Mid-slot TCP splits at every critical stats-chunk boundary
  // ---------------------------------------------------------------------------
  console.log(`==> [3/9] Mid-slot TCP splits (${SPLIT_CUTS.length} stats-chunk boundaries) during WASM boot`);
  for (const { name, plan, styleIncomplete } of SPLIT_CUTS) {
    const where = `suite 3 (${name})`;
    await control("stats_delay_ms=30&comments_delay_ms=0&hold_comments=0");
    const cut = proxy.arm(plan);
    const mark = rpcRequests.length;
    const doc = await navigateDocument(cdp, `${proxy.origin}/issues`, { waitLoad: false });
    assert(doc.status === 200, `${where}: proxied GET /issues -> ${doc.status}`);
    assertStrictCsp(doc.headers, where);
    await cut;
    await waitMounted(`WASM mounted while split ${name}`);
    assert(proxy.isHeld(), `${where}: proxy released before mount`);
    assert((await evalJs(cdp, "document.readyState")) === "loading", `${where}: document finished before release`);
    assertCleanHydration(await controller(), `${where} held`);
    assert((await evalJs(cdp, adoptedStatsTotalJs)) === 0, `${where}: unacked stats chunk adopted early`);
    await waitFor(cdp, `${where}: stats skeleton while the chunk is incomplete`, visibleJs('[data-testid="stats-skeleton"]'));
    if (styleIncomplete) {
      const hoisted = await evalJs(cdp, `Boolean(document.head.querySelector('style[data-nve-ssr-chunk="stats"]'))`);
      assert(!hoisted, `${where}: truncated chunk <style> hoisted into <head>`);
    }
    await sleep(150);
    if (name === "inside-slot-div") await shot(cdp, "issues-mid-slot-split.png");
    proxy.release();
    await waitFor(cdp, `${where}: stats-total visible after release`, visibleJs('[data-testid="stats-total"]'));
    await waitFor(cdp, `${where}: DSD host removed`, `() => document.querySelector('[data-nve-dsd-host]') === null`);
    await sleep(250);
    const totals = await evalJs(cdp, statsTotalCount);
    assert(totals === 1, `${where}: expected exactly one stats-total, found ${totals}`);
    assertStyledStatsTile(await evalJs(cdp, statsTileStyleJs), where);
    assertCleanHydration(await controller(), `${where} released`);
    assert(rpcSince(mark, STATS_RPC) === 0, `${where}: ${rpcSince(mark, STATS_RPC)} Stats/Get RPCs`);
    assert(rpcSince(mark) === 0, `${where}: expected 0 RPCs, saw ${rpcSince(mark)}`);
    console.log(`  PASS (${name}): never adopted early; 1 styled stats-total, 0 mismatches/replaced, 0 RPCs.`);
  }

  // ---------------------------------------------------------------------------
  // 4. Truncated streams fall back to client RPCs on DOMContentLoaded
  // ---------------------------------------------------------------------------
  console.log("==> [4/9] Truncated streams (no stream-end) fall back to client RPCs on DOMContentLoaded");
  const bootCut = { name: "after-boot", plan: { marker: '<script type="module" async src="/pkg/issues/boot.js"></script>' } };
  for (const { name, plan } of [bootCut, ...TRUNCATE_CUTS]) {
    const where = `suite 4 (${name})`;
    await control("stats_delay_ms=30&comments_delay_ms=0&hold_comments=1");
    const cut = proxy.arm(plan);
    const mark = rpcRequests.length;
    const doc = await navigateDocument(cdp, `${proxy.origin}/issues`, { waitLoad: false });
    assert(doc.status === 200, `${where}: proxied GET /issues -> ${doc.status}`);
    await cut;
    await waitMounted(`WASM mounted on the to-be-truncated stream (${name})`);
    assert((await evalJs(cdp, "document.readyState")) === "loading", `${where}: document finished before truncation`);
    assert((await commentsWaiting()) >= 1, `${where}: comments barrier not held`);
    await sleep(200);
    assert(rpcSince(mark) === 0, `${where}: stream-pending queries fetched before DOMContentLoaded (${rpcSince(mark)})`);
    const domReady = waitForEvent(cdp, "Page.domContentEventFired", () => true, 10000, `DOMContentLoaded after truncation (${name})`);
    proxy.truncate();
    await domReady.promise;
    await waitUntil(`${where}: Comments/List RPC after DOMContentLoaded`, () => rpcSince(mark, COMMENTS_RPC) >= 1);
    await waitUntil(`${where}: Stats/Get RPC after DOMContentLoaded`, () => rpcSince(mark, STATS_RPC) >= 1);
    await waitFor(cdp, `${where}: stats panel rendered from client RPC`, visibleJs('[data-testid="stats-total"]'));
    await control("hold_comments=0");
    await waitFor(cdp, `${where}: featured comments rendered from client RPC`, visibleJs('[data-testid="featured-comments-panel"]'));
    await sleep(300);
    assert(await evalJs(cdp, `document.querySelector('[data-nve-dsd-host]') === null`), `${where}: DSD host left behind`);
    const truncatedStyles = await evalJs(cdp, `document.querySelectorAll('style[data-nve-ssr-chunk="stats"]').length`);
    assert(name !== "inside-chunk-style" || truncatedStyles === 0, `${where}: truncated chunk <style> survived (${truncatedStyles})`);
    // The slot-bound stats payload was never ingested: exactly one client fetch each.
    assert(rpcSince(mark, COMMENTS_RPC) === 1, `${where}: ${rpcSince(mark, COMMENTS_RPC)} Comments/List RPCs`);
    assert(rpcSince(mark, STATS_RPC) === 1, `${where}: ${rpcSince(mark, STATS_RPC)} Stats/Get RPCs`);
    assert((await evalJs(cdp, statsTotalCount)) === 1, `${where}: expected exactly one stats-total`);
    assertStyledStatsTile(await evalJs(cdp, statsTileStyleJs), where);
    assertCleanHydration(await controller(), where);
    if (name === "after-boot") await shot(cdp, "issues-truncated-stream.png");
    if (name === "inside-chunk-style") await shot(cdp, "issues-truncated-mid-style.png");
    console.log(`  PASS (${name}): stream-pending cleared on DOMContentLoaded; 1 RPC each, styled stats, 0 mismatches.`);
  }
}
