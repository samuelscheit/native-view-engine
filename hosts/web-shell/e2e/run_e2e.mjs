#!/usr/bin/env node
/**
 * Self-contained headless Chrome CDP E2E suite for `hosts/web-shell` (Web WASM).
 *
 * Zero external npm dependencies: uses Node built-ins (`http`, `fs`, `child_process`,
 * global `WebSocket`, global `fetch`) + local Chrome/Chromium `--headless=new`.
 *
 * Usage:
 *   node hosts/web-shell/e2e/run_e2e.mjs [--screenshot /tmp/nve_web_studio_e2e.png] [--rebuild] [--dev]
 */

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const WEB_SHELL_DIR = path.resolve(__dirname, "..");
const PKG_DIR = path.join(WEB_SHELL_DIR, "pkg");

const MIME_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".mjs": "application/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
};

function parseArgs(argv) {
  let screenshotPath = "/tmp/nve_web_studio_e2e.png";
  let rebuild = false;
  let devBuild = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--screenshot") {
      if (i + 1 >= argv.length) {
        throw new Error("--screenshot requires a file path");
      }
      screenshotPath = argv[++i];
    } else if (arg === "--no-screenshot") {
      screenshotPath = "";
    } else if (arg === "--rebuild") {
      rebuild = true;
    } else if (arg === "--dev") {
      devBuild = true;
    } else if (arg === "-h" || arg === "--help") {
      console.log(
        "Usage: node hosts/web-shell/e2e/run_e2e.mjs [--screenshot <out.png>] [--no-screenshot] [--rebuild] [--dev]"
      );
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return { screenshotPath, rebuild, devBuild };
}

function ensureWasmBuilt(rebuild, devBuild) {
  const jsOut = path.join(PKG_DIR, "nve_web_shell.js");
  const wasmOut = path.join(PKG_DIR, "nve_web_shell_bg.wasm");
  if (!rebuild && fs.existsSync(jsOut) && fs.existsSync(wasmOut)) {
    return;
  }
  const buildScript = path.join(WEB_SHELL_DIR, "build.sh");
  const args = devBuild ? ["--dev"] : [];
  console.log(`==> Building Web WASM bundle via ${buildScript} ${args.join(" ")}...`);
  const res = spawnSync(buildScript, args, { stdio: "inherit" });
  if (res.status !== 0) {
    throw new Error(`hosts/web-shell/build.sh failed with exit code ${res.status}`);
  }
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
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  for (const cmd of ["google-chrome", "chromium", "chromium-browser"]) {
    const res = spawnSync("which", [cmd], { encoding: "utf8" });
    if (res.status === 0 && res.stdout.trim()) {
      return res.stdout.trim();
    }
  }

  throw new Error(
    "Chrome/Chromium binary not found. Set NVE_CHROME_BIN or install Google Chrome."
  );
}

function startStaticServer(rootDir) {
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      const reqUrl = new URL(req.url || "/", "http://127.0.0.1");
      let relPath = decodeURIComponent(reqUrl.pathname);
      if (relPath === "/") {
        relPath = "/index.html";
      }
      const absPath = path.resolve(rootDir, "." + relPath);
      if (!absPath.startsWith(rootDir)) {
        res.writeHead(403);
        res.end("Forbidden");
        return;
      }
      fs.stat(absPath, (err, stat) => {
        if (err || !stat.isFile()) {
          res.writeHead(404);
          res.end("Not Found");
          return;
        }
        const ext = path.extname(absPath).toLowerCase();
        res.writeHead(200, {
          "Content-Type": MIME_TYPES[ext] || "application/octet-stream",
          "Cache-Control": "no-store",
        });
        fs.createReadStream(absPath).pipe(res);
      });
    });
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      resolve({ server, port: addr.port });
    });
  });
}

function launchHeadlessChrome(chromeBin) {
  return new Promise((resolve, reject) => {
    const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "nve-web-e2e-chrome-"));
    const args = [
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
    ];
    const proc = spawn(chromeBin, args, {
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stderrBuf = "";
    let settled = false;
    const timeout = setTimeout(() => {
      if (!settled) {
        settled = true;
        try {
          proc.kill("SIGKILL");
        } catch {}
        reject(new Error(`Timed out waiting for Chrome DevTools endpoint.\nstderr:\n${stderrBuf}`));
      }
    }, 15000);

    proc.stderr.on("data", (chunk) => {
      stderrBuf += chunk.toString("utf8");
      const match = stderrBuf.match(/DevTools listening on (ws:\/\/127\.0\.0\.1:(\d+)\/devtools\/browser\/[^\s]+)/);
      if (match && !settled) {
        settled = true;
        clearTimeout(timeout);
        resolve({
          proc,
          userDataDir,
          browserWsUrl: match[1],
          debugPort: Number(match[2]),
        });
      }
    });

    proc.on("exit", (code, signal) => {
      if (!settled) {
        settled = true;
        clearTimeout(timeout);
        reject(
          new Error(
            `Chrome exited early (code=${code}, signal=${signal}).\nstderr:\n${stderrBuf}`
          )
        );
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
    if (typeof WebSocket !== "function") {
      throw new Error("Node global WebSocket is required (Node 22+).");
    }
    await new Promise((resolve, reject) => {
      const ws = new WebSocket(this.wsUrl);
      this.ws = ws;
      ws.addEventListener("open", () => resolve());
      ws.addEventListener("error", (ev) => reject(new Error(`CDP WebSocket error: ${ev.message || "unknown"}`)));
      ws.addEventListener("message", (ev) => {
        const msg = JSON.parse(String(ev.data));
        if (msg.id !== undefined) {
          const entry = this.pending.get(msg.id);
          if (entry) {
            this.pending.delete(msg.id);
            if (msg.error) {
              entry.reject(new Error(`CDP ${entry.method} failed: ${JSON.stringify(msg.error)}`));
            } else {
              entry.resolve(msg.result);
            }
          }
        } else if (msg.method) {
          const handlers = this.listeners.get(msg.method);
          if (handlers) {
            for (const fn of handlers) {
              fn(msg.params);
            }
          }
        }
      });
      ws.addEventListener("close", () => {
        for (const [, entry] of this.pending) {
          entry.reject(new Error(`CDP WebSocket closed while awaiting ${entry.method}`));
        }
        this.pending.clear();
      });
    });
  }

  on(method, handler) {
    if (!this.listeners.has(method)) {
      this.listeners.set(method, []);
    }
    this.listeners.get(method).push(handler);
  }

  send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      this.pending.set(id, { resolve, reject, method });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.close();
    }
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
      `Page JS evaluation threw: ${res.exceptionDetails.text} ${JSON.stringify(res.exceptionDetails.exception || {})}`
    );
  }
  return res.result ? res.result.value : undefined;
}

async function waitForCondition(cdp, description, fnSource, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const ok = await evalJs(cdp, `Boolean((${fnSource})())`);
      if (ok) {
        return;
      }
    } catch (err) {
      lastError = err;
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  const snapshot = await evalJs(
    cdp,
    `(() => ({ url: location.href, status: document.getElementById('nve-status')?.textContent, text: document.getElementById('app')?.innerText?.slice(0, 600) }))()`
  ).catch(() => null);
  throw new Error(
    `Timed out waiting for: ${description}\nLast error: ${lastError}\nDOM snapshot: ${JSON.stringify(snapshot, null, 2)}`
  );
}

async function clickElement(cdp, selectorOrFinder, description) {
  const rect = await evalJs(
    cdp,
    `(() => {
      const el = (${selectorOrFinder});
      if (!el) return null;
      el.scrollIntoView({ block: 'center', inline: 'center' });
      const r = el.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, h: r.height };
    })()`
  );
  if (!rect || rect.w <= 0 || rect.h <= 0) {
    throw new Error(`Clickable element not found or zero-sized: ${description} (${JSON.stringify(rect)})`);
  }
  await cdp.send("Input.dispatchMouseEvent", {
    type: "mouseMoved",
    x: rect.x,
    y: rect.y,
    button: "none",
  });
  await cdp.send("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x: rect.x,
    y: rect.y,
    button: "left",
    clickCount: 1,
  });
  await cdp.send("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: rect.x,
    y: rect.y,
    button: "left",
    clickCount: 1,
  });
}

function assertIncludes(haystack, needle, context) {
  if (!haystack.includes(needle)) {
    throw new Error(`Assertion failed (${context}): expected text to include ${JSON.stringify(needle)}.\nActual text:\n${haystack}`);
  }
}

async function getAppText(cdp) {
  return (await evalJs(cdp, `document.getElementById('app')?.innerText || ''`)) || "";
}

async function stopChrome(chrome) {
  if (!chrome) return;
  const { proc, userDataDir } = chrome;
  if (proc.exitCode === null && !proc.killed) {
    proc.kill("SIGTERM");
    await new Promise((resolve) => {
      const timer = setTimeout(() => {
        try {
          proc.kill("SIGKILL");
        } catch {}
        resolve();
      }, 3000);
      proc.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
  }
  try {
    fs.rmSync(userDataDir, { recursive: true, force: true });
  } catch {}
}

async function main() {
  const { screenshotPath, rebuild, devBuild } = parseArgs(process.argv.slice(2));
  ensureWasmBuilt(rebuild, devBuild);
  const chromeBin = findChromeBinary();

  const { server, port } = await startStaticServer(WEB_SHELL_DIR);
  const baseUrl = `http://127.0.0.1:${port}`;
  let chrome = null;
  let cdp = null;

  const cleanup = async () => {
    try {
      cdp?.close();
    } catch {}
    await stopChrome(chrome);
    await new Promise((resolve) => server.close(() => resolve()));
  };

  process.on("SIGINT", async () => {
    await cleanup();
    process.exit(130);
  });
  process.on("SIGTERM", async () => {
    await cleanup();
    process.exit(143);
  });

  try {
    console.log(`==> Serving ${WEB_SHELL_DIR} at ${baseUrl}`);
    console.log(`==> Launching headless Chrome: ${chromeBin}`);
    chrome = await launchHeadlessChrome(chromeBin);

    const targetsRes = await fetch(`http://127.0.0.1:${chrome.debugPort}/json/list`);
    const targets = await targetsRes.json();
    const pageTarget = targets.find((t) => t.type === "page");
    if (!pageTarget?.webSocketDebuggerUrl) {
      throw new Error(`No inspectable page target found: ${JSON.stringify(targets)}`);
    }

    cdp = new CdpClient(pageTarget.webSocketDebuggerUrl);
    await cdp.connect();

    const errors = [];
    const consoleLogs = [];

    cdp.on("Runtime.exceptionThrown", (params) => {
      const details = params.exceptionDetails || {};
      const msg = details.exception?.description || details.text || JSON.stringify(details);
      errors.push(`[exception] ${msg}`);
    });
    cdp.on("Runtime.consoleAPICalled", (params) => {
      const text = (params.args || [])
        .map((a) => (a.value !== undefined ? String(a.value) : a.description || a.type))
        .join(" ");
      consoleLogs.push(`[console.${params.type}] ${text}`);
      if (params.type === "error" || params.type === "assert") {
        errors.push(`[console.${params.type}] ${text}`);
      }
      if (text.includes("[NVE Web Panic]") || text.includes("panicked at")) {
        errors.push(`[wasm-panic] ${text}`);
      }
    });
    cdp.on("Log.entryAdded", (params) => {
      const entry = params.entry || {};
      if (entry.level === "error") {
        errors.push(`[log.error:${entry.source || "unknown"}] ${entry.text}`);
      }
    });

    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 840,
      deviceScaleFactor: 2,
      mobile: false,
    });

    // =========================================================================
    // 1. Aether Workspace + NVE Studio (`/?app=studio&ws=off`)
    // =========================================================================
    console.log("==> [1/2] Testing Aether Workspace + NVE Studio (?app=studio&ws=off)...");
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=studio&ws=off` });

    await waitForCondition(
      cdp,
      "StudioWorkspaceApp mounted with AETHER, Board, # engineering, NVE STUDIO",
      () => {
        const ctrl = window.__nveController;
        const text = document.getElementById("app")?.innerText || "";
        return (
          ctrl &&
          ctrl.app_name === "StudioWorkspaceApp" &&
          text.includes("AETHER") &&
          text.includes("Board") &&
          text.includes("# engineering") &&
          text.includes("NVE STUDIO") &&
          text.includes("AET-101")
        );
      }
    );

    const studioNodeCount = await evalJs(cdp, `window.__nveController.node_count()`);
    if (!(studioNodeCount > 20)) {
      throw new Error(`Expected Studio node_count > 20, got ${studioNodeCount}`);
    }

    const studioInitialText = await getAppText(cdp);
    for (const expected of [
      "AETHER",
      "Board",
      "# engineering",
      "NVE STUDIO",
      "Issue Board",
      "AET-101",
      "Zero-copy EventRing ABI",
      "Done (1)",
      "X-Ray HUD: OFF",
      "Snapshot 1 / 1",
    ]) {
      assertIncludes(studioInitialText, expected, "studio initial render");
    }

    if (screenshotPath) {
      const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
      fs.mkdirSync(path.dirname(screenshotPath), { recursive: true });
      fs.writeFileSync(screenshotPath, Buffer.from(shot.data, "base64"));
      console.log(`    Saved Studio screenshot: ${screenshotPath}`);
    }

    // 1a. Click task status pill on AET-101 -> advances In Progress -> Done
    await clickElement(
      cdp,
      `document.querySelector('[testid="task-status-1"]')`,
      "task-status-1"
    );
    await waitForCondition(
      cdp,
      "AET-101 status advanced to Done (2)",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("Done (2)") && text.includes("Status -> Done") && text.includes("Snapshot 2 / 2");
      }
    );

    // 1b. Click `# engineering` nav tab -> switches channel view
    await clickElement(
      cdp,
      `document.querySelector('[testid="nav-engineering"]')`,
      "nav-engineering"
    );
    await waitForCondition(
      cdp,
      "# engineering channel view rendered",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return (
          text.includes("Single ABI drives all 4 hosts!") &&
          text.includes("QUICK REPLY") &&
          text.includes("Snapshot 3 / 3")
        );
      }
    );

    // 1c. Click X-Ray toggle -> enables X-Ray HUD
    await clickElement(
      cdp,
      `document.querySelector('[testid="toggle-xray"]')`,
      "toggle-xray"
    );
    await waitForCondition(
      cdp,
      "X-Ray HUD toggled ON",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("X-Ray HUD: ON") && text.includes("Snapshot 4 / 4");
      }
    );

    // 1d. Click Undo -> time-travels back to Snapshot 3 / 4 with X-Ray HUD: OFF
    await clickElement(cdp, `document.querySelector('[testid="tt-undo"]')`, "tt-undo");
    await waitForCondition(
      cdp,
      "Time-travel Undo restored Snapshot 3 / 4 and X-Ray HUD: OFF",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("X-Ray HUD: OFF") && text.includes("Snapshot 3 / 4");
      }
    );

    // 1e. Click `<< #1` -> time-travels back to initial Issue Board snapshot
    await clickElement(cdp, `document.querySelector('[testid="tt-first"]')`, "tt-first");
    await waitForCondition(
      cdp,
      "Time-travel << #1 restored Issue Board Snapshot 1 / 4",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return (
          text.includes("Issue Board") &&
          text.includes("Done (1)") &&
          text.includes("Snapshot 1 / 4")
        );
      }
    );

    console.log(
      `    PASS Studio Workspace (nodes=${studioNodeCount}, status cycle, # engineering, X-Ray, Undo, First)`
    );

    // =========================================================================
    // 2. Counter App (`/?app=counter&ws=off`)
    // =========================================================================
    console.log("==> [2/2] Testing Counter App (?app=counter&ws=off)...");
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=counter&ws=off` });

    await waitForCondition(
      cdp,
      "CounterApp mounted with Count: 0",
      () => {
        const ctrl = window.__nveController;
        const text = document.getElementById("app")?.innerText || "";
        return (
          ctrl &&
          ctrl.app_name === "CounterApp" &&
          text.includes("NATIVE VIEW ENGINE") &&
          text.includes("Count: 0") &&
          text.includes("Increment +") &&
          text.includes("Reset")
        );
      }
    );

    const findButtonByText = (label) =>
      `Array.from(document.querySelectorAll('#app button')).find((b) => b.innerText.trim() === ${JSON.stringify(label)})`;

    // Click Increment + twice -> Count: 1 -> Count: 2
    await clickElement(cdp, findButtonByText("Increment +"), "Increment + (1st click)");
    await waitForCondition(
      cdp,
      "Counter incremented to Count: 1",
      () => (document.getElementById("app")?.innerText || "").includes("Count: 1")
    );

    await clickElement(cdp, findButtonByText("Increment +"), "Increment + (2nd click)");
    await waitForCondition(
      cdp,
      "Counter incremented to Count: 2",
      () => (document.getElementById("app")?.innerText || "").includes("Count: 2")
    );

    // Click Reset -> Count: 0
    await clickElement(cdp, findButtonByText("Reset"), "Reset");
    await waitForCondition(
      cdp,
      "Counter reset to Count: 0",
      () => (document.getElementById("app")?.innerText || "").includes("Count: 0")
    );

    console.log("    PASS Counter App (Increment x2 -> Count: 2 -> Reset -> Count: 0)");

    if (errors.length > 0) {
      throw new Error(
        `Browser console/runtime errors detected (${errors.length}):\n${errors.join("\n")}`
      );
    }

    console.log(`==> ALL WEB WASM E2E CHECKS PASSED (0 console errors, 0 WASM panics)`);
  } finally {
    await cleanup();
  }
}

main().catch((err) => {
  console.error("E2E FAILED:", err?.stack || err);
  process.exit(1);
});
