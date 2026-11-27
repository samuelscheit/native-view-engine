#!/usr/bin/env node
/**
 * Self-contained headless Chrome CDP E2E & visual QA harness for `examples/nve-docs-site`.
 *
 * Zero external npm dependencies: uses Node built-ins (`http`, `fs`, `path`, `os`, `child_process`,
 * global `WebSocket`, global `fetch`) + local Chrome/Chromium `--headless=new`.
 *
 * Usage:
 *   node examples/nve-docs-site/e2e/run_docs_e2e.mjs [distDir] [--shots-dir /tmp/nve-docs-shots]
 */

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawn, spawnSync } from "node:child_process";

const MIME_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".mjs": "application/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".xml": "application/xml; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".nvs2": "application/octet-stream",
};

function parseArgs(argv) {
  let distDir = "/tmp/nve-site-dist";
  let shotsDir = "/tmp/nve-landing-shots";
  let basePath = "";
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--shots-dir") {
      shotsDir = argv[++i];
    } else if (arg === "--base-path") {
      basePath = argv[++i] || "";
    } else if (!arg.startsWith("-")) {
      distDir = arg;
    } else {
      throw new Error(`Unknown flag: ${arg}`);
    }
  }
  basePath = basePath.trim().replace(/^\/+|\/+$/g, "");
  basePath = basePath ? `/${basePath}` : "";
  return { distDir: path.resolve(distDir), shotsDir: path.resolve(shotsDir), basePath };
}

function findChromeBinary() {
  const candidates = [
    process.env.NVE_CHROME_BIN,
    process.env.CHROME_BIN,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ].filter(Boolean);

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate;
  }
  for (const cmd of ["google-chrome", "chromium", "chromium-browser"]) {
    const res = spawnSync("which", [cmd], { encoding: "utf8" });
    if (res.status === 0 && res.stdout.trim()) return res.stdout.trim();
  }
  throw new Error("Chrome/Chromium binary not found. Set NVE_CHROME_BIN or install Chrome.");
}

function startStaticServer(rootDir, basePath) {
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      const reqUrl = new URL(req.url || "/", "http://127.0.0.1");
      let reqPath = decodeURIComponent(reqUrl.pathname);
      if (basePath && reqPath.startsWith(basePath)) {
        reqPath = reqPath.slice(basePath.length) || "/";
      }
      const cleanRel = reqPath.replace(/^\/+/, "");
      const directPath = path.resolve(rootDir, cleanRel);
      if (!directPath.startsWith(rootDir)) {
        res.writeHead(403);
        res.end("Forbidden");
        return;
      }

      let chosen = null;
      let status = 200;
      if (fs.existsSync(directPath) && fs.statSync(directPath).isFile()) {
        chosen = directPath;
      } else {
        const indexCandidate = path.join(directPath, "index.html");
        if (fs.existsSync(indexCandidate) && fs.statSync(indexCandidate).isFile()) {
          chosen = indexCandidate;
        } else {
          const fallback404 = path.join(rootDir, "404.html");
          if (fs.existsSync(fallback404)) {
            chosen = fallback404;
            status = 404;
          }
        }
      }

      if (!chosen) {
        res.writeHead(404);
        res.end("Not Found");
        return;
      }
      const ext = path.extname(chosen).toLowerCase();
      res.writeHead(status, {
        "Content-Type": MIME_TYPES[ext] || "application/octet-stream",
        "Cache-Control": "no-store",
      });
      fs.createReadStream(chosen).pipe(res);
    });
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function launchHeadlessChrome(chromeBin) {
  return new Promise((resolve, reject) => {
    const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "nve-docs-e2e-chrome-"));
    const args = [
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${userDataDir}`,
      "--no-sandbox",
      "--disable-setuid-sandbox",
      "--disable-gpu",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-background-networking",
      "--disable-extensions",
      "--mute-audio",
      "--window-size=1440,900",
      "about:blank",
    ];
    const proc = spawn(chromeBin, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stderrBuf = "";
    let settled = false;
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        try { proc.kill("SIGKILL"); } catch {}
        reject(new Error(`Timed out waiting for Chrome DevTools.\nstderr:\n${stderrBuf}`));
      }
    }, 15000);

    proc.stderr.on("data", (chunk) => {
      stderrBuf += chunk.toString("utf8");
      const m = stderrBuf.match(/DevTools listening on (ws:\/\/127\.0\.0\.1:(\d+)\/devtools\/browser\/[^\s]+)/);
      if (m && !settled) {
        settled = true;
        clearTimeout(timer);
        resolve({ proc, userDataDir, debugPort: Number(m[2]) });
      }
    });
    proc.on("exit", (code, sig) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        reject(new Error(`Chrome exited early (code=${code}, sig=${sig}).\nstderr:\n${stderrBuf}`));
      }
    });
  });
}

class CdpSession {
  constructor(wsUrl) {
    this.wsUrl = wsUrl;
    this.nextId = 1;
    this.pending = new Map();
    this.consoleErrors = [];
    this.pageErrors = [];
    this.ws = null;
  }

  async connect() {
    await new Promise((resolve, reject) => {
      const ws = new WebSocket(this.wsUrl);
      this.ws = ws;
      ws.addEventListener("open", () => resolve());
      ws.addEventListener("error", (e) => reject(e));
      ws.addEventListener("message", (ev) => {
        const msg = JSON.parse(typeof ev.data === "string" ? ev.data : ev.data.toString());
        if (msg.id && this.pending.has(msg.id)) {
          const { resolve: res, reject: rej } = this.pending.get(msg.id);
          this.pending.delete(msg.id);
          if (msg.error) rej(new Error(`CDP ${msg.error.code}: ${msg.error.message}`));
          else res(msg.result);
        } else if (msg.method === "Runtime.consoleAPICalled") {
          if (msg.params?.type === "error") {
            const txt = (msg.params.args || []).map((a) => a.value ?? a.description ?? "").join(" ");
            this.consoleErrors.push(txt);
          }
        } else if (msg.method === "Runtime.exceptionThrown") {
          const d = msg.params?.exceptionDetails;
          this.pageErrors.push(d?.exception?.description || d?.text || "uncaught exception");
        }
      });
    });
    await this.send("Page.enable");
    await this.send("Runtime.enable");
    await this.send("DOM.enable");
    await this.send("Network.enable");
  }

  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  async eval(expression) {
    const res = await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (res.exceptionDetails) {
      throw new Error(
        `Page eval exception: ${res.exceptionDetails.exception?.description || res.exceptionDetails.text}`
      );
    }
    return res.result?.value;
  }

  async setViewport(width, height, mobile = false, scale = 1) {
    await this.send("Emulation.setDeviceMetricsOverride", {
      width,
      height,
      deviceScaleFactor: scale,
      mobile,
    });
  }

  async navigate(url) {
    const loaded = new Promise((resolve) => {
      const onMsg = (ev) => {
        const msg = JSON.parse(typeof ev.data === "string" ? ev.data : ev.data.toString());
        if (msg.method === "Page.loadEventFired") {
          this.ws.removeEventListener("message", onMsg);
          resolve();
        }
      };
      this.ws.addEventListener("message", onMsg);
    });
    await this.send("Page.navigate", { url });
    await Promise.race([
      loaded,
      new Promise((_, rej) => setTimeout(() => rej(new Error(`Timeout navigating to ${url}`)), 15000)),
    ]);
  }

  async waitFor(fnExpr, timeoutMs = 10000, label = "condition") {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
      const ok = await this.eval(fnExpr);
      if (ok) return ok;
      await new Promise((r) => setTimeout(r, 50));
    }
    throw new Error(`Timed out after ${timeoutMs}ms waiting for ${label}`);
  }

  async capturePng(outPath) {
    const { data } = await this.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(outPath, Buffer.from(data, "base64"));
  }

  close() {
    try { this.ws?.close(); } catch {}
  }
}

function assert(cond, msg) {
  if (!cond) throw new Error(`ASSERTION FAILED: ${msg}`);
}

async function createPageCdp(debugPort) {
  const res = await fetch(`http://127.0.0.1:${debugPort}/json/new?about:blank`, { method: "PUT" });
  const target = await res.json();
  const cdp = new CdpSession(target.webSocketDebuggerUrl);
  await cdp.connect();
  return { cdp, targetId: target.id };
}

async function run() {
  const { distDir, shotsDir, basePath } = parseArgs(process.argv.slice(2));
  assert(fs.existsSync(path.join(distDir, "index.html")), `Missing ${distDir}/index.html`);
  fs.mkdirSync(shotsDir, { recursive: true });

  const chromeBin = findChromeBinary();
  const { server, port } = await startStaticServer(distDir, basePath);
  const origin = `http://127.0.0.1:${port}${basePath}`;
  const chrome = await launchHeadlessChrome(chromeBin);

  try {
    // -----------------------------------------------------------------------
    // 1. JS-DISABLED STATIC SSR VERIFICATION (/, /examples/, /docs/, /docs/guides/ui-components/)
    // -----------------------------------------------------------------------
    console.log("==> [1/5] Verifying JS-disabled static SSR on /, /examples/, /docs/, and /docs/guides/ui-components/...");
    const { cdp: ssrCdp } = await createPageCdp(chrome.debugPort);
    await ssrCdp.setViewport(1440, 900, false, 1);
    await ssrCdp.send("Emulation.setScriptExecutionDisabled", { value: true });

    // 1a. Landing Page (/) & Examples Gallery (/examples/)
    for (const [route, expectedH1] of [
      ["/", "Native-speed UI in Rust"],
      ["/examples/", "Interactive Examples"],
    ]) {
      const resp = await fetch(`${origin}${route}`);
      assert(resp.status === 200, `Expected HTTP 200 for ${route}, got ${resp.status}`);
      const rawHtml = await resp.text();
      assert(!rawHtml.includes("nve_web_shell.js"), `SSR ${route} must NOT preload Studio WASM`);
      await ssrCdp.navigate(`${origin}${route}`);
      const info = await ssrCdp.eval(`(() => {
        const main = document.querySelector("main");
        const h1 = document.querySelector("h1");
        const root = document.querySelector('[data-testid="docs-root"]');
        const bg = root ? getComputedStyle(root).backgroundColor : getComputedStyle(document.body).backgroundColor;
        const sidebar = document.querySelector('[data-testid="docs-sidebar-desktop"]');
        return {
          hasMain: !!main,
          h1Text: h1 ? h1.textContent.trim() : "",
          proseLen: document.body.innerText.trim().length,
          bg,
          hasSidebar: !!sidebar,
          scrollOverflowX: document.documentElement.scrollWidth - window.innerWidth,
          hydrated: !!window.__NVE_DOCS__,
        };
      })()`);
      assert(info.hasMain, `SSR ${route}: missing <main>`);
      assert(info.h1Text.includes(expectedH1), `SSR ${route}: expected H1 containing "${expectedH1}", got "${info.h1Text}"`);
      assert(info.proseLen > 300, `SSR ${route}: body prose too short (${info.proseLen})`);
      assert(info.bg === "rgb(9, 9, 11)", `SSR ${route}: expected #09090b bg, got ${info.bg}`);
      assert(!info.hasSidebar, `SSR ${route}: full-bleed page should not render docs sidebar`);
      assert(info.scrollOverflowX <= 2, `SSR ${route}: horizontal overflow (${info.scrollOverflowX}px)`);
      assert(!info.hydrated, `SSR ${route}: JS should be disabled`);
      console.log(`    ✓ SSR (JS off) ${route}: h1="${info.h1Text}", bg=${info.bg}, overflowX=${info.scrollOverflowX}px`);
    }

    // 1b. Docs Suite (/docs/ & /docs/guides/ui-components/)
    for (const route of ["/docs/", "/docs/guides/ui-components/"]) {
      const resp = await fetch(`${origin}${route}`);
      assert(resp.status === 200, `Expected HTTP 200 for ${route}, got ${resp.status}`);
      await ssrCdp.navigate(`${origin}${route}`);

      const info = await ssrCdp.eval(`(() => {
        const nav = document.querySelector("nav");
        const main = document.querySelector("main");
        const article = document.querySelector("article");
        const h1 = document.querySelector("h1");
        const code = document.querySelector("pre, code");
        const root = document.querySelector('[data-testid="docs-root"]');
        const bg = root ? getComputedStyle(root).backgroundColor : getComputedStyle(document.body).backgroundColor;
        const sidebar = document.querySelector('[data-testid="docs-sidebar-desktop"]');
        const toc = document.querySelector('[data-testid="docs-toc"]');
        return {
          hasNav: !!nav,
          hasMain: !!main,
          hasArticle: !!article,
          h1Text: h1 ? h1.textContent.trim() : "",
          hasCode: !!code,
          proseLen: document.body.innerText.trim().length,
          bg,
          sidebarW: sidebar ? sidebar.getBoundingClientRect().width : 0,
          mainW: main ? main.getBoundingClientRect().width : 0,
          tocW: toc ? toc.getBoundingClientRect().width : 0,
          hydrated: !!window.__NVE_DOCS__,
        };
      })()`);

      assert(info.hasNav, `SSR ${route}: missing <nav>`);
      assert(info.hasMain, `SSR ${route}: missing <main>`);
      assert(info.hasArticle, `SSR ${route}: missing <article>`);
      assert(info.h1Text.length > 0, `SSR ${route}: empty <h1>`);
      assert(info.hasCode, `SSR ${route}: missing code element`);
      assert(info.proseLen > 300, `SSR ${route}: prose too short (${info.proseLen})`);
      assert(info.bg === "rgb(9, 9, 11)", `SSR ${route}: expected #09090b bg, got ${info.bg}`);
      assert(info.sidebarW >= 240, `SSR ${route}: sidebar collapsed (${info.sidebarW}px)`);
      assert(info.mainW >= 500, `SSR ${route}: main column too narrow (${info.mainW}px)`);
      assert(info.tocW >= 200, `SSR ${route}: right TOC missing (${info.tocW}px)`);
      assert(!info.hydrated, `SSR ${route}: JS should be disabled`);
      console.log(
        `    ✓ SSR (JS off) ${route}: h1="${info.h1Text}", bg=${info.bg}, cols=${Math.round(info.sidebarW)}/${Math.round(info.mainW)}/${Math.round(info.tocW)}px`
      );
    }
    ssrCdp.close();

    // -----------------------------------------------------------------------
    // 2. JS-ENABLED HYDRATION ON LANDING (/), HERO TABS, THEME, ⌘K, DOCS & EXAMPLES GALLERY
    // -----------------------------------------------------------------------
    console.log("==> [2/5] Verifying Landing hydration, hero interactive tabs, theme toggle, ⌘K search, /docs/** & /examples/...");
    const { cdp } = await createPageCdp(chrome.debugPort);
    await cdp.setViewport(1440, 900, false, 1);

    const requestedUrls = [];
    cdp.ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(typeof ev.data === "string" ? ev.data : ev.data.toString());
      if (msg.method === "Network.requestWillBeSent" && msg.params?.request?.url) {
        requestedUrls.push(msg.params.request.url);
      }
    });

    await cdp.send("Page.addScriptToEvaluateOnNewDocument", {
      source: `
        window.__SSR_CAPTURED_ROOT__ = null;
        window.__SSR_CAPTURED_H1__ = null;
        document.addEventListener("DOMContentLoaded", () => {
          window.__SSR_CAPTURED_ROOT__ = document.querySelector('#app [data-nve-id="1"]');
          window.__SSR_CAPTURED_H1__ = document.querySelector('#app h1');
        });
      `,
    });

    await cdp.navigate(`${origin}/`);
    await cdp.waitFor(`!!window.__NVE_DOCS__ || !!window.__NVE_DOCS_ERROR__`, 12000, "Landing WASM hydration");

    const hydrateInfo = await cdp.eval(`(() => {
      if (window.__NVE_DOCS_ERROR__) return { error: window.__NVE_DOCS_ERROR__ };
      const c = window.__NVE_DOCS__;
      const rootAfter = document.querySelector('#app [data-nve-id="1"]');
      const h1After = document.querySelector('#app h1');
      return {
        hydrated: c.hydrated,
        adopted: c.adopted_nodes,
        mismatches: c.mismatches,
        replaced: c.replaced_nodes,
        seededRules: c.seeded_rules,
        route: c.current_route,
        darkMode: c.dark_mode,
        rootIdentityPreserved: window.__SSR_CAPTURED_ROOT__ !== null && window.__SSR_CAPTURED_ROOT__ === rootAfter,
        h1IdentityPreserved: window.__SSR_CAPTURED_H1__ !== null && window.__SSR_CAPTURED_H1__ === h1After,
      };
    })()`);

    assert(!hydrateInfo.error, `WASM hydration error: ${hydrateInfo.error}`);
    assert(hydrateInfo.hydrated === true, "expected hydrated=true");
    assert(hydrateInfo.adopted > 50, `expected adopted > 50, got ${hydrateInfo.adopted}`);
    assert(hydrateInfo.mismatches === 0, `expected 0 hydration mismatches on /, got ${hydrateInfo.mismatches}`);
    assert(hydrateInfo.replaced === 0, `expected 0 replaced nodes on /, got ${hydrateInfo.replaced}`);
    assert(hydrateInfo.rootIdentityPreserved, "Landing SSR root DOM identity was NOT preserved!");
    assert(hydrateInfo.h1IdentityPreserved, "Landing SSR <h1> DOM identity was NOT preserved!");
    assert(
      !requestedUrls.some((u) => u.includes("nve_web_shell")),
      `Landing page must NEVER download Studio WASM bundle! Requests: ${requestedUrls.join(", ")}`
    );
    console.log(
      `    ✓ Landing hydrated in-place: adopted=${hydrateInfo.adopted}, mismatches=0, domIdentityPreserved=true, studioWasmLazy=true`
    );

    // Screenshot 1: Landing Desktop Dark (1440x900)
    await cdp.capturePng(path.join(shotsDir, "landing-desktop-dark.png"));

    // Interactive Hero Preview Card checks (Live UI increment -> RSX Code -> Binary IR -> Live UI)
    await cdp.eval(`document.querySelector('[data-testid="hero-live-inc"]').click()`);
    await cdp.waitFor(
      `document.querySelector('[data-testid="hero-live-count"]')?.textContent?.includes("Count: 4")`,
      4000,
      "hero live counter increment"
    );
    await cdp.eval(`document.querySelector('[data-testid="hero-tab-code"]').click()`);
    await cdp.waitFor(`!!document.querySelector('[data-testid="hero-panel-code"]')`, 4000, "hero RSX Code tab");
    await cdp.eval(`document.querySelector('[data-testid="hero-tab-ir"]').click()`);
    await cdp.waitFor(
      `document.querySelector('[data-testid="hero-panel-ir"]')?.textContent?.includes("Count: 4")`,
      4000,
      "hero Binary IR live sync"
    );
    await cdp.eval(`document.querySelector('[data-testid="hero-tab-live"]').click()`);
    await cdp.waitFor(`!!document.querySelector('[data-testid="hero-panel-live"]')`, 4000, "hero Live UI restore");
    console.log("    ✓ Hero Preview Card tabs (Live UI -> RSX Code -> Binary IR) verified");

    // Theme toggle Dark -> Light -> Screenshot 2 -> Dark
    await cdp.eval(`document.querySelector('[data-testid="docs-theme-toggle"]').click()`);
    await cdp.waitFor(`window.__NVE_DOCS__.dark_mode === false`, 4000, "light theme switch");
    const lightBg = await cdp.eval(`getComputedStyle(document.querySelector('[data-testid="docs-root"]')).backgroundColor`);
    assert(lightBg === "rgb(255, 255, 255)", `Expected light canvas rgb(255, 255, 255), got ${lightBg}`);
    await cdp.capturePng(path.join(shotsDir, "landing-desktop-light.png"));
    console.log(`    ✓ Landing light mode verified (landing-desktop-light.png captured)`);

    await cdp.eval(`document.querySelector('[data-testid="docs-theme-toggle"]').click()`);
    await cdp.waitFor(`window.__NVE_DOCS__.dark_mode === true`, 4000, "dark theme restore");

    // Development & Benchmark Progression Dashboard: scroll to controls + charts, capture screenshot, test Log scale + category pills + Table view
    await cdp.eval(`document.querySelector('[data-testid="bench-controls-bar"]').scrollIntoView({ block: "start" })`);
    await new Promise((r) => setTimeout(r, 200));
    await cdp.waitFor(`!!document.querySelector('[data-testid="bench-latest-canvas"]')`, 4000, "benchmark NVE_DRAW1 canvas");
    await cdp.capturePng(path.join(shotsDir, "landing-benchmarks.png"));

    const yaxisBefore = await cdp.eval(`document.querySelector('[data-testid="bench-bar-yaxis"]')?.textContent || ""`);
    await cdp.eval(`document.querySelector('[data-testid="bench-log-scale"]').click()`);
    await cdp.waitFor(
      `(document.querySelector('[data-testid="bench-bar-yaxis"]')?.textContent || "") !== ${JSON.stringify(yaxisBefore)}`,
      4000,
      "benchmark Log scale toggle"
    );
    await cdp.eval(`document.querySelector('[data-testid="bench-filter-web-wasm"]').click()`);
    await cdp.waitFor(
      `!!document.querySelector('[data-testid="bench-card-web-ssr-payload"]') && !document.querySelector('[data-testid="bench-card-cmdbuf-encode"]')`,
      4000,
      "benchmark Web & WASM category filter"
    );
    await cdp.eval(`document.querySelector('[data-testid="bench-filter-all"]').click()`);
    await cdp.waitFor(`!!document.querySelector('[data-testid="bench-card-cmdbuf-encode"]')`, 4000, "benchmark All filter restore");
    await cdp.eval(`document.querySelector('[data-testid="bench-table-toggle"]').click()`);
    await cdp.waitFor(
      `!!document.querySelector('[data-testid="bench-latest-table"]') && !document.querySelector('[data-testid="bench-latest-canvas"]')`,
      4000,
      "benchmark Table view toggle"
    );
    await cdp.eval(`document.querySelector('[data-testid="bench-table-toggle"]').click()`);
    await cdp.waitFor(`!!document.querySelector('[data-testid="bench-latest-canvas"]')`, 4000, "benchmark Chart view restore");
    console.log("    ✓ Development & Benchmark Progression dashboard (Log scale, category pills, Table view, landing-benchmarks.png) verified");

    // SPA navigation from Landing -> Examples Gallery (/examples)
    await cdp.eval(`window.__SPA_ALIVE__ = "yes";`);
    await cdp.eval(`document.querySelector('[data-testid="docs-section-examples"]').click()`);
    await cdp.waitFor(`location.pathname.endsWith("/examples")`, 5000, "SPA pushState to /examples");
    assert((await cdp.eval(`window.__SPA_ALIVE__`)) === "yes", "Examples nav reloaded the page!");
    await cdp.waitFor(`!!document.querySelector('[data-testid="examples-gallery-page"]')`, 4000, "examples gallery mount");
    await cdp.capturePng(path.join(shotsDir, "examples-gallery.png"));
    console.log("    ✓ SPA navigation to /examples verified (examples-gallery.png captured)");

    // Switch live embedded playground on /examples
    await cdp.eval(`document.querySelector('[data-testid="examples-pg-tab-crdt-simulator"]').click()`);
    await cdp.waitFor(`!!document.querySelector('[data-testid="pg-crdt-convergence"]')`, 4000, "gallery CRDT tab");

    // SPA navigation from Examples -> Docs (/docs) -> /docs/guides/ui-components
    await cdp.eval(`document.querySelector('[data-testid="docs-section-docs"]').click()`);
    await cdp.waitFor(`location.pathname.endsWith("/docs")`, 5000, "SPA pushState to /docs");
    assert((await cdp.eval(`window.__SPA_ALIVE__`)) === "yes", "Docs nav reloaded the page!");
    await cdp.eval(`document.querySelector('[data-testid="docs-nav-guides-ui-components"]').click()`);
    await cdp.waitFor(
      `location.pathname.endsWith("/docs/guides/ui-components") && !!document.querySelector('[data-testid="pg-ui-root"]')`,
      5000,
      "SPA sidebar nav to /docs/guides/ui-components"
    );
    await cdp.eval(`document.querySelector('[data-testid="pg-ui-btn-primary"]').click()`);
    await cdp.waitFor(
      `document.querySelector('[data-testid="pg-ui-count-badge"]')?.textContent?.includes("Actions: 1")`,
      4000,
      "UI playground button click"
    );
    await cdp.eval(`document.querySelector('[data-testid="pg-vlist-jump-75000"]').click()`);
    await cdp.waitFor(
      `document.querySelector('[data-testid="pg-vlist-selected"]')?.textContent?.includes("#75000")`,
      4000,
      "VirtualList jump to #75000"
    );
    console.log("    ✓ Docs /docs/guides/ui-components + UI Playground + 100k VirtualList verified");

    // ⌘K Search Modal + real NVS2 query -> /docs/guides/local-first-crdt
    await cdp.eval(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true, cancelable: true }))`);
    await cdp.waitFor(`!!document.querySelector('[data-testid="docs-search-dialog"]')`, 4000, "⌘K search modal open");
    await cdp.eval(`(() => {
      const inp = document.querySelector('[data-testid="docs-search-input"]');
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
      setter.call(inp, "CRDT");
      inp.dispatchEvent(new Event("input", { bubbles: true }));
    })()`);
    await cdp.waitFor(
      `document.querySelector('[data-testid="docs-search-hit-0"]')?.textContent?.toLowerCase()?.includes("crdt")`,
      4000,
      "NVS2 search filtering for CRDT"
    );
    await cdp.eval(`document.querySelector('[data-testid="docs-search-hit-0"]').click()`);
    await cdp.waitFor(
      `location.pathname.endsWith("/docs/guides/local-first-crdt") && !document.querySelector('[data-testid="docs-search-dialog"]')`,
      5000,
      "search hit navigation to /docs/guides/local-first-crdt"
    );
    assert((await cdp.eval(`window.__SPA_ALIVE__`)) === "yes", "Search navigation reloaded the page!");

    await cdp.eval(`document.querySelector('[data-testid="pg-crdt-a-edit"]').click()`);
    await cdp.eval(`document.querySelector('[data-testid="pg-crdt-b-edit"]').click()`);
    await cdp.eval(`document.querySelector('[data-testid="pg-crdt-sync"]').click()`);
    await cdp.waitFor(
      `document.querySelector('[data-testid="pg-crdt-convergence"]')?.textContent?.includes("Converged") && !document.querySelector('[data-testid="pg-crdt-bytes"]')?.textContent?.includes("0 B")`,
      4000,
      "CRDT simulator NVC1 sync"
    );
    console.log("    ✓ ⌘K NVS2 search + CRDT binary NVC1 convergence passed with 0 reloads");

    assert(cdp.consoleErrors.length === 0, `Desktop console errors: ${cdp.consoleErrors.join(" | ")}`);
    assert(cdp.pageErrors.length === 0, `Desktop page errors: ${cdp.pageErrors.join(" | ")}`);
    cdp.close();

    // -----------------------------------------------------------------------
    // 3. MOBILE RESPONSIVE CHECK (390x844) ON LANDING + WAVE C2B ZERO-CLS / REPLAY / SPA ON BENCHMARKS
    // -----------------------------------------------------------------------
    console.log("==> [3/5] Verifying mobile 390×844 Landing, zero-flicker /docs/reference/benchmarks, CLS===0, throttled-WASM replay & SPA nav...");
    const { cdp: mobCdp } = await createPageCdp(chrome.debugPort);
    await mobCdp.setViewport(390, 844, true, 2);
    await mobCdp.navigate(`${origin}/`);
    await mobCdp.waitFor(
      `window.__NVE_DOCS__ && window.__NVE_DOCS__.viewport_mode === "mobile"`,
      10000,
      "mobile landing hydration & responsive switch"
    );
    const mobInfo = await mobCdp.eval(`(() => {
      const toggle = document.querySelector('[data-testid="docs-mobile-menu-toggle"]');
      return {
        hasMenuToggle: !!toggle && getComputedStyle(toggle).display !== "none",
        hasLanding: !!document.querySelector('[data-testid="landing-page"]'),
        mismatches: window.__NVE_DOCS__.mismatches,
        overflowX: document.documentElement.scrollWidth - window.innerWidth,
      };
    })()`);
    assert(mobInfo.hasMenuToggle, "Mobile missing visible hamburger Menu toggle");
    assert(mobInfo.hasLanding, "Mobile missing landing page");
    assert(mobInfo.mismatches === 0, `Mobile hydration mismatches: ${mobInfo.mismatches}`);
    assert(mobInfo.overflowX <= 2, `Mobile horizontal overflow: ${mobInfo.overflowX}px`);
    await mobCdp.capturePng(path.join(shotsDir, "landing-mobile.png"));

    await mobCdp.eval(`document.querySelector('[data-testid="docs-mobile-menu-toggle"]').click()`);
    await mobCdp.waitFor(`!!document.querySelector('[data-testid="docs-sidebar-drawer"]')`, 4000, "mobile drawer open");
    assert(mobCdp.consoleErrors.length === 0, `Mobile console errors: ${mobCdp.consoleErrors.join(" | ")}`);
    assert(mobCdp.pageErrors.length === 0, `Mobile page errors: ${mobCdp.pageErrors.join(" | ")}`);
    mobCdp.close();
    console.log("    ✓ Mobile 390×844 Landing + drawer verified (landing-mobile.png captured)");

    // 3b. Mobile (390×844) /docs/reference/benchmarks with JS DISABLED
    const { cdp: mobNoJs } = await createPageCdp(chrome.debugPort);
    await mobNoJs.setViewport(390, 844, true, 2);
    await mobNoJs.send("Emulation.setScriptExecutionDisabled", { value: true });
    await mobNoJs.navigate(`${origin}/docs/reference/benchmarks/`);
    const noJsBench = await mobNoJs.eval(`(() => {
      const sidebar = document.querySelector('[data-testid="docs-sidebar-desktop"]');
      const toc = document.querySelector('[data-testid="docs-toc"]');
      const toggle = document.querySelector('[data-testid="docs-mobile-menu-toggle"]');
      return {
        sidebarDisplay: sidebar ? getComputedStyle(sidebar).display : "missing",
        sidebarWidth: sidebar ? sidebar.getBoundingClientRect().width : -1,
        tocDisplay: toc ? getComputedStyle(toc).display : "missing",
        tocWidth: toc ? toc.getBoundingClientRect().width : -1,
        toggleDisplay: toggle ? getComputedStyle(toggle).display : "missing",
        toggleWidth: toggle ? toggle.getBoundingClientRect().width : -1,
        scrollWidth: document.documentElement.scrollWidth,
        hydrated: !!window.__NVE_DOCS__,
      };
    })()`);
    assert(noJsBench.sidebarDisplay === "none" && noJsBench.sidebarWidth === 0, `Mobile JS-off sidebar must be hidden: ${JSON.stringify(noJsBench)}`);
    assert(noJsBench.tocDisplay === "none" && noJsBench.tocWidth === 0, `Mobile JS-off TOC must be hidden: ${JSON.stringify(noJsBench)}`);
    assert(noJsBench.toggleDisplay !== "none" && noJsBench.toggleWidth > 0, `Mobile JS-off menu toggle must be visible: ${JSON.stringify(noJsBench)}`);
    assert(noJsBench.scrollWidth <= 390, `Mobile JS-off scrollWidth ${noJsBench.scrollWidth} > 390`);
    assert(!noJsBench.hydrated, "Mobile JS-off must not hydrate");
    await mobNoJs.capturePng(path.join(shotsDir, "mobile-benchmarks-nojs.png"));
    mobNoJs.close();
    console.log(
      `    ✓ Mobile JS-off /docs/reference/benchmarks: sidebar=none(0px), toc=none(0px), menuToggle=${Math.round(noJsBench.toggleWidth)}px, scrollWidth=${noJsBench.scrollWidth}px`
    );

    // 3c. Mobile (390×844) /docs/reference/benchmarks live hydration + CLS === 0
    const { cdp: mobHyd } = await createPageCdp(chrome.debugPort);
    await mobHyd.setViewport(390, 844, true, 2);
    await mobHyd.send("Page.addScriptToEvaluateOnNewDocument", {
      source: `
        window.__NVE_CLS__ = 0;
        window.__NVE_HYDRATED_DETAIL__ = null;
        try {
          new PerformanceObserver((list) => {
            for (const entry of list.getEntries()) {
              if (!entry.hadRecentInput) window.__NVE_CLS__ += entry.value;
            }
          }).observe({ type: "layout-shift", buffered: true });
        } catch (_) {}
        window.addEventListener("nve:docs-hydrated", (ev) => {
          window.__NVE_HYDRATED_DETAIL__ = ev.detail;
        });
      `,
    });
    await mobHyd.navigate(`${origin}/docs/reference/benchmarks/`);
    await mobHyd.waitFor(`!!window.__NVE_HYDRATED_DETAIL__ || !!window.__NVE_DOCS_ERROR__`, 12000, "mobile benchmarks hydration");
    await mobHyd.eval(`new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))`);
    const mobHydInfo = await mobHyd.eval(`(() => {
      if (window.__NVE_DOCS_ERROR__) return { error: window.__NVE_DOCS_ERROR__ };
      const c = window.__NVE_DOCS__;
      return {
        cls: window.__NVE_CLS__,
        detail: window.__NVE_HYDRATED_DETAIL__,
        adopted: c.adopted_nodes,
        mismatches: c.mismatches,
        replaced: c.replaced_nodes,
        replayed: c.replayed_events,
        reconciled: c.reconciled_dirty_controls,
        scrollWidth: document.documentElement.scrollWidth,
      };
    })()`);
    assert(!mobHydInfo.error, `Mobile benchmarks hydration error: ${mobHydInfo.error}`);
    assert(mobHydInfo.cls < 0.001, `Mobile benchmarks CLS too high: ${mobHydInfo.cls}`);
    assert(mobHydInfo.mismatches === 0, `Mobile benchmarks mismatches=${mobHydInfo.mismatches}`);
    assert(mobHydInfo.replaced === 0, `Mobile benchmarks replaced=${mobHydInfo.replaced}`);
    assert(mobHydInfo.adopted > 50, `Mobile benchmarks adopted=${mobHydInfo.adopted}`);
    assert(mobHydInfo.scrollWidth <= 390, `Mobile hydrated scrollWidth=${mobHydInfo.scrollWidth}`);
    assert(
      typeof mobHydInfo.detail?.replayed_events === "number" &&
      typeof mobHydInfo.detail?.reconciled_dirty_controls === "number",
      `nve:docs-hydrated missing replay telemetry: ${JSON.stringify(mobHydInfo.detail)}`
    );
    await mobHyd.capturePng(path.join(shotsDir, "mobile-benchmarks-hydrated.png"));
    assert(mobHyd.consoleErrors.length === 0, `Mobile benchmarks console errors: ${mobHyd.consoleErrors.join(" | ")}`);
    assert(mobHyd.pageErrors.length === 0, `Mobile benchmarks page errors: ${mobHyd.pageErrors.join(" | ")}`);
    mobHyd.close();
    console.log(
      `    ✓ Mobile hydrated /docs/reference/benchmarks: CLS=${mobHydInfo.cls.toFixed(6)}, adopted=${mobHydInfo.adopted}, mismatches=0, scrollWidth=${mobHydInfo.scrollWidth}px`
    );

    // 3d. Throttled-WASM Pre-Hydration Event Replay (Menu click + Cmd+K shortcut)
    async function runThrottledReplayScenario(actionExpr, assertReadyExpr, shotName, label) {
      const { cdp: rCdp } = await createPageCdp(chrome.debugPort);
      await rCdp.setViewport(390, 844, true, 2);
      const heldRequests = [];
      let releaseResolver = null;
      const wasmHeld = new Promise((resolve) => { releaseResolver = resolve; });
      rCdp.ws.addEventListener("message", (ev) => {
        const msg = JSON.parse(typeof ev.data === "string" ? ev.data : ev.data.toString());
        if (msg.method === "Fetch.requestPaused") {
          heldRequests.push(msg.params.requestId);
          if (releaseResolver) {
            releaseResolver();
            releaseResolver = null;
          }
        }
      });
      await rCdp.send("Fetch.enable", {
        patterns: [{ urlPattern: "*nve_docs_site.js*", requestStage: "Request" }],
      });
      await rCdp.send("Page.addScriptToEvaluateOnNewDocument", {
        source: `
          window.__NVE_HYDRATED_DETAIL__ = null;
          window.addEventListener("nve:docs-hydrated", (ev) => {
            window.__NVE_HYDRATED_DETAIL__ = ev.detail;
          });
        `,
      });
      await rCdp.send("Page.navigate", { url: `${origin}/docs/reference/benchmarks/` });
      await Promise.race([
        wasmHeld,
        new Promise((_, rej) => setTimeout(() => rej(new Error(`Timed out intercepting nve_docs_site.js for ${label}`)), 8000)),
      ]);
      await rCdp.waitFor(
        `typeof window.__nveReplayDrain === "function" && !!document.querySelector('[data-testid="docs-mobile-menu-toggle"]')`,
        8000,
        `${label} SSR DOM & replay script ready`
      );
      const preHydrated = await rCdp.eval(`!!window.__NVE_DOCS__`);
      assert(!preHydrated, `${label}: WASM must still be gated before pre-hydration interaction`);
      await rCdp.eval(actionExpr);
      for (const requestId of heldRequests) {
        await rCdp.send("Fetch.continueRequest", { requestId });
      }
      await rCdp.send("Fetch.disable");
      await rCdp.waitFor(`!!window.__NVE_HYDRATED_DETAIL__`, 12000, `${label} hydration after WASM release`);
      await rCdp.waitFor(assertReadyExpr, 5000, `${label} replayed UI effect`);
      const replayInfo = await rCdp.eval(`({
        replayed: window.__NVE_DOCS__.replayed_events,
        detailReplayed: window.__NVE_HYDRATED_DETAIL__?.replayed_events,
        mismatches: window.__NVE_DOCS__.mismatches,
      })`);
      assert(replayInfo.replayed >= 1, `${label}: expected replayed_events >= 1, got ${replayInfo.replayed}`);
      assert(replayInfo.detailReplayed === replayInfo.replayed, `${label}: CustomEvent replayed_events mismatch`);
      assert(replayInfo.mismatches === 0, `${label}: mismatches=${replayInfo.mismatches}`);
      if (shotName) {
        await rCdp.capturePng(path.join(shotsDir, shotName));
      }
      assert(rCdp.consoleErrors.length === 0, `${label} console errors: ${rCdp.consoleErrors.join(" | ")}`);
      assert(rCdp.pageErrors.length === 0, `${label} page errors: ${rCdp.pageErrors.join(" | ")}`);
      rCdp.close();
      return replayInfo;
    }

    const menuReplay = await runThrottledReplayScenario(
      `document.querySelector('[data-testid="docs-mobile-menu-toggle"]').click()`,
      `!!document.querySelector('[data-testid="docs-sidebar-drawer"]')`,
      "mobile-drawer-replayed.png",
      "throttled-WASM mobile Menu tap replay"
    );
    console.log(`    ✓ Pre-hydration Menu tap replayed upon WASM load (replayed_events=${menuReplay.replayed}, mobile-drawer-replayed.png captured)`);

    const cmdKReplay = await runThrottledReplayScenario(
      `window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true, cancelable: true }))`,
      `!!document.querySelector('[data-testid="docs-search-dialog"]')`,
      null,
      "throttled-WASM Meta+K replay"
    );
    console.log(`    ✓ Pre-hydration Meta+K replayed upon WASM load (replayed_events=${cmdKReplay.replayed})`);

    // 3e. Desktop (1440×900) /docs/reference/benchmarks screenshot + Post-hydration <a href> SPA Navigation
    const { cdp: deskBench } = await createPageCdp(chrome.debugPort);
    await deskBench.setViewport(1440, 900, false, 1);
    await deskBench.navigate(`${origin}/docs/reference/benchmarks/`);
    await deskBench.waitFor(`!!window.__NVE_DOCS__`, 12000, "desktop benchmarks hydration");
    await deskBench.capturePng(path.join(shotsDir, "desktop-benchmarks.png"));
    await deskBench.eval(`window.__nveSpaMarker = 42;`);
    const beforePath = await deskBench.eval(`location.pathname`);
    await deskBench.eval(`document.querySelector('[data-testid="docs-nav-getting-started-quickstart"]').click()`);
    await deskBench.waitFor(
      `location.pathname !== ${JSON.stringify(beforePath)} && location.pathname.includes("/getting-started/quickstart")`,
      5000,
      "post-hydration <a href> SPA navigation"
    );
    const spaMarkerAfter = await deskBench.eval(`window.__nveSpaMarker`);
    assert(spaMarkerAfter === 42, `Post-hydration SPA click caused full page reload! marker=${spaMarkerAfter}`);
    assert(deskBench.consoleErrors.length === 0, `Desktop benchmarks console errors: ${deskBench.consoleErrors.join(" | ")}`);
    assert(deskBench.pageErrors.length === 0, `Desktop benchmarks page errors: ${deskBench.pageErrors.join(" | ")}`);
    deskBench.close();
    console.log("    ✓ Desktop /docs/reference/benchmarks captured + post-hydration <a href> SPA nav preserved window.__nveSpaMarker === 42");

    // 3f. Deep Table & Inline Flow Layout Audit on why-nve, quickstart & benchmarks (1440, 1100, 390)
    const { cdp: layoutCdp } = await createPageCdp(chrome.debugPort);
    const layoutExpr = `(() => {
      const docOverflow = document.documentElement.scrollWidth - window.innerWidth;
      const main = document.querySelector('[data-testid="docs-main-scroll"]');
      const mainOverflow = main ? main.scrollWidth - main.clientWidth : 0;
      const cellOverlaps = [];
      const textOverflows = [];
      for (const tbl of document.querySelectorAll('table, [role="table"]')) {
        const rows = tbl.querySelectorAll('tr, [role="row"]');
        rows.forEach((row, ri) => {
          const cells = Array.from(row.children).filter(
            (c) => ["TH", "TD"].includes(c.tagName) || ["columnheader", "cell"].includes(c.getAttribute("role"))
          );
          for (let i = 0; i < cells.length; i++) {
            const c = cells[i];
            const cr = c.getBoundingClientRect();
            if (i + 1 < cells.length) {
              const nr = cells[i + 1].getBoundingClientRect();
              if (cr.right > nr.left + 1) {
                cellOverlaps.push({ row: ri, col: i, right: Math.round(cr.right), nextLeft: Math.round(nr.left) });
              }
            }
            for (const child of c.querySelectorAll("span, code, a, strong, em, s")) {
              for (const r of child.getClientRects()) {
                if (r.width > 0 && r.right > cr.right + 2) {
                  textOverflows.push({
                    row: ri,
                    col: i,
                    text: (child.textContent || "").slice(0, 32),
                    childRight: Math.round(r.right),
                    cellRight: Math.round(cr.right),
                  });
                }
              }
            }
          }
        });
      }
      const danglingPunct = [];
      for (const code of document.querySelectorAll('[data-testid="docs-inline-code"]')) {
        const cRects = code.getClientRects();
        if (!cRects.length) continue;
        const cFirst = cRects[0];
        const cLast = cRects[cRects.length - 1];
        const prev = code.previousElementSibling;
        if (prev && /[(\\[]$/.test(prev.textContent || "")) {
          const pRects = prev.getClientRects();
          const pLast = pRects[pRects.length - 1];
          if (pLast && Math.abs(cFirst.top - pLast.top) > 12) {
            danglingPunct.push({ kind: "open", prev: (prev.textContent || "").slice(-12), code: code.textContent, dy: Math.round(cFirst.top - pLast.top) });
          }
        }
        const next = code.nextElementSibling;
        if (next && /^[)\\],.;:!?]/.test(next.textContent || "")) {
          const nRects = next.getClientRects();
          const nFirst = nRects[0];
          if (nFirst && Math.abs(nFirst.top - cLast.top) > 12) {
            danglingPunct.push({ kind: "close", code: code.textContent, next: (next.textContent || "").slice(0, 12), dy: Math.round(nFirst.top - cLast.top) });
          }
        }
      }
      return {
        docOverflow,
        mainOverflow,
        cellOverlaps,
        textOverflows: textOverflows.slice(0, 8),
        danglingPunct: danglingPunct.slice(0, 8),
        mismatches: window.__NVE_DOCS__?.mismatches ?? -1,
      };
    })()`;

    for (const [vw, vh, mob] of [[1440, 900, false], [1100, 900, false], [390, 844, true]]) {
      await layoutCdp.setViewport(vw, vh, mob, mob ? 2 : 1);
      for (const route of ["/docs/getting-started/why-nve/", "/docs/getting-started/quickstart/", "/docs/reference/benchmarks/"]) {
        await layoutCdp.navigate(`${origin}${route}`);
        await layoutCdp.waitFor(`!!window.__NVE_DOCS__ || !!window.__NVE_DOCS_ERROR__`, 12000, `${vw}x${vh} ${route} hydration`);
        const res = await layoutCdp.eval(layoutExpr);
        assert(res.mismatches === 0, `${vw}x${vh} ${route}: hydration mismatches=${res.mismatches}`);
        assert(res.docOverflow <= 1, `${vw}x${vh} ${route}: document horizontal overflow ${res.docOverflow}px`);
        assert(res.mainOverflow <= 1, `${vw}x${vh} ${route}: main scroll horizontal overflow ${res.mainOverflow}px`);
        assert(res.cellOverlaps.length === 0, `${vw}x${vh} ${route}: overlapping table cells: ${JSON.stringify(res.cellOverlaps)}`);
        assert(res.textOverflows.length === 0, `${vw}x${vh} ${route}: table cell text overflows: ${JSON.stringify(res.textOverflows)}`);
        assert(res.danglingPunct.length === 0, `${vw}x${vh} ${route}: dangling punctuation around inline code: ${JSON.stringify(res.danglingPunct)}`);
      }
    }
    assert(layoutCdp.consoleErrors.length === 0, `Layout audit console errors: ${layoutCdp.consoleErrors.join(" | ")}`);
    assert(layoutCdp.pageErrors.length === 0, `Layout audit page errors: ${layoutCdp.pageErrors.join(" | ")}`);
    layoutCdp.close();
    console.log("    ✓ Table overlap, overflow & inline-code cohesion audit passed across 1440/1100/390px");

    // -----------------------------------------------------------------------
    // 4. LIVE LAZY-LOADED WASM EXAMPLE APPS (/examples/studio/ & /examples/kitchen-sink/)
    // -----------------------------------------------------------------------
    console.log("==> [4/5] Verifying lazy-loaded live WASM example apps (/examples/studio/ & /examples/kitchen-sink/)...");
    const { cdp: exCdp } = await createPageCdp(chrome.debugPort);
    await exCdp.setViewport(1440, 900, false, 1);

    await exCdp.navigate(`${origin}/examples/studio/`);
    const studioMeta = await exCdp.eval(`({
      hasBack: !!document.querySelector('[data-testid="portal-back-examples"]'),
      hasDocs: !!document.querySelector('[data-testid="portal-docs-link"]'),
      hasFrame: !!document.getElementById("nve-example-frame"),
      mode: document.querySelector(".portal-shell")?.dataset?.bundleMode || "",
    })`);
    assert(studioMeta.hasBack && studioMeta.hasDocs, "Portal Bar missing navigation links on /examples/studio/");

    if (studioMeta.hasFrame) {
      await exCdp.waitFor(
        `(() => {
          const f = document.getElementById("nve-example-frame");
          const txt = f?.contentDocument?.body?.innerText || "";
          return txt.includes("AETHER");
        })()`,
        15000,
        "Studio WASM AETHER render"
      );
      await new Promise((r) => setTimeout(r, 250));
      await exCdp.capturePng(path.join(shotsDir, "example-studio-live.png"));
      console.log(`    ✓ /examples/studio/ live WASM booted (mode=${studioMeta.mode}, rendered AETHER, example-studio-live.png captured)`);

      // Switch to Kitchen Sink via Portal Bar link and verify Count: 0 -> 1
      await exCdp.navigate(`${origin}/examples/kitchen-sink/`);
      await exCdp.waitFor(
        `(() => {
          const f = document.getElementById("nve-example-frame");
          const txt = f?.contentDocument?.body?.innerText || "";
          return txt.includes("NVE Kitchen Sink") && txt.includes("Count: 0");
        })()`,
        15000,
        "Kitchen Sink WASM Count: 0 render"
      );
      await exCdp.eval(`(() => {
        const doc = document.getElementById("nve-example-frame").contentDocument;
        const btn = doc.querySelector('[data-testid="ks-counter-inc"]')
          || Array.from(doc.querySelectorAll("#app [data-nve-id]")).find((el) => el.textContent && el.textContent.trim() === "Increment +");
        if (!btn) throw new Error("Kitchen Sink Increment + button not found");
        btn.click();
      })()`);
      await exCdp.waitFor(
        `(() => {
          const f = document.getElementById("nve-example-frame");
          const txt = f?.contentDocument?.body?.innerText || "";
          return txt.includes("Count: 1");
        })()`,
        5000,
        "Kitchen Sink WASM increment Count: 0 -> 1"
      );
      console.log("    ✓ /examples/kitchen-sink/ live WASM booted and incremented Count: 0 -> 1");
    } else {
      console.log(`    ! /examples/studio/ in fallback mode (${studioMeta.mode}) — skipping live WASM assertions`);
    }

    assert(exCdp.consoleErrors.length === 0, `Examples console errors: ${exCdp.consoleErrors.join(" | ")}`);
    assert(exCdp.pageErrors.length === 0, `Examples page errors: ${exCdp.pageErrors.join(" | ")}`);
    exCdp.close();

    console.log("==> [5/5] ALL HEADLESS CHROME LANDING + DOCS + LIVE WASM EXAMPLES E2E CHECKS PASSED!");
  } finally {
    try { chrome.proc.kill("SIGTERM"); } catch {}
    try { fs.rmSync(chrome.userDataDir, { recursive: true, force: true }); } catch {}
    server.close();
  }
}

run().catch((err) => {
  console.error(err?.stack || String(err));
  process.exit(1);
});
