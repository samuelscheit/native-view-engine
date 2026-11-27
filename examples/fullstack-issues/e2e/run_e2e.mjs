#!/usr/bin/env node
/**
 * Real Headless Chrome CDP E2E suite for `examples/fullstack-issues` + `nve-http-web::FetchTransport`.
 *
 * Zero npm dependencies: Node built-ins + local Chrome/Chromium `--headless=new`.
 *
 * Covers:
 *  1. Live `FetchTransport` matrix in Chrome V8 (CBOR ok + Nve-Invalidate, chunked ReadableStream
 *     max_response_bytes cap, Content-Length cap, manual 302 redirect rejection, timeout_ms, and
 *     mid-flight Future-drop AbortController cancellation).
 *  2. Hydrated Web WASM `IssuesApp` with `HttpOnly` cookie auth (zero-input WebAsyncWakeGuard load,
 *     0ms local FormDecode validation, server 422 FieldErrors + draft preservation, single-submit
 *     no-double-submit proof, CreateIssue + Nve-Invalidate refetch, optimistic CloseIssue, filter
 *     tabs, RenameIssue with FormHiddenField, AddComment, desktop + mobile screenshots).
 *  3. True 0-JS browser mode (`Emulation.setScriptExecutionDisabled`): native HTML `<form>` POST
 *     to `/issues/create` -> HTTP `303 See Other` -> `GET /issues` rendering the created issue.
 *  4. Real browser cross-origin CORS preflight (`OPTIONS` + `POST /rpc/*` with `Nve-Invalidate`
 *     exposure from a distinct origin).
 */

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, "../../..");
const WEB_SHELL_DIR = path.join(REPO_ROOT, "hosts/web-shell");
const ISSUES_PKG_DIR = path.join(WEB_SHELL_DIR, "pkg/issues");

function parseArgs(argv) {
  let screenshotDir = "";
  let skipBuild = false;
  let releaseWasm = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--screenshot-dir") {
      screenshotDir = argv[++i] || "";
    } else if (a === "--no-build") {
      skipBuild = true;
    } else if (a === "--release") {
      releaseWasm = true;
    } else if (a === "-h" || a === "--help") {
      console.log(
        "Usage: node examples/fullstack-issues/e2e/run_e2e.mjs [--screenshot-dir <dir>] [--no-build] [--release]"
      );
      process.exit(0);
    } else {
      throw new Error(`Unknown arg: ${a}`);
    }
  }
  return { screenshotDir, skipBuild, releaseWasm };
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
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  for (const cmd of ["google-chrome", "chromium", "chromium-browser"]) {
    const r = spawnSync("which", [cmd], { encoding: "utf8" });
    if (r.status === 0 && r.stdout.trim()) return r.stdout.trim();
  }
  throw new Error("Chrome/Chromium binary not found. Set NVE_CHROME_BIN.");
}

function runChecked(cmd, args, cwd = REPO_ROOT, extraEnv = null) {
  console.log(`==> ${cmd} ${args.join(" ")}`);
  const env = extraEnv ? { ...process.env, ...extraEnv } : process.env;
  const r = spawnSync(cmd, args, { cwd, env, stdio: "inherit" });
  if (r.status !== 0) {
    throw new Error(`Command failed (${r.status}): ${cmd} ${args.join(" ")}`);
  }
}

function startProbeServer(issuesPkgDir) {
  const timeoutAbortedCount = { value: 0 };
  const dropAbortedCount = { value: 0 };
  const pkgRootPrefix = issuesPkgDir.endsWith(path.sep) ? issuesPkgDir : issuesPkgDir + path.sep;
  const corsHeaders = {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "POST, OPTIONS",
    "Access-Control-Allow-Headers": "content-type, authorization, x-nve-csrf",
    "Access-Control-Expose-Headers": "nve-error-kind, nve-invalidate",
  };
  const MIME = {
    ".html": "text/html; charset=utf-8",
    ".js": "application/javascript; charset=utf-8",
    ".wasm": "application/wasm",
    ".json": "application/json; charset=utf-8",
  };

  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      const url = new URL(req.url || "/", "http://127.0.0.1");
      const p = url.pathname;

      if (req.method === "OPTIONS") {
        res.writeHead(204, corsHeaders);
        res.end();
        return;
      }

      if (p === "/cors-runner") {
        res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
        res.end(`<!doctype html><html><body><div id="cors-root">CORS Runner</div></body></html>`);
        return;
      }

      if (p.startsWith("/pkg/issues/")) {
        const rel = p.slice("/pkg/issues/".length);
        const abs = path.resolve(issuesPkgDir, rel);
        if (!abs.startsWith(pkgRootPrefix) || !fs.existsSync(abs) || !fs.statSync(abs).isFile()) {
          res.writeHead(404);
          res.end("Not Found");
          return;
        }
        const ext = path.extname(abs);
        res.writeHead(200, { "Content-Type": MIME[ext] || "application/octet-stream" });
        fs.createReadStream(abs).pipe(res);
        return;
      }

      if (p === "/probe/rpc/probe.v1.Test/Ok") {
        // Return valid CBOR u64(42) = 0x18 0x2a + Nve-Invalidate tag.
        res.writeHead(200, {
          ...corsHeaders,
          "Content-Type": "application/cbor",
          "Nve-Invalidate": "0000000000000007,000000000000002a",
        });
        res.end(Buffer.from([0x18, 0x2a]));
        return;
      }

      if (p === "/probe/rpc/probe.v1.Test/StreamOversize") {
        res.writeHead(200, {
          ...corsHeaders,
          "Content-Type": "application/cbor",
          "Transfer-Encoding": "chunked",
        });
        let sent = 0;
        const iv = setInterval(() => {
          if (res.destroyed || sent >= 8) {
            clearInterval(iv);
            if (!res.destroyed) res.end();
            return;
          }
          sent++;
          res.write(Buffer.alloc(1024, 0x41));
        }, 5);
        req.on("close", () => clearInterval(iv));
        return;
      }

      if (p === "/probe/rpc/probe.v1.Test/ContentLengthOversize") {
        res.writeHead(200, {
          ...corsHeaders,
          "Content-Type": "application/cbor",
          "Content-Length": "9999999",
        });
        res.write(Buffer.alloc(64, 0x42));
        return;
      }

      if (p === "/probe/rpc/probe.v1.Test/Redirect") {
        res.writeHead(302, {
          ...corsHeaders,
          Location: "/probe/rpc/probe.v1.Test/Ok",
        });
        res.end();
        return;
      }

      if (p === "/probe/rpc/probe.v1.Test/SlowTimeout" || p === "/probe/rpc/probe.v1.Test/SlowDrop") {
        const isDrop = p.endsWith("/SlowDrop");
        const t = setTimeout(() => {
          if (!res.destroyed) {
            res.writeHead(200, { ...corsHeaders, "Content-Type": "application/cbor" });
            res.end(Buffer.from([0x01]));
          }
        }, 5000);
        res.on("close", () => {
          clearTimeout(t);
          if (!res.writableEnded) {
            if (isDrop) dropAbortedCount.value++;
            else timeoutAbortedCount.value++;
          }
        });
        return;
      }

      res.writeHead(404, corsHeaders);
      res.end("Not Found");
    });
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      resolve({ server, port: addr.port, timeoutAbortedCount, dropAbortedCount });
    });
  });
}

function startIssuesServer(binPath, wasmDir, corsOrigin) {
  return new Promise((resolve, reject) => {
    const env = {
      ...process.env,
      NVE_ISSUES_BIND: "127.0.0.1:0",
      NVE_ISSUES_WASM_DIR: wasmDir,
    };
    if (corsOrigin) {
      env.NVE_ISSUES_CORS_ORIGIN = corsOrigin;
    }
    const proc = spawn(binPath, [], {
      cwd: REPO_ROOT,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let out = "";
    let err = "";
    let settled = false;
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        try { proc.kill("SIGKILL"); } catch {}
        reject(new Error(`Timed out starting issues server.\nstdout:\n${out}\nstderr:\n${err}`));
      }
    }, 15000);

    proc.stdout.on("data", (chunk) => {
      out += chunk.toString("utf8");
      const m = out.match(/listening on http:\/\/127\.0\.0\.1:(\d+)/);
      if (m && !settled) {
        settled = true;
        clearTimeout(timer);
        resolve({ proc, port: Number(m[1]), getLogs: () => ({ out, err }) });
      }
    });
    proc.stderr.on("data", (chunk) => {
      err += chunk.toString("utf8");
    });
    proc.on("exit", (code, sig) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        reject(new Error(`issues server exited early (${code}/${sig}).\nstdout:\n${out}\nstderr:\n${err}`));
      }
    });
  });
}

function launchHeadlessChrome(chromeBin) {
  return new Promise((resolve, reject) => {
    const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "nve-issues-e2e-chrome-"));
    const proc = spawn(
      chromeBin,
      [
        "--headless=new",
        "--remote-debugging-port=0",
        `--user-data-dir=${userDataDir}`,
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-default-apps",
        "--disable-extensions",
        "--disable-sync",
        "--metrics-recording-only",
        "--mute-audio",
        "--window-size=1280,840",
        "about:blank",
      ],
      { stdio: ["ignore", "pipe", "pipe"] }
    );
    let stderrBuf = "";
    let settled = false;
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        try { proc.kill("SIGKILL"); } catch {}
        reject(new Error(`Chrome launch timeout:\n${stderrBuf}`));
      }
    }, 15000);
    proc.stderr.on("data", (c) => {
      stderrBuf += c.toString("utf8");
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
        reject(new Error(`Chrome exited early (${code}/${sig}):\n${stderrBuf}`));
      }
    });
  });
}

class CdpClient {
  constructor(wsUrl) {
    this.wsUrl = wsUrl;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Map();
    this.ws = null;
  }
  async connect() {
    await new Promise((resolve, reject) => {
      const ws = new WebSocket(this.wsUrl);
      this.ws = ws;
      ws.addEventListener("open", () => resolve());
      ws.addEventListener("error", (ev) => reject(new Error(`CDP WS error: ${ev.message || "unknown"}`)));
      ws.addEventListener("message", (ev) => {
        const msg = JSON.parse(String(ev.data));
        if (msg.id !== undefined) {
          const e = this.pending.get(msg.id);
          if (e) {
            this.pending.delete(msg.id);
            if (msg.error) e.reject(new Error(`CDP ${e.method}: ${JSON.stringify(msg.error)}`));
            else e.resolve(msg.result);
          }
        } else if (msg.method) {
          for (const fn of this.listeners.get(msg.method) || []) fn(msg.params);
        }
      });
    });
  }
  on(method, fn) {
    if (!this.listeners.has(method)) this.listeners.set(method, []);
    this.listeners.get(method).push(fn);
  }
  off(method, fn) {
    const arr = this.listeners.get(method);
    if (!arr) return;
    const idx = arr.indexOf(fn);
    if (idx >= 0) arr.splice(idx, 1);
  }
  send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      this.pending.set(id, { resolve, reject, method });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  close() {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) this.ws.close();
  }
}

async function evalJs(cdp, expression) {
  const res = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (res.exceptionDetails) {
    throw new Error(
      `JS threw: ${res.exceptionDetails.text} ${JSON.stringify(res.exceptionDetails.exception || {})}`
    );
  }
  return res.result ? res.result.value : undefined;
}

async function waitFor(cdp, desc, fnSrc, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  let lastErr = null;
  while (Date.now() < deadline) {
    try {
      const ok = await evalJs(cdp, `Boolean((${fnSrc})())`);
      if (ok) return;
    } catch (e) {
      lastErr = e;
    }
    await new Promise((r) => setTimeout(r, 40));
  }
  const snap = await evalJs(
    cdp,
    `({ url: location.href, text: document.body?.innerText?.slice(0, 600), html: document.body?.innerHTML?.slice(0, 600) })`
  ).catch(() => null);
  throw new Error(`Timed out waiting for: ${desc}\nLast err: ${lastErr}\nSnapshot: ${JSON.stringify(snap, null, 2)}`);
}

async function openTab(debugPort) {
  const targets = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
  const page = targets.find((t) => t.type === "page");
  if (!page) throw new Error("No Chrome page target found");
  const cdp = new CdpClient(page.webSocketDebuggerUrl);
  await cdp.connect();
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Network.enable");
  return cdp;
}

async function navigateAndWait(cdp, url, timeoutMs = 15000) {
  const loaded = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cdp.off("Page.loadEventFired", handler);
      reject(new Error(`Timed out waiting for Page.loadEventFired on ${url}`));
    }, timeoutMs);
    const handler = () => {
      clearTimeout(timer);
      cdp.off("Page.loadEventFired", handler);
      resolve();
    };
    cdp.on("Page.loadEventFired", handler);
  });
  await cdp.send("Page.navigate", { url });
  await loaded;
}

async function setInputValue(cdp, selector, value) {
  await evalJs(
    cdp,
    `(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      if (!el) throw new Error("Missing input: " + ${JSON.stringify(selector)});
      el.focus();
      el.value = ${JSON.stringify(value)};
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    })()`
  );
}

async function capturePng(cdp, filePath) {
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(filePath, Buffer.from(data, "base64"));
  console.log(`  [screenshot] ${filePath}`);
}

async function main() {
  const { screenshotDir, skipBuild, releaseWasm } = parseArgs(process.argv.slice(2));
  if (!skipBuild) {
    runChecked("cargo", ["build", "--locked", "-p", "nve-example-issues-server"]);
    const wasmArgs = releaseWasm ? ["--release", "issues"] : ["--dev", "issues"];
    runChecked(path.join(WEB_SHELL_DIR, "build.sh"), wasmArgs, REPO_ROOT, {
      NVE_ISSUES_PROBES: "1",
    });
  }
  const serverBin = path.join(REPO_ROOT, "target/debug/nve-example-issues-server");
  if (!fs.existsSync(serverBin)) {
    throw new Error(`Missing server binary: ${serverBin}`);
  }
  if (!fs.existsSync(path.join(ISSUES_PKG_DIR, "manifest.json"))) {
    throw new Error(`Missing WASM bundle: ${ISSUES_PKG_DIR}/manifest.json`);
  }

  const probe = await startProbeServer(ISSUES_PKG_DIR);
  const probeOrigin = `http://127.0.0.1:${probe.port}`;
  const issuesSrv = await startIssuesServer(serverBin, ISSUES_PKG_DIR, probeOrigin);
  const issuesOrigin = `http://127.0.0.1:${issuesSrv.port}`;
  const chromeBin = findChromeBinary();
  const chrome = await launchHeadlessChrome(chromeBin);

  const consoleErrors = [];
  const rpcRequests = [];

  try {
    const cdp = await openTab(chrome.debugPort);
    cdp.on("Runtime.exceptionThrown", (p) => {
      const msg = p.exceptionDetails?.exception?.description || p.exceptionDetails?.text || JSON.stringify(p);
      console.error("  [browser exception]", msg);
      consoleErrors.push(msg);
    });
    cdp.on("Runtime.consoleAPICalled", (p) => {
      const line = p.args.map((a) => a.value ?? a.description ?? "").join(" ");
      if (p.type === "error" || p.type === "warning") {
        console.error(`  [browser ${p.type}]`, line);
      }
      if (p.type === "error") {
        consoleErrors.push(line);
      }
    });
    cdp.on("Network.responseReceived", (p) => {
      if (p.response.status >= 400) {
        console.error(`  [http ${p.response.status}] ${p.response.url}`);
      }
    });
    cdp.on("Network.requestWillBeSent", (p) => {
      if (p.request.url.includes("/rpc/")) {
        rpcRequests.push({ method: p.request.method, url: p.request.url });
      }
    });

    // Set HttpOnly session cookie (invisible to document.cookie, sent automatically by browser).
    await cdp.send("Network.setCookie", {
      name: "nve_issues_session",
      value: "web-demo-session",
      domain: "127.0.0.1",
      path: "/",
      httpOnly: true,
    });

    // -------------------------------------------------------------------------
    // 1. Hydrated Web WASM IssuesApp (Cookie auth + zero-input async wake)
    // -------------------------------------------------------------------------
    console.log("==> [1/4] Hydrated Web WASM IssuesApp (cookie auth + zero-input wake)");
    await navigateAndWait(cdp, `${issuesOrigin}/issues`);

    const docCookie = await evalJs(cdp, "document.cookie");
    if (docCookie !== "") {
      throw new Error(`Expected HttpOnly cookie invisible to JS, got document.cookie=${docCookie}`);
    }

    await waitFor(
      cdp,
      "initial 3 issues loaded via FetchTransport with zero user input",
      `() => Boolean(document.querySelector('[testid="issue-row-1"]') && document.querySelector('[testid="issue-row-2"]') && document.querySelector('[testid="issue-row-3"]'))`
    );
    console.log("  PASS: Zero-input async FetchTransport loaded all 3 seeded issues.");

    if (screenshotDir) {
      fs.mkdirSync(screenshotDir, { recursive: true });
      await cdp.send("Emulation.setDeviceMetricsOverride", {
        width: 1280,
        height: 840,
        deviceScaleFactor: 1,
        mobile: false,
      });
      await new Promise((r) => setTimeout(r, 150));
      await capturePng(cdp, path.join(screenshotDir, "issues-desktop.png"));
    }

    // 1a. Local 0ms FormDecode validation (empty title -> 0 network RPC calls!).
    const createBeforeLocal = rpcRequests.filter((r) => r.url.includes("Issues/Create")).length;
    await evalJs(cdp, `document.querySelector('[testid="create-submit-btn"]').click()`);
    await waitFor(
      cdp,
      "0ms local FormDecode validation error rendered inline",
      `() => document.body.innerText.includes("title must not be empty")`
    );
    await new Promise((r) => setTimeout(r, 120));
    const createAfterLocal = rpcRequests.filter((r) => r.url.includes("Issues/Create")).length;
    if (createAfterLocal !== createBeforeLocal) {
      throw new Error(`Local validation fired ${createAfterLocal - createBeforeLocal} RPC calls (expected 0)`);
    }
    console.log("  PASS: Local 0ms FormDecode blocked empty submit with 0 network requests.");

    // 1b. Server 422 duplicate-title FieldErrors + draft preservation + single-submit check.
    const dupTitle = "Add H0 preloaded SSR + fail-closed hydration";
    const createBefore422 = rpcRequests.filter((r) => r.url.includes("Issues/Create")).length;
    await setInputValue(cdp, '[testid="create-title-input"]', dupTitle);
    await evalJs(cdp, `document.querySelector('[testid="create-submit-btn"]').click()`);
    await waitFor(
      cdp,
      "server 422 duplicate-title FieldErrors rendered inline",
      `() => document.body.innerText.includes("an issue with this title already exists")`
    );
    await new Promise((r) => setTimeout(r, 120));
    const createAfter422 = rpcRequests.filter((r) => r.url.includes("Issues/Create")).length;
    if (createAfter422 - createBefore422 !== 1) {
      throw new Error(
        `Expected exactly 1 CreateIssue RPC call (no Web double-submit!), saw ${createAfter422 - createBefore422}`
      );
    }
    const preservedDraft = await evalJs(
      cdp,
      `({ val: document.querySelector('[testid="create-title-input"]')?.value, dirty: document.querySelector('[testid="create-issue-form"]')?.getAttribute("data-nve-dirty") })`
    );
    if (preservedDraft.val !== dupTitle || preservedDraft.dirty !== "true") {
      throw new Error(`Expected dirty draft preserved after 422, got ${JSON.stringify(preservedDraft)}`);
    }
    console.log("  PASS: Server 422 FieldErrors bound inline, draft preserved, exactly 1 RPC (zero double-submit).");

    // 1c. Valid CreateIssue -> Nve-Invalidate auto-refetch -> #4 appears + input resets.
    const createBeforeOk = rpcRequests.filter((r) => r.url.includes("Issues/Create")).length;
    await setInputValue(cdp, '[testid="create-title-input"]', "Live Chrome WASM Issue");
    await evalJs(cdp, `document.querySelector('[testid="create-submit-btn"]').click()`);
    await waitFor(
      cdp,
      "created issue #4 appears via Nve-Invalidate refetch",
      `() => document.querySelector('[testid="issue-title-4"]')?.textContent?.includes("Live Chrome WASM Issue")`
    );
    const createAfterOk = rpcRequests.filter((r) => r.url.includes("Issues/Create")).length;
    if (createAfterOk - createBeforeOk !== 1) {
      throw new Error(`Expected 1 CreateIssue call on valid submit, saw ${createAfterOk - createBeforeOk}`);
    }
    const resetAfterOk = await evalJs(
      cdp,
      `({ val: document.querySelector('[testid="create-title-input"]')?.value, dirty: document.querySelector('[testid="create-issue-form"]')?.getAttribute("data-nve-dirty") })`
    );
    if (resetAfterOk.val !== "" || resetAfterOk.dirty !== "false") {
      throw new Error(`Expected create-title-input to reset cleanly after CreateIssue, got ${JSON.stringify(resetAfterOk)}`);
    }
    console.log("  PASS: CreateIssue succeeded (1 RPC), reset form cleanly, and auto-refetched #4 via Nve-Invalidate.");

    // 1d. Optimistic CloseIssue (pause network via CDP Fetch.enable to prove instant optimistic UI BEFORE server replies!) + Filter Tabs.
    await cdp.send("Fetch.enable", {
      patterns: [{ urlPattern: "*Issues/Close*", requestStage: "Request" }],
    });
    let pausedCloseReqId = null;
    const closePaused = new Promise((resolve) => {
      const onPaused = (p) => {
        pausedCloseReqId = p.requestId;
        cdp.off("Fetch.requestPaused", onPaused);
        resolve();
      };
      cdp.on("Fetch.requestPaused", onPaused);
    });
    await evalJs(cdp, `document.querySelector('[testid="issue-close-4"]').click()`);
    await closePaused;
    await waitFor(
      cdp,
      "optimistic CloseIssue shows Closed BEFORE server responds",
      `() => document.querySelector('[testid="issue-state-4"]')?.textContent?.includes("Closed")`
    );
    await cdp.send("Fetch.continueRequest", { requestId: pausedCloseReqId });
    await cdp.send("Fetch.disable");
    await waitFor(
      cdp,
      "confirmed CloseIssue remains Closed after server response",
      `() => document.querySelector('[testid="issue-state-4"]')?.textContent?.includes("Closed")`
    );
    await evalJs(cdp, `document.querySelector('[testid="filter-closed"]').click()`);
    await waitFor(
      cdp,
      "Closed filter shows #4 and hides open #2",
      `() => Boolean(document.querySelector('[testid="issue-row-4"]')) && !document.querySelector('[testid="issue-row-2"]')`
    );
    await evalJs(cdp, `document.querySelector('[testid="filter-all"]').click()`);
    await waitFor(
      cdp,
      "All filter restores #2",
      `() => Boolean(document.querySelector('[testid="issue-row-2"]'))`
    );
    console.log("  PASS: Optimistic CloseIssue and filter query switching work cleanly.");

    // 1e. Inline RenameIssue (FormHiddenField) + AddComment.
    await evalJs(cdp, `document.querySelector('[testid="issue-select-2"]').click()`);
    await waitFor(
      cdp,
      "detail panel opens for #2",
      `() => Boolean(document.querySelector('[testid="rename-title-input"]'))`
    );
    await setInputValue(cdp, '[testid="rename-title-input"]', "Renamed Live in Chrome");
    await evalJs(cdp, `document.querySelector('[testid="rename-submit-btn"]').click()`);
    await waitFor(
      cdp,
      "issue #2 renamed in list",
      `() => document.querySelector('[testid="issue-title-2"]')?.textContent?.includes("Renamed Live in Chrome")`
    );
    await setInputValue(cdp, '[testid="comment-body-input"]', "Browser E2E comment");
    await evalJs(cdp, `document.querySelector('[testid="comment-submit-btn"]').click()`);
    await waitFor(
      cdp,
      "comment appears in detail panel",
      `() => document.body.innerText.includes("Browser E2E comment")`
    );
    console.log("  PASS: ActionForm<RenameIssue> (with FormHiddenField) and ActionForm<AddComment> work live.");

    if (screenshotDir) {
      await capturePng(cdp, path.join(screenshotDir, "issues-desktop-detail.png"));
      await cdp.send("Emulation.setDeviceMetricsOverride", {
        width: 390,
        height: 844,
        deviceScaleFactor: 2,
        mobile: true,
      });
      await new Promise((r) => setTimeout(r, 180));
      await capturePng(cdp, path.join(screenshotDir, "issues-mobile.png"));
      await cdp.send("Emulation.clearDeviceMetricsOverride");
    }

    // -------------------------------------------------------------------------
    // 2. Live FetchTransport Browser Matrix (in Chrome V8 + Fetch API)
    // -------------------------------------------------------------------------
    console.log("==> [2/4] Live FetchTransport edge-case matrix in Chrome");
    const probeReport = await evalJs(
      cdp,
      `window.__nveIssuesMod.nve_probe_fetch_transport(${JSON.stringify(probeOrigin)})`
    );
    console.log("  FetchTransport probe result:", probeReport);
    if (!String(probeReport).startsWith("OK:")) {
      throw new Error(`FetchTransport browser probe failed: ${probeReport}`);
    }
    if (probe.timeoutAbortedCount.value < 1 || probe.dropAbortedCount.value < 1) {
      throw new Error(
        `Expected BOTH timeout and dropped FetchPromiseFuture to abort server streams (timeoutAborted=${probe.timeoutAbortedCount.value}, dropAborted=${probe.dropAbortedCount.value})`
      );
    }
    console.log(
      `  PASS: All FetchTransport browser probes passed (timeoutAborted=${probe.timeoutAbortedCount.value}, dropAborted=${probe.dropAbortedCount.value}).`
    );

    // -------------------------------------------------------------------------
    // 3. Real Cross-Origin CORS Preflight (OPTIONS + POST from distinct origin)
    // -------------------------------------------------------------------------
    console.log("==> [3/4] Real cross-origin CORS preflight from distinct browser origin");
    await navigateAndWait(cdp, `${probeOrigin}/cors-runner`);
    const corsResult = await evalJs(
      cdp,
      `(async () => {
        const manifest = await (await fetch("/pkg/issues/manifest.json")).json();
        const mod = await import("/pkg/issues/" + manifest.js);
        await mod.default({ module_or_path: "/pkg/issues/" + manifest.wasm });
        return await mod.nve_probe_cross_origin_cors(${JSON.stringify(issuesOrigin)});
      })()`
    );
    console.log("  Cross-origin CORS probe result:", corsResult);
    if (!String(corsResult).startsWith("OK:")) {
      throw new Error(`Cross-origin CORS browser probe failed: ${corsResult}`);
    }
    console.log("  PASS: Cross-origin CORS preflight + CBOR RPC + exposed Nve-Invalidate header verified in Chrome.");

    // -------------------------------------------------------------------------
    // 4. 0-JS Browser Mode (script execution disabled -> native HTML <form> 303 PRG)
    // -------------------------------------------------------------------------
    console.log("==> [4/4] True 0-JS browser mode (HTML <form method=post> -> 303 -> GET /issues)");
    await cdp.send("Emulation.setScriptExecutionDisabled", { value: true });
    await navigateAndWait(cdp, `${issuesOrigin}/issues`);

    // CDP Runtime.evaluate still works for test inspection even with page scripts disabled,
    // OR verify via DOM + real form.submit()! Let's check `<form action="/issues/create">` exists,
    // set its input value, and submit via native HTMLFormElement.prototype.submit (zero page JS!).
    const zeroJsBefore = await evalJs(
      cdp,
      `({
        hasWasmMount: Boolean(document.getElementById("app")?.getAttribute("data-nve-live")),
        hasCreateForm: Boolean(document.querySelector('form[action="/issues/create"][method="post"]')),
        hasLiveIssue: document.body.innerText.includes("Live Chrome WASM Issue"),
      })`
    );
    if (zeroJsBefore.hasWasmMount) {
      throw new Error("WASM should NOT have mounted when script execution is disabled!");
    }
    if (!zeroJsBefore.hasCreateForm || !zeroJsBefore.hasLiveIssue) {
      throw new Error(`0-JS SSR page missing expected form/issues: ${JSON.stringify(zeroJsBefore)}`);
    }

    const navDone = new Promise((resolve) => {
      const h = () => resolve();
      cdp.on("Page.loadEventFired", h);
    });
    await evalJs(
      cdp,
      `(() => {
        const form = document.querySelector('form[action="/issues/create"]');
        const input = form.querySelector('input[name="title"]');
        input.value = "Created via 0-JS HTML Form";
        form.requestSubmit ? form.requestSubmit() : form.submit();
      })()`
    );
    await navDone;

    const zeroJsAfter = await evalJs(
      cdp,
      `({
        path: location.pathname,
        hasCreated: document.body.innerText.includes("Created via 0-JS HTML Form"),
        hasWasmMount: Boolean(document.getElementById("app")?.getAttribute("data-nve-live")),
      })`
    );
    if (zeroJsAfter.path !== "/issues" || !zeroJsAfter.hasCreated || zeroJsAfter.hasWasmMount) {
      throw new Error(`0-JS HTML form 303 PRG failed: ${JSON.stringify(zeroJsAfter)}`);
    }
    console.log("  PASS: 0-JS HTML <form method=post> -> 303 See Other -> GET /issues created issue with zero JS.");

    if (consoleErrors.length > 0) {
      throw new Error(`Unexpected browser console/page errors:\n${consoleErrors.join("\n")}`);
    }

    cdp.close();
    console.log("\nALL 4 LIVE CHROME FULLSTACK E2E SUITES PASSED!");
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
