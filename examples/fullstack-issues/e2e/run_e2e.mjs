#!/usr/bin/env node
/**
 * Real Headless Chrome CDP E2E suite for `examples/fullstack-issues` streaming SSR + WASM.
 *
 * Zero npm dependencies: Node built-ins + local Chrome/Chromium `--headless=new`. The issues
 * server runs with `NVE_ISSUES_TEST_HOOKS=1` (deterministic `/_test/stream-control` barrier).
 * Every suite requires 0 console errors and 0 CSP violations.
 *
 *  1. Post-stream streaming SSR hydration (strict nonce CSP, 0 mismatches/replacements, 0 RPCs).
 *  2. True mid-stream async WASM boot behind a server barrier (+ interaction while the stream is
 *     open, reactive comments slot after release, still 0 Comments/Stats RPCs).
 *  3. Pre-headers `404` (`?issue=999`) + post-headers `<ErrorBoundary>` hydration
 *     (`?comments_issue=999`) with 0 RPCs.
 *  4. Full interactive app flows (FormDecode, 422 FieldErrors, CreateIssue + Nve-Invalidate incl.
 *     live stats, CDP-paused optimistic CloseIssue, filters, RenameIssue, AddComment) + screenshots.
 *  5. Live `FetchTransport` 6-probe browser matrix + live-DOM probes: native `<dialog>` /
 *     `[popover]` top layer (modality flips, moves, retags), a real nve-ui `Dialog` closed by a
 *     TRUSTED backdrop click (native `closedBy` and the fallback path), container-root hydration,
 *     and `<select>` SSR base-value parity.
 *  6. Real cross-origin CORS preflight.
 *  7. True 0-JS mode: DSD slots, ErrorBoundary slot, native `<form>` POST -> 303 -> GET /issues.
 */

import fs from "node:fs";
import path from "node:path";

import {
  ISSUES_PKG_DIR,
  REPO_ROOT,
  WEB_SHELL_DIR,
  capturePng,
  evalJs,
  findChromeBinary,
  launchHeadlessChrome,
  navigateAndWait,
  navigateDocument,
  openTab,
  parseArgs,
  runChecked,
  setInputValue,
  sleep,
  startIssuesServer,
  startProbeServer,
  waitFor,
  waitForEvent,
} from "./harness.mjs";

const SESSION_COOKIE = "nve_issues_session=web-demo-session";
const COMMENTS_RPC = "/rpc/issues.v1.Comments/List";
const STATS_RPC = "/rpc/issues.v1.Stats/Get";

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

/** JS predicate source: element exists and has a laid-out box. */
const visibleJs = (sel) =>
  `(() => { const el = document.querySelector(${JSON.stringify(sel)}); return Boolean(el && el.getBoundingClientRect().height > 0); })`;

function assertStrictCsp(headers, where) {
  const csp = headers["content-security-policy"] || "";
  const scriptSrc = csp.split(";").map((d) => d.trim()).find((d) => d.startsWith("script-src ")) || "";
  assert(/'nonce-[A-Za-z0-9+/=]{24}'/.test(csp), `${where}: CSP lacks style nonce: ${csp}`);
  assert(scriptSrc === "script-src 'self' 'wasm-unsafe-eval'", `${where}: weak script-src: ${scriptSrc}`);
  assert(!scriptSrc.includes("unsafe-inline"), `${where}: script-src allows unsafe-inline`);
}

async function main() {
  const { screenshotDir, skipBuild, releaseWasm } = parseArgs(process.argv.slice(2));
  if (!skipBuild) {
    runChecked("cargo", ["build", "--locked", "-p", "nve-example-issues-server"]);
    const wasmArgs = releaseWasm ? ["--release", "issues"] : ["--dev", "issues"];
    runChecked(path.join(WEB_SHELL_DIR, "build.sh"), wasmArgs, REPO_ROOT, { NVE_ISSUES_PROBES: "1" });
  }
  const serverBin = path.join(REPO_ROOT, "target/debug/nve-example-issues-server");
  assert(fs.existsSync(serverBin), `Missing server binary: ${serverBin}`);
  assert(
    fs.existsSync(path.join(ISSUES_PKG_DIR, "manifest.json")),
    `Missing WASM bundle: ${ISSUES_PKG_DIR}/manifest.json`
  );
  if (screenshotDir) fs.mkdirSync(screenshotDir, { recursive: true });
  const shot = async (cdp, name) => {
    if (screenshotDir) await capturePng(cdp, path.join(screenshotDir, name));
  };

  const probe = await startProbeServer(ISSUES_PKG_DIR);
  const probeOrigin = `http://127.0.0.1:${probe.port}`;
  const issuesSrv = await startIssuesServer(serverBin, ISSUES_PKG_DIR, probeOrigin);
  const origin = `http://127.0.0.1:${issuesSrv.port}`;
  const chrome = await launchHeadlessChrome(findChromeBinary());

  const consoleErrors = [];
  const cspErrors = [];
  const rpcRequests = [];
  const rpcSince = (mark, needle = "/rpc/") => rpcRequests.slice(mark).filter((u) => u.includes(needle)).length;

  const control = async (query) => {
    const res = await fetch(`${origin}/_test/stream-control?${query}`, { method: "POST" });
    assert(res.status === 200, `stream-control ${query} -> ${res.status}`);
  };
  const commentsWaiting = async () => {
    const text = await (await fetch(`${origin}/_test/stream-status`)).text();
    return Number(/comments_waiting=(\d+)/.exec(text)?.[1] ?? -1);
  };

  try {
    const cdp = await openTab(chrome.debugPort);
    cdp.on("Runtime.exceptionThrown", (p) => {
      const msg = p.exceptionDetails?.exception?.description || p.exceptionDetails?.text || JSON.stringify(p);
      console.error("  [browser exception]", msg);
      consoleErrors.push(msg);
    });
    cdp.on("Runtime.consoleAPICalled", (p) => {
      const line = p.args.map((a) => a.value ?? a.description ?? "").join(" ");
      if (p.type === "error" || p.type === "warning") console.error(`  [browser ${p.type}]`, line);
      if (p.type === "error") consoleErrors.push(line);
    });
    cdp.on("Log.entryAdded", ({ entry }) => {
      if (entry.source === "security" || /Content Security Policy/i.test(entry.text)) {
        console.error("  [CSP]", entry.text);
        cspErrors.push(entry.text);
      }
    });
    cdp.on("Network.responseReceived", (p) => {
      if (p.response.status >= 400) console.error(`  [http ${p.response.status}] ${p.response.url}`);
    });
    cdp.on("Network.requestWillBeSent", (p) => {
      if (p.request.url.includes("/rpc/")) rpcRequests.push(p.request.url);
    });
    await cdp.send("Network.setCookie", {
      name: "nve_issues_session",
      value: "web-demo-session",
      domain: "127.0.0.1",
      path: "/",
      httpOnly: true,
    });
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 840, deviceScaleFactor: 1, mobile: false });

    const controller = () =>
      evalJs(
        cdp,
        `(() => { const c = window.__nveIssuesController; return {
          hydrated: c.is_hydrated(), mismatches: c.hydration_mismatch_count(),
          replaced: c.hydration_replaced_count(), adopted: c.hydration_adopted_count() }; })()`
      );
    const waitMounted = (desc, timeoutMs = 20000) =>
      waitFor(cdp, desc, `() => Boolean(window.__nveIssuesController && window.__nveIssuesController.is_hydrated())`, timeoutMs);
    const assertCleanHydration = (h, where) => {
      assert(h.hydrated === true, `${where}: not hydrated ${JSON.stringify(h)}`);
      assert(h.mismatches === 0, `${where}: hydration mismatches ${JSON.stringify(h)}`);
      assert(h.replaced === 0, `${where}: hydration replaced nodes ${JSON.stringify(h)}`);
    };

    // -------------------------------------------------------------------------
    // 1. Post-stream streaming SSR hydration
    // -------------------------------------------------------------------------
    console.log("==> [1/7] Post-stream streaming SSR hydration (strict nonce CSP, 0 RPCs)");
    await control("stats_delay_ms=10&comments_delay_ms=25&hold_comments=0");
    let mark = rpcRequests.length;
    const doc1 = await navigateDocument(cdp, `${origin}/issues`);
    assert(doc1.status === 200, `GET /issues -> ${doc1.status}`);
    assertStrictCsp(doc1.headers, "suite 1");
    assert((await evalJs(cdp, "document.cookie")) === "", "HttpOnly cookie leaked into document.cookie");
    await waitMounted("WASM mounted after post-stream SSR");
    await waitFor(cdp, "DSD host removed post-stream", `() => document.querySelector('[data-nve-dsd-host]') === null`);
    const h1 = await controller();
    assertCleanHydration(h1, "suite 1");
    assert(h1.adopted > 20, `suite 1: expected > 20 adopted nodes, got ${h1.adopted}`);
    await waitFor(cdp, "stats panel visible", visibleJs('[data-testid="stats-panel"]'));
    await waitFor(cdp, "featured comments visible", visibleJs('[data-testid="featured-comments-panel"]'));
    await sleep(300);
    assert(rpcSince(mark) === 0, `suite 1: expected 0 /rpc/* requests, saw ${rpcSince(mark)}`);
    console.log(`  PASS: hydrated (adopted=${h1.adopted}, 0 mismatches/replaced), DSD host removed, 0 RPCs.`);

    // -------------------------------------------------------------------------
    // 2. True mid-stream WASM async boot behind the server barrier
    // -------------------------------------------------------------------------
    console.log("==> [2/7] Mid-stream async WASM boot behind held comments barrier");
    await control("stats_delay_ms=0&comments_delay_ms=0&hold_comments=1");
    mark = rpcRequests.length;
    const doc2 = await navigateDocument(cdp, `${origin}/issues`, { waitLoad: false });
    assert(doc2.status === 200, `held GET /issues -> ${doc2.status}`);
    assertStrictCsp(doc2.headers, "suite 2");
    await waitMounted("WASM mounted mid-stream");
    const waiting = await commentsWaiting();
    const readyState = await evalJs(cdp, "document.readyState");
    assert(waiting >= 1, `suite 2: expected comments_waiting >= 1, got ${waiting}`);
    assert(readyState === "loading", `suite 2: expected document still loading, got ${readyState}`);
    assertCleanHydration(await controller(), "suite 2");
    await waitFor(cdp, "stats panel rendered mid-stream", visibleJs('[data-testid="stats-panel"]'));
    assert(
      await evalJs(cdp, `${visibleJs('[data-testid="comments-skeleton"]')}()`),
      "suite 2: comments skeleton must stay visible while held"
    );
    assert(rpcSince(mark) === 0, `suite 2: expected 0 RPCs at mid-stream mount, saw ${rpcSince(mark)}`);
    await evalJs(cdp, `document.querySelector('[data-testid="filter-open"]').click()`);
    await waitFor(
      cdp,
      "Open filter mid-stream shows #2 and hides #3",
      `() => Boolean(document.querySelector('[data-testid="issue-row-2"]')) && !document.querySelector('[data-testid="issue-row-3"]')`
    );
    await evalJs(cdp, `document.querySelector('[data-testid="filter-all"]').click()`);
    await waitFor(cdp, "All filter restores #3", `() => Boolean(document.querySelector('[data-testid="issue-row-3"]'))`);
    assert((await commentsWaiting()) >= 1, "suite 2: stream closed before release (deadline?)");
    await control("hold_comments=0");
    await waitFor(cdp, "featured comments render reactively after release", visibleJs('[data-testid="featured-comments-panel"]'));
    await waitFor(cdp, "DSD host removed after stream end", `() => document.querySelector('[data-nve-dsd-host]') === null`);
    await sleep(300);
    assert(rpcSince(mark, COMMENTS_RPC) === 0, `suite 2: ${rpcSince(mark, COMMENTS_RPC)} Comments/List RPCs`);
    assert(rpcSince(mark, STATS_RPC) === 0, `suite 2: ${rpcSince(mark, STATS_RPC)} Stats/Get RPCs`);
    assertCleanHydration(await controller(), "suite 2 (post-release)");
    console.log("  PASS: mounted mid-stream, interacted while open, comments slot streamed in, 0 Comments/Stats RPCs.");

    // -------------------------------------------------------------------------
    // 3. Pre-headers 404 + post-headers ErrorBoundary hydration
    // -------------------------------------------------------------------------
    console.log("==> [3/7] Pre-headers 404 + post-headers ErrorBoundary hydration");
    await control("stats_delay_ms=10&comments_delay_ms=25&hold_comments=0");
    const missing = await fetch(`${origin}/issues?issue=999`, { headers: { cookie: SESSION_COOKIE } });
    assert(missing.status === 404, `?issue=999 -> ${missing.status}`);
    assert(!missing.headers.get("content-security-policy"), "404 must be decided before SSR headers");
    mark = rpcRequests.length;
    const doc3 = await navigateDocument(cdp, `${origin}/issues?comments_issue=999`);
    assert(doc3.status === 200, `?comments_issue=999 -> ${doc3.status}`);
    assertStrictCsp(doc3.headers, "suite 3");
    await waitMounted("WASM mounted on ErrorBoundary page");
    assertCleanHydration(await controller(), "suite 3");
    await waitFor(cdp, "comments ErrorBoundary visible", visibleJs('[data-testid="comments-error-boundary"]'));
    await sleep(300);
    assert(rpcSince(mark) === 0, `suite 3: expected 0 RPCs, saw ${rpcSince(mark)}`);
    console.log("  PASS: ?issue=999 -> 404 pre-headers; ?comments_issue=999 hydrates ErrorBoundary with 0 RPCs.");

    // -------------------------------------------------------------------------
    // 4. Full interactive app flows + screenshots
    // -------------------------------------------------------------------------
    console.log("==> [4/7] Interactive app flows + desktop/mobile screenshots");
    await control("stats_delay_ms=0&comments_delay_ms=0&hold_comments=0");
    await navigateDocument(cdp, `${origin}/issues`);
    await waitMounted("WASM mounted for interactive flows");
    await waitFor(
      cdp,
      "3 seeded issues hydrated",
      `() => ['1','2','3'].every((n) => document.querySelector('[data-testid="issue-row-' + n + '"]'))`
    );
    await sleep(150);
    await shot(cdp, "issues-desktop.png");

    const createCount = () => rpcRequests.filter((u) => u.includes("Issues/Create")).length;
    const statsTotal = `(() => document.querySelector('[data-testid="stats-total"]')?.lastElementChild?.textContent?.trim())`;
    assert((await evalJs(cdp, `${statsTotal}()`)) === "3", "stats-total should start at 3");

    // 4a. Local 0ms FormDecode validation -> 0 RPCs.
    let before = createCount();
    await evalJs(cdp, `document.querySelector('[data-testid="create-submit-btn"]').click()`);
    await waitFor(cdp, "local FormDecode error", `() => document.body.innerText.includes("title must not be empty")`);
    await sleep(120);
    assert(createCount() === before, `local validation fired ${createCount() - before} RPCs`);

    // 4b. Server 422 duplicate-title FieldErrors + draft preservation, exactly 1 RPC.
    const dupTitle = "Add H0 preloaded SSR + fail-closed hydration";
    before = createCount();
    await setInputValue(cdp, '[data-testid="create-title-input"]', dupTitle);
    await evalJs(cdp, `document.querySelector('[data-testid="create-submit-btn"]').click()`);
    await waitFor(cdp, "server 422 FieldErrors", `() => document.body.innerText.includes("an issue with this title already exists")`);
    await sleep(120);
    assert(createCount() - before === 1, `expected 1 CreateIssue RPC on 422, saw ${createCount() - before}`);
    const draft = await evalJs(
      cdp,
      `({ val: document.querySelector('[data-testid="create-title-input"]')?.value, dirty: document.querySelector('[data-testid="create-issue-form"]')?.getAttribute("data-nve-dirty") })`
    );
    assert(draft.val === dupTitle && draft.dirty === "true", `draft not preserved: ${JSON.stringify(draft)}`);

    // 4c. Valid CreateIssue -> Nve-Invalidate refetch of list AND live stats 3 -> 4.
    before = createCount();
    await setInputValue(cdp, '[data-testid="create-title-input"]', "Live Chrome WASM Issue");
    await evalJs(cdp, `document.querySelector('[data-testid="create-submit-btn"]').click()`);
    await waitFor(
      cdp,
      "created #4 via Nve-Invalidate refetch",
      `() => document.querySelector('[data-testid="issue-title-4"]')?.textContent?.includes("Live Chrome WASM Issue")`
    );
    await waitFor(cdp, "stats-total updates live 3 -> 4", `() => ${statsTotal}() === "4"`);
    assert(createCount() - before === 1, `expected 1 CreateIssue RPC, saw ${createCount() - before}`);
    const reset = await evalJs(
      cdp,
      `({ val: document.querySelector('[data-testid="create-title-input"]')?.value, dirty: document.querySelector('[data-testid="create-issue-form"]')?.getAttribute("data-nve-dirty") })`
    );
    assert(reset.val === "" && reset.dirty === "false", `form not reset: ${JSON.stringify(reset)}`);

    // 4d. CDP-paused optimistic CloseIssue + filter tabs.
    await cdp.send("Fetch.enable", { patterns: [{ urlPattern: "*Issues/Close*", requestStage: "Request" }] });
    let pausedId = null;
    const closePaused = waitForEvent(cdp, "Fetch.requestPaused", (p) => Boolean(p?.requestId), 10000, "Issues/Close paused");
    try {
      await evalJs(cdp, `document.querySelector('[data-testid="issue-close-4"]').click()`);
      const paused = await closePaused.promise;
      pausedId = paused.requestId;
      await waitFor(
        cdp,
        "optimistic Closed before server reply",
        `() => document.querySelector('[data-testid="issue-state-4"]')?.textContent?.includes("Closed")`
      );
      const finished = waitForEvent(
        cdp,
        "Network.loadingFinished",
        (p) => p?.requestId === paused.networkId,
        10000,
        "Issues/Close finished"
      );
      const reqId = pausedId;
      pausedId = null;
      await cdp.send("Fetch.continueRequest", { requestId: reqId });
      await finished.promise;
    } finally {
      closePaused.cancel();
      if (pausedId) await cdp.send("Fetch.continueRequest", { requestId: pausedId }).catch(() => {});
      await cdp.send("Fetch.disable").catch(() => {});
    }
    await waitFor(cdp, "confirmed Closed", `() => document.querySelector('[data-testid="issue-state-4"]')?.textContent?.includes("Closed")`);
    await evalJs(cdp, `document.querySelector('[data-testid="filter-closed"]').click()`);
    await waitFor(
      cdp,
      "Closed filter shows #4, hides #2",
      `() => Boolean(document.querySelector('[data-testid="issue-row-4"]')) && !document.querySelector('[data-testid="issue-row-2"]')`
    );
    await evalJs(cdp, `document.querySelector('[data-testid="filter-all"]').click()`);
    await waitFor(cdp, "All filter restores #2", `() => Boolean(document.querySelector('[data-testid="issue-row-2"]'))`);

    // 4e. RenameIssue (FormHiddenField) + AddComment in the detail panel.
    await evalJs(cdp, `document.querySelector('[data-testid="issue-select-2"]').click()`);
    await waitFor(cdp, "detail panel for #2", `() => Boolean(document.querySelector('[data-testid="rename-title-input"]'))`);
    await setInputValue(cdp, '[data-testid="rename-title-input"]', "Renamed Live in Chrome");
    await evalJs(cdp, `document.querySelector('[data-testid="rename-submit-btn"]').click()`);
    await waitFor(
      cdp,
      "#2 renamed",
      `() => document.querySelector('[data-testid="issue-title-2"]')?.textContent?.includes("Renamed Live in Chrome")`
    );
    await setInputValue(cdp, '[data-testid="comment-body-input"]', "Browser E2E comment");
    await evalJs(cdp, `document.querySelector('[data-testid="comment-submit-btn"]').click()`);
    await waitFor(cdp, "comment in detail panel", `() => document.body.innerText.includes("Browser E2E comment")`);
    await sleep(150);
    await shot(cdp, "issues-desktop-detail.png");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
    await sleep(200);
    await shot(cdp, "issues-mobile.png");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 840, deviceScaleFactor: 1, mobile: false });
    console.log("  PASS: FormDecode, 422 + draft, CreateIssue + live stats 3->4, optimistic Close, filters, Rename, AddComment.");

    // -------------------------------------------------------------------------
    // 5. Live FetchTransport browser matrix (from the probe origin's own page: the issues page's
    //    strict `connect-src 'self'` CSP rightly blocks fetches to the probe server)
    // -------------------------------------------------------------------------
    console.log("==> [5/7] Live FetchTransport edge-case matrix in Chrome");
    await navigateAndWait(cdp, `${probeOrigin}/cors-runner`);
    await evalJs(
      cdp,
      `(async () => {
        const manifest = await (await fetch("/pkg/issues/manifest.json")).json();
        const mod = await import("/pkg/issues/" + manifest.js);
        await mod.default({ module_or_path: "/pkg/issues/" + manifest.wasm });
        window.__nveProbeMod = mod;
      })()`
    );
    const probeReport = await evalJs(cdp, `window.__nveProbeMod.nve_probe_fetch_transport(${JSON.stringify(probeOrigin)})`);
    console.log("  FetchTransport probe result:", probeReport);
    assert(String(probeReport).startsWith("OK:"), `FetchTransport probe failed: ${probeReport}`);
    assert(
      probe.timeoutAbortedCount.value >= 1 && probe.dropAbortedCount.value >= 1,
      `expected timeout + drop aborts (timeout=${probe.timeoutAbortedCount.value}, drop=${probe.dropAbortedCount.value})`
    );
    console.log("  PASS: all FetchTransport browser probes.");

    // Live-DOM probes on the same bundle: native top-layer overlays and container-as-root mounts.
    const probeCall = async (probeFn) => {
      const report = await evalJs(cdp, `window.__nveProbeMod.${probeFn}()`);
      console.log(`  ${probeFn}:`, report);
      assert(String(report).startsWith("OK:"), `${probeFn} failed: ${report}`);
    };
    for (const probeFn of [
      "nve_probe_native_overlays",
      "nve_probe_container_root_hydration",
      "nve_probe_select_hydration",
    ]) {
      await probeCall(probeFn);
    }
    // Real nve-ui Dialog: a TRUSTED click on the ::backdrop (viewport corner, outside the centered
    // dialog box) must close it and reach app state through the bridged native `close` event.
    const trustedClick = async (x, y) => {
      for (const type of ["mousePressed", "mouseReleased"]) {
        await cdp.send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
      }
      // `close` is dispatched from a queued task: let it run before pumping.
      await evalJs(cdp, "new Promise((r) => setTimeout(r, 50))");
    };
    await probeCall("nve_probe_dialog_mount");
    await trustedClick(4, 4);
    await probeCall("nve_probe_dialog_after_backdrop_click");
    const hadClosedBy = await evalJs(
      cdp,
      "(() => { const had = 'closedBy' in HTMLDialogElement.prototype; delete HTMLDialogElement.prototype.closedBy; return had; })()"
    );
    console.log(`  native closedBy before delete: ${hadClosedBy}; fallback round:`);
    await trustedClick(4, 4);
    await probeCall("nve_probe_dialog_after_backdrop_click");
    await evalJs(cdp, "window.__nveProbeMod.nve_probe_dialog_unmount()");
    console.log("  PASS: top layer (flips/moves/retags), trusted backdrop Dialog close (native + fallback), container-root + <select> hydration.");

    // -------------------------------------------------------------------------
    // 6. Real cross-origin CORS preflight
    // -------------------------------------------------------------------------
    console.log("==> [6/7] Real cross-origin CORS preflight");
    const corsResult = await evalJs(cdp, `window.__nveProbeMod.nve_probe_cross_origin_cors(${JSON.stringify(origin)})`);
    console.log("  Cross-origin CORS probe result:", corsResult);
    assert(String(corsResult).startsWith("OK:"), `CORS probe failed: ${corsResult}`);
    console.log("  PASS: CORS preflight + CBOR RPC + exposed Nve-Invalidate.");

    // -------------------------------------------------------------------------
    // 7. True 0-JS browser mode
    // -------------------------------------------------------------------------
    console.log("==> [7/7] True 0-JS mode (DSD slots, ErrorBoundary slot, native form PRG)");
    await control("stats_delay_ms=10&comments_delay_ms=60&hold_comments=0");
    await cdp.send("Emulation.setScriptExecutionDisabled", { value: true });
    // Rendered height of an element, descending through `display: contents` slot wrappers.
    const boxHeightJs = `const boxHeight = (el) => !el ? 0 : getComputedStyle(el).display === "contents"
      ? Math.max(0, ...Array.from(el.children, boxHeight)) : el.getBoundingClientRect().height;`;
    const slotState = (slotName) => `(() => { ${boxHeightJs}
      const host = document.querySelector('[data-nve-dsd-host]');
      const slot = host?.shadowRoot?.querySelector('slot[name="${slotName}"]');
      const assigned = slot ? slot.assignedElements() : [];
      return { count: assigned.length, height: boxHeight(assigned[0]) };
    })()`;
    const doc7 = await navigateDocument(cdp, `${origin}/issues`);
    assert(doc7.status === 200, `0-JS GET /issues -> ${doc7.status}`);
    const zeroJs = await evalJs(
      cdp,
      `({ live: document.getElementById("app")?.hasAttribute("data-nve-live"),
          shadow: document.querySelector('[data-nve-dsd-host]')?.shadowRoot?.mode ?? null,
          mounted: Boolean(window.__nveIssuesController) })`
    );
    assert(!zeroJs.live && !zeroJs.mounted, `0-JS: WASM must not mount: ${JSON.stringify(zeroJs)}`);
    assert(zeroJs.shadow === "open", `0-JS: expected open DSD shadowRoot, got ${zeroJs.shadow}`);
    for (const name of ["nve-slot-stats", "nve-slot-comments"]) {
      const s = await evalJs(cdp, slotState(name));
      assert(s.count === 1 && s.height > 0, `0-JS: ${name} not slotted/visible: ${JSON.stringify(s)}`);
    }
    await shot(cdp, "issues-0js-desktop.png");

    await navigateDocument(cdp, `${origin}/issues?comments_issue=999`);
    const boundary = await evalJs(
      cdp,
      `(() => { ${boxHeightJs}
        const el = document.querySelector('[data-nve-dsd-host] > [slot="nve-slot-comments"] [data-testid="comments-error-boundary"]');
        return { slotted: Boolean(el?.closest('[slot]')?.assignedSlot), height: boxHeight(el) }; })()`
    );
    assert(boundary.slotted && boundary.height > 0, `0-JS ErrorBoundary not slotted/visible: ${JSON.stringify(boundary)}`);

    await navigateDocument(cdp, `${origin}/issues`);
    const shadowText = `(() => { const h = document.querySelector('[data-nve-dsd-host]'); return (h?.shadowRoot?.textContent ?? "") + (h?.textContent ?? ""); })`;
    assert(
      await evalJs(cdp, `${shadowText}().includes("Live Chrome WASM Issue")`),
      "0-JS page missing the issue created in suite 4"
    );
    const prgDone = waitForEvent(cdp, "Page.loadEventFired", () => true, 15000, "0-JS form PRG load");
    try {
      await evalJs(
        cdp,
        `(() => {
          const form = document.querySelector('[data-nve-dsd-host]').shadowRoot
            .querySelector('form[action="/issues/create"][method="post"]');
          form.querySelector('input[name="title"]').value = "Created via 0-JS HTML Form";
          form.requestSubmit();
        })()`
      );
      await prgDone.promise;
    } catch (err) {
      prgDone.cancel();
      throw err;
    }
    const after = await evalJs(
      cdp,
      `({ path: location.pathname, created: ${shadowText}().includes("Created via 0-JS HTML Form"),
          live: document.getElementById("app")?.hasAttribute("data-nve-live") })`
    );
    assert(after.path === "/issues" && after.created && !after.live, `0-JS PRG failed: ${JSON.stringify(after)}`);
    await cdp.send("Emulation.setScriptExecutionDisabled", { value: false });
    console.log("  PASS: DSD slots + ErrorBoundary slotted with 0 JS; native form 303 -> GET /issues shows created issue.");

    assert(consoleErrors.length === 0, `Unexpected browser console/page errors:\n${consoleErrors.join("\n")}`);
    assert(cspErrors.length === 0, `Unexpected CSP violations:\n${cspErrors.join("\n")}`);
    cdp.close();
    console.log("\nALL 7 LIVE CHROME FULLSTACK E2E SUITES PASSED (0 console errors, 0 CSP violations)!");
  } finally {
    try { chrome.proc.kill("SIGKILL"); } catch {}
    try { fs.rmSync(chrome.userDataDir, { recursive: true, force: true }); } catch {}
    try { issuesSrv.proc.kill("SIGTERM"); } catch {}
    probe.server.close();
  }
}

main().catch((err) => {
  console.error("\nFULLSTACK E2E FAILED:", err);
  process.exit(1);
});
