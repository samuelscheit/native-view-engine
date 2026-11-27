/**
 * Zero-dependency Headless Chrome CDP + process harness for `examples/fullstack-issues/e2e`:
 * Chrome discovery/launch, a minimal CDP client, the CORS/FetchTransport probe server, and the
 * issues server launcher (`NVE_ISSUES_TEST_HOOKS=1`).
 */

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
export const REPO_ROOT = path.resolve(__dirname, "../../..");
export const WEB_SHELL_DIR = path.join(REPO_ROOT, "hosts/web-shell");
export const ISSUES_PKG_DIR = path.join(WEB_SHELL_DIR, "pkg/issues");

export function parseArgs(argv) {
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

export function findChromeBinary() {
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

export function runChecked(cmd, args, cwd = REPO_ROOT, extraEnv = null) {
  console.log(`==> ${cmd} ${args.join(" ")}`);
  const env = extraEnv ? { ...process.env, ...extraEnv } : process.env;
  const r = spawnSync(cmd, args, { cwd, env, stdio: "inherit" });
  if (r.status !== 0) {
    throw new Error(`Command failed (${r.status}): ${cmd} ${args.join(" ")}`);
  }
}

export function startProbeServer(issuesPkgDir) {
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

export function startIssuesServer(binPath, wasmDir, corsOrigin) {
  return new Promise((resolve, reject) => {
    const env = {
      ...process.env,
      NVE_ISSUES_BIND: "127.0.0.1:0",
      NVE_ISSUES_WASM_DIR: wasmDir,
      NVE_ISSUES_TEST_HOOKS: "1",
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

export function launchHeadlessChrome(chromeBin) {
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

export class CdpClient {
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
          for (const fn of [...(this.listeners.get(msg.method) || [])]) fn(msg.params);
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

export async function evalJs(cdp, expression) {
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

export async function waitFor(cdp, desc, fnSrc, timeoutMs = 15000) {
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

export async function openTab(debugPort) {
  const targets = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
  const page = targets.find((t) => t.type === "page");
  if (!page) throw new Error("No Chrome page target found");
  const cdp = new CdpClient(page.webSocketDebuggerUrl);
  await cdp.connect();
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Network.enable");
  await cdp.send("Log.enable");
  return cdp;
}

export function waitForEvent(cdp, method, predicate = () => true, timeoutMs = 15000, desc = method) {
  let timer = null;
  let handler = null;
  const cancel = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    if (handler !== null) {
      cdp.off(method, handler);
      handler = null;
    }
  };
  const promise = new Promise((resolve, reject) => {
    timer = setTimeout(() => {
      cancel();
      reject(new Error(`Timed out waiting for CDP event: ${desc}`));
    }, timeoutMs);
    handler = (params) => {
      if (predicate(params)) {
        cancel();
        resolve(params);
      }
    };
    cdp.on(method, handler);
  });
  // Suppress unhandledRejection if a timeout fires before the caller attaches `await`.
  promise.catch(() => {});
  return { promise, cancel };
}

export async function navigateAndWait(cdp, url, timeoutMs = 15000) {
  const loaded = waitForEvent(
    cdp,
    "Page.loadEventFired",
    () => true,
    timeoutMs,
    `Page.loadEventFired on ${url}`
  );
  try {
    await cdp.send("Page.navigate", { url });
    await loaded.promise;
  } catch (err) {
    loaded.cancel();
    throw err;
  }
}

/**
 * Navigates to `url` and resolves with the main document's `{ status, headers }` (lowercased
 * header names). With `waitLoad: false` it resolves on response headers only, so it works while
 * a streaming SSR response is still held open by the server.
 */
export async function navigateDocument(cdp, url, { waitLoad = true, timeoutMs = 15000 } = {}) {
  const docResponse = waitForEvent(
    cdp,
    "Network.responseReceived",
    (p) => p.type === "Document" && p.response.url === url,
    timeoutMs,
    `document response for ${url}`
  );
  const loaded = waitLoad
    ? waitForEvent(cdp, "Page.loadEventFired", () => true, timeoutMs, `Page.loadEventFired on ${url}`)
    : null;
  try {
    await cdp.send("Page.navigate", { url });
    const { response } = await docResponse.promise;
    if (loaded) await loaded.promise;
    const headers = Object.fromEntries(
      Object.entries(response.headers).map(([k, v]) => [k.toLowerCase(), v])
    );
    return { status: response.status, headers };
  } catch (err) {
    docResponse.cancel();
    loaded?.cancel();
    throw err;
  }
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function setInputValue(cdp, selector, value) {
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

export async function capturePng(cdp, filePath) {
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(filePath, Buffer.from(data, "base64"));
  console.log(`  [screenshot] ${filePath}`);
}

