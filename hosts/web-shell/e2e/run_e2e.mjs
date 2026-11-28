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
const REPO_ROOT = path.resolve(WEB_SHELL_DIR, "..", "..");
const PKG_DIR = path.join(WEB_SHELL_DIR, "pkg");
// hosts/web-shell/build.sh bundles: one production bundle per app + the dev bundle (pairing, probes).
const BUNDLES = ["studio", "kitchen-sink", "dev"];

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

/** The entry files `build.sh` published for `bundle` (`pkg/<bundle>/<hash>/...`), or null. */
function bundleFiles(bundle) {
  const dir = path.join(PKG_DIR, bundle);
  try {
    const manifest = JSON.parse(fs.readFileSync(path.join(dir, "manifest.json"), "utf8"));
    const files = { js: path.join(dir, manifest.js), wasm: path.join(dir, manifest.wasm) };
    return fs.existsSync(files.js) && fs.existsSync(files.wasm) ? files : null;
  } catch {
    return null;
  }
}

/**
 * Every bundle (studio and counter included, which carry no test hook) installs the console panic
 * hook at instantiation: the wasm exports the `#[wasm_bindgen(start)]` entry and carries the hook's
 * `[NVE Web Panic]` prefix.
 */
function assertPanicHookInEveryBundle() {
  for (const bundle of BUNDLES) {
    const bytes = fs.readFileSync(bundleFiles(bundle).wasm);
    const exports = WebAssembly.Module.exports(new WebAssembly.Module(bytes)).map((e) => e.name);
    if (!exports.includes("__wbindgen_start") || !bytes.includes("[NVE Web Panic]")) {
      throw new Error(`pkg/${bundle} does not install the panic hook at start (exports: ${exports.length})`);
    }
  }
}

/** 1-based line of the first line in `file` containing `needle`. */
function lineOf(file, needle) {
  const idx = fs.readFileSync(file, "utf8").split("\n").findIndex((l) => l.includes(needle));
  if (idx < 0) throw new Error(`${needle} not found in ${file}`);
  return idx + 1;
}

function ensureWasmBuilt(rebuild, devBuild) {
  const built = BUNDLES.every((bundle) => bundleFiles(bundle) !== null);
  if (!rebuild && built) {
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
    `(() => ({
      url: location.href,
      innerWidth: window.innerWidth,
      innerHeight: window.innerHeight,
      appRect: document.getElementById('app')?.getBoundingClientRect()?.toJSON(),
      viewportMode: document.querySelector('[data-testid="workspace-root"]')?.getAttribute('data-viewport-mode'),
      hasMobileTopBar: Boolean(document.querySelector('[data-testid="mobile-top-bar"]')),
      hasDesktopSidebar: Boolean(document.querySelector('[data-testid="desktop-sidebar"]')),
      status: document.getElementById('nve-status')?.textContent,
      text: document.getElementById('app')?.innerText?.slice(0, 400),
    }))()`
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

async function typeIntoInput(cdp, selector, text, description) {
  await clickElement(cdp, selector, description);
  // Select any existing text and replace via real CDP input events so browser focus/input listeners fire.
  await evalJs(
    cdp,
    `(() => {
      const el = (${selector});
      if (!el) throw new Error("Input not found: " + ${JSON.stringify(description)});
      el.focus();
      el.select?.();
      return document.activeElement === el;
    })()`
  );
  await cdp.send("Input.insertText", { text });
}

async function pressKey(cdp, { key, code, windowsVirtualKeyCode = 0, modifiers = 0, text = "" }) {
  const downParams = {
    type: "keyDown",
    key,
    code,
    windowsVirtualKeyCode,
    modifiers,
  };
  if (text) {
    downParams.text = text;
    downParams.unmodifiedText = text;
  }
  await cdp.send("Input.dispatchKeyEvent", downParams);
  await cdp.send("Input.dispatchKeyEvent", {
    type: "keyUp",
    key,
    code,
    windowsVirtualKeyCode,
    modifiers,
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

function resolveTargetDir() {
  const envDir = process.env.CARGO_TARGET_DIR;
  if (!envDir) {
    return path.join(REPO_ROOT, "target");
  }
  return path.isAbsolute(envDir) ? envDir : path.resolve(REPO_ROOT, envDir);
}

function resolvePairArtifacts() {
  const targetDir = resolveTargetDir();
  const exeExt = process.platform === "win32" ? ".exe" : "";
  const libPrefix = process.platform === "win32" ? "" : "lib";
  const libExt =
    process.platform === "darwin" ? "dylib" : process.platform === "win32" ? "dll" : "so";

  const nveBin = path.join(targetDir, "debug", `nve${exeExt}`);
  const kitchenSinkDylib = path.join(targetDir, "debug", `${libPrefix}nve_example_kitchen_sink.${libExt}`);
  return { nveBin, kitchenSinkDylib };
}

function ensurePairArtifacts() {
  const { nveBin, kitchenSinkDylib } = resolvePairArtifacts();
  console.log("==> Building nve-cli + nve-example-kitchen-sink for live pair E2E...");
  const res = spawnSync("cargo", ["build", "-p", "nve-example-kitchen-sink", "-p", "nve-cli"], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });
  if (res.status !== 0) {
    throw new Error(`cargo build -p nve-example-kitchen-sink -p nve-cli failed with exit code ${res.status}`);
  }
  if (!fs.existsSync(nveBin) || !fs.existsSync(kitchenSinkDylib)) {
    throw new Error(
      `Expected live pair artifacts after build:\n  nveBin=${nveBin} (exists=${fs.existsSync(nveBin)})\n  kitchenSinkDylib=${kitchenSinkDylib} (exists=${fs.existsSync(kitchenSinkDylib)})`
    );
  }
  return { nveBin, kitchenSinkDylib };
}

async function stopChildProcess(proc) {
  if (!proc) return;
  if (proc.exitCode === null && !proc.killed) {
    try {
      proc.kill("SIGTERM");
    } catch {}
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
}

function launchPairServer(nveBin, dylibPath) {
  return new Promise((resolve, reject) => {
    const proc = spawn(
      nveBin,
      [
        "pair",
        "--dylib",
        dylibPath,
        "--host",
        "127.0.0.1",
        "--udp-port",
        "0",
        "--ws-port",
        "0",
      ],
      {
        cwd: REPO_ROOT,
        stdio: ["ignore", "pipe", "pipe"],
      }
    );

    let stdoutBuf = "";
    let stderrBuf = "";
    let settled = false;

    const fail = (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      try {
        proc.kill("SIGKILL");
      } catch {}
      reject(err);
    };

    const timeout = setTimeout(() => {
      fail(
        new Error(
          `Timed out waiting for NVE_PAIR_READY from nve pair.\nstdout:\n${stdoutBuf}\nstderr:\n${stderrBuf}`
        )
      );
    }, 15000);

    proc.stdout.on("data", (chunk) => {
      stdoutBuf += chunk.toString("utf8");
      const match = stdoutBuf.match(
        /^NVE_PAIR_READY\s+ws=(\S+)\s+token=(\S+)\s+udp=(\S+)\s+tcp=(\S+)(?:\s+cert_hash=(\S+))?\s*$/m
      );
      if (match && !settled) {
        const [, wsUrl, token, udpAddr, tcpAddr] = match;
        const wsPort = Number(new URL(wsUrl).port);
        const udpPort = Number(udpAddr.split(":").pop());
        const tcpPort = Number(tcpAddr.split(":").pop());
        if (!(wsPort > 0 && udpPort > 0 && tcpPort === wsPort)) {
          fail(
            new Error(
              `Invalid resolved ports in NVE_PAIR_READY: ws=${wsUrl} udp=${udpAddr} tcp=${tcpAddr}`
            )
          );
          return;
        }
        settled = true;
        clearTimeout(timeout);
        resolve({
          proc,
          wsUrl,
          token,
          udpAddr,
          tcpAddr,
          wsPort,
          udpPort,
          stdout: stdoutBuf,
        });
      }
    });

    proc.stderr.on("data", (chunk) => {
      stderrBuf += chunk.toString("utf8");
    });

    proc.on("error", (err) => {
      fail(new Error(`Failed to spawn nve pair (${nveBin}): ${err.message}`));
    });

    proc.on("exit", (code, signal) => {
      if (!settled) {
        fail(
          new Error(
            `nve pair exited before NVE_PAIR_READY (code=${code}, signal=${signal}).\nstdout:\n${stdoutBuf}\nstderr:\n${stderrBuf}`
          )
        );
      }
    });
  });
}

function assertNonCollapsedAndNonOverlapping(items, minW, minH, context) {
  if (!Array.isArray(items) || items.length === 0) {
    throw new Error(`Geometry assertion failed (${context}): empty item list`);
  }
  for (const item of items) {
    if (!(item.w >= minW && item.h >= minH)) {
      throw new Error(
        `Collapsed geometry (${context}): ${item.id} has ${item.w}x${item.h} (min ${minW}x${minH})`
      );
    }
  }
  for (let i = 0; i < items.length; i++) {
    for (let j = i + 1; j < items.length; j++) {
      const a = items[i];
      const b = items[j];
      const separated =
        a.right <= b.left + 1 ||
        b.right <= a.left + 1 ||
        a.bottom <= b.top + 1 ||
        b.bottom <= a.top + 1;
      if (!separated) {
        throw new Error(
          `Overlapping geometry (${context}): ${a.id} ${JSON.stringify(a)} overlaps ${b.id} ${JSON.stringify(b)}`
        );
      }
    }
  }
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
  assertPanicHookInEveryBundle();
  const chromeBin = findChromeBinary();

  const { server, port } = await startStaticServer(WEB_SHELL_DIR);
  const baseUrl = `http://127.0.0.1:${port}`;
  let chrome = null;
  let cdp = null;
  let pairServer = null;

  const cleanup = async () => {
    try {
      cdp?.close();
    } catch {}
    await stopChildProcess(pairServer?.proc);
    pairServer = null;
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
    let allowExpectedOfflineWasmError = false;

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
        if (allowExpectedOfflineWasmError && text.includes("[NVE Web] Boot failed:")) {
          return;
        }
        errors.push(`[console.${params.type}] ${text}`);
      }
      if (text.includes("[NVE Web Panic]") || text.includes("panicked at")) {
        errors.push(`[wasm-panic] ${text}`);
      }
    });
    cdp.on("Log.entryAdded", (params) => {
      const entry = params.entry || {};
      if (entry.level === "error") {
        if (
          allowExpectedOfflineWasmError &&
          (String(entry.url || "").endsWith(".wasm") ||
            String(entry.text || "").includes("ERR_BLOCKED_BY_CLIENT") ||
            String(entry.text || "").includes("Failed to load resource"))
        ) {
          return;
        }
        errors.push(`[log.error:${entry.source || "unknown"}] ${entry.text}`);
      }
    });

    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 800,
      deviceScaleFactor: 2,
      mobile: false,
    });

    // =========================================================================
    // 1. Aether Workspace + NVE Studio Desktop 1280x800 (`/?app=studio&ws=off`)
    // =========================================================================
    console.log("==> [1/7] Testing Aether Workspace + NVE Studio Desktop 1280x800 (?app=studio&ws=off)...");
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=studio&ws=off` });
    await waitForCondition(
      cdp,
      "production studio bundle loaded (studio exports only: no counter or dev probes)",
      () => {
        const m = window.__nveWasmMod;
        return (
          window.__nveBundle === "studio" &&
          typeof m?.start_studio_workspace_app === "function" &&
          m.start_kitchen_sink_app === undefined &&
          m.run_semantic_style_hydration_probe === undefined &&
          m.run_hostile_frame_probe === undefined &&
          m.panic_probe === undefined
        );
      }
    );

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

    // 1a. Verify Image avatar `<img>` loaded with `naturalWidth > 0`, `object-fit: cover`, and `alt`.
    await waitForCondition(
      cdp,
      "Image avatar <img> loaded with naturalWidth > 0 and object-fit: cover",
      () => {
        const img = document.querySelector('img[data-testid="task-avatar-1"]');
        if (!img) return false;
        const style = getComputedStyle(img);
        return img.complete && img.naturalWidth > 0 && style.objectFit === "cover" && Boolean(img.alt);
      }
    );
    const avatarInfo = await evalJs(
      cdp,
      `(() => {
        const img = document.querySelector('img[data-testid="task-avatar-1"]');
        return { naturalWidth: img.naturalWidth, naturalHeight: img.naturalHeight, objectFit: getComputedStyle(img).objectFit, alt: img.alt };
      })()`
    );
    console.log(`    PASS Image avatar <img> (${avatarInfo.naturalWidth}x${avatarInfo.naturalHeight}, object-fit=${avatarInfo.objectFit}, alt=${avatarInfo.alt})`);

    // 1b. Board TextInput live search filter + clear
    await typeIntoInput(
      cdp,
      `document.querySelector('[data-testid="board-search-input"]')`,
      "AppKit",
      "board-search-input"
    );
    await waitForCondition(
      cdp,
      "Board search filtered to AET-102 and excluded AET-101",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("AET-102") && !text.includes("AET-101");
      }
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="board-search-clear"]')`,
      "board-search-clear"
    );
    await waitForCondition(
      cdp,
      "Board search cleared and AET-101 restored",
      () => (document.getElementById("app")?.innerText || "").includes("AET-101")
    );

    // 1c. Board TextInput composer + Enter submit creates AET-105
    await typeIntoInput(
      cdp,
      `document.querySelector('[data-testid="board-new-task-input"]')`,
      "Ship Web Canvas E2E",
      "board-new-task-input"
    );
    await pressKey(cdp, { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await waitForCondition(
      cdp,
      "Board composer Enter submit created custom task AET-105",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("AET-105") && text.includes("Ship Web Canvas E2E");
      }
    );

    // 1d. Click task status pill on AET-101 -> advances In Progress -> Done
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="task-status-1"]')`,
      "task-status-1"
    );
    await waitForCondition(
      cdp,
      "AET-101 status advanced to Done (2)",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("Done (2)") && text.includes("Status -> Done");
      }
    );

    // 1e. Real CDP Cmd+K / Ctrl+K palette open -> filter -> ArrowDown -> Enter -> Escape
    await pressKey(cdp, { key: "k", code: "KeyK", windowsVirtualKeyCode: 75, modifiers: 4 });
    await waitForCondition(
      cdp,
      "Cmd+K opened command palette via real CDP keyboard event",
      () => Boolean(document.querySelector('[data-testid="command-palette"]'))
    );
    await typeIntoInput(
      cdp,
      `document.querySelector('[data-testid="palette-input"]')`,
      "Accent:",
      "palette-input"
    );
    await waitForCondition(
      cdp,
      "Command palette filtered to Accent options",
      () => (document.querySelector('[data-testid="command-palette"]')?.innerText || "").includes("Violet")
    );
    await pressKey(cdp, { key: "ArrowDown", code: "ArrowDown", windowsVirtualKeyCode: 40 });
    await pressKey(cdp, { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await waitForCondition(
      cdp,
      "Command palette ArrowDown + Enter applied Accent -> Violet and closed",
      () => {
        const pal = document.querySelector('[data-testid="command-palette"]');
        const text = document.getElementById("app")?.innerText || "";
        return !pal && text.includes("Accent -> Violet");
      }
    );
    // Re-open with Ctrl+K and close with Escape
    await pressKey(cdp, { key: "k", code: "KeyK", windowsVirtualKeyCode: 75, modifiers: 2 });
    await waitForCondition(
      cdp,
      "Ctrl+K re-opened command palette",
      () => Boolean(document.querySelector('[data-testid="command-palette"]'))
    );
    await pressKey(cdp, { key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await waitForCondition(
      cdp,
      "Escape closed command palette",
      () => !document.querySelector('[data-testid="command-palette"]')
    );
    console.log("    PASS Cmd+K / Ctrl+K palette (open, filter, ArrowDown, Enter, Escape)");

    // 1f. `# engineering` channel view + chat composer TextInput Enter submit
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="nav-engineering"]')`,
      "nav-engineering"
    );
    await waitForCondition(
      cdp,
      "# engineering channel view rendered",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("Single ABI drives all 4 hosts!") && text.includes("QUICK REPLY");
      }
    );
    await typeIntoInput(
      cdp,
      `document.querySelector('[data-testid="chat-composer-input"]')`,
      "Live from Web WASM!",
      "chat-composer-input"
    );
    await pressKey(cdp, { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await waitForCondition(
      cdp,
      "Chat composer Enter submit posted message",
      () => (document.getElementById("app")?.innerText || "").includes("Live from Web WASM!")
    );

    // 1g. Islands view `<Island>` bus broadcast & gesture arbitration
    await clickElement(cdp, `document.querySelector('[data-testid="nav-islands"]')`, "nav-islands");
    await waitForCondition(
      cdp,
      "Islands view rendered",
      () => Boolean(document.querySelector('[data-testid="islands-summary"]'))
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="island-bus-theme"]')`,
      "island-bus-theme"
    );
    await waitForCondition(
      cdp,
      "Islands NveChannelBus theme broadcast recorded",
      () => (document.querySelector('[data-testid="islands-summary"]')?.innerText || "").includes("Bus: 1 tx")
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="island-gesture-claim"]')`,
      "island-gesture-claim"
    );
    await waitForCondition(
      cdp,
      "Islands GestureArbitrator Claim recorded",
      () => (document.querySelector('[data-testid="islands-gesture-readout"]')?.innerText || "").includes("Claim | winner:")
    );
    console.log("    PASS Islands view (NveChannelBus broadcast + GestureArbitrator Claim)");

    // 1h. 100k Stream view jump #50k and #99,999
    await clickElement(cdp, `document.querySelector('[data-testid="nav-stream"]')`, "nav-stream");
    await waitForCondition(
      cdp,
      "100k Stream view rendered",
      () => Boolean(document.querySelector('[data-testid="stream-viewport"]'))
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="stream-jump-50k"]')`,
      "stream-jump-50k"
    );
    await waitForCondition(
      cdp,
      "100k Stream jumped to #50k (STR-50000)",
      () => (document.getElementById("app")?.innerText || "").includes("STR-50000")
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="stream-jump-end"]')`,
      "stream-jump-end"
    );
    await waitForCondition(
      cdp,
      "100k Stream jumped to #99,999 (STR-99999)",
      () => (document.getElementById("app")?.innerText || "").includes("STR-99999")
    );
    console.log("    PASS 100k Stream view (#50k -> STR-50000, #99,999 -> STR-99999)");

    // 1i. Metrics view `<canvas>` NVE_DRAW1 non-blank painted pixel verification
    await clickElement(cdp, `document.querySelector('[data-testid="nav-metrics"]')`, "nav-metrics");
    await waitForCondition(
      cdp,
      "Metrics <canvas> rendered with NVE_DRAW1 draw ops",
      () => {
        const c = document.querySelector('canvas[data-testid="metrics-sparkline-canvas"]');
        return Boolean(c && c.width > 0 && c.height > 0 && Number(c.getAttribute("data-nve-draw-ops")) > 0);
      }
    );
    const canvasCheck = await evalJs(
      cdp,
      `(() => {
        const c = document.querySelector('canvas[data-testid="metrics-sparkline-canvas"]');
        const ctx = c.getContext('2d');
        const img = ctx.getImageData(0, 0, c.width, c.height).data;
        let nonTransparent = 0;
        const colors = new Set();
        for (let i = 0; i < img.length; i += 4) {
          if (img[i + 3] > 0) {
            nonTransparent++;
            if (colors.size < 32) {
              colors.add((img[i] << 16) | (img[i + 1] << 8) | img[i + 2]);
            }
          }
        }
        return {
          width: c.width,
          height: c.height,
          drawOps: Number(c.getAttribute('data-nve-draw-ops') || 0),
          nonTransparent,
          distinctColors: colors.size,
        };
      })()`
    );
    if (!(canvasCheck.nonTransparent > 100 && canvasCheck.distinctColors >= 2)) {
      throw new Error(
        `Metrics <canvas> did not paint non-blank vector pixels: ${JSON.stringify(canvasCheck)}`
      );
    }
    console.log(
      `    PASS Metrics <canvas> NVE_DRAW1 (${canvasCheck.width}x${canvasCheck.height}px, ops=${canvasCheck.drawOps}, painted=${canvasCheck.nonTransparent}px, colors=${canvasCheck.distinctColors})`
    );

    // 1j. X-Ray toggle + deterministic time-travel Undo / First
    await clickElement(cdp, `document.querySelector('[data-testid="toggle-xray"]')`, "toggle-xray");
    await waitForCondition(
      cdp,
      "X-Ray HUD toggled ON",
      () => (document.getElementById("app")?.innerText || "").includes("X-Ray HUD: ON")
    );
    await clickElement(cdp, `document.querySelector('[data-testid="tt-undo"]')`, "tt-undo");
    await waitForCondition(
      cdp,
      "Time-travel Undo restored X-Ray HUD: OFF",
      () => (document.getElementById("app")?.innerText || "").includes("X-Ray HUD: OFF")
    );
    await clickElement(cdp, `document.querySelector('[data-testid="tt-first"]')`, "tt-first");
    await waitForCondition(
      cdp,
      "Time-travel << #1 restored Issue Board",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("Issue Board") && text.includes("Done (1)");
      }
    );

    console.log(
      `    PASS Studio Workspace Desktop (nodes=${studioNodeCount}, Board, Images, TextInput, Cmd+K, Islands, 100k Stream, Metrics Canvas, Time-Travel)`
    );

    // =========================================================================
    // 2. Responsive Mobile Portrait Resize (`393x852`, < 620pt breakpoint)
    // =========================================================================
    console.log("==> [2/7] Testing Responsive Mobile Portrait Resize (393x852 -> 1280x800)...");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 393,
      height: 852,
      deviceScaleFactor: 3,
      mobile: true,
    });
    await waitForCondition(
      cdp,
      "Studio Workspace dynamically collapsed to mobile layout (mobile-top-bar visible)",
      () => {
        const topBar = document.querySelector('[data-testid="mobile-top-bar"]');
        const desktopSidebar = document.querySelector('[data-testid="desktop-sidebar"]');
        const center = document.querySelector('[data-testid="center-workspace"]');
        if (!topBar || desktopSidebar || !center) return false;
        const tbRect = topBar.getBoundingClientRect();
        const cwRect = center.getBoundingClientRect();
        return tbRect.width > 0 && tbRect.height > 0 && cwRect.width > 300 && cwRect.left >= 0 && cwRect.right <= 395;
      }
    );
    const mobileMetrics = await evalJs(
      cdp,
      `(() => {
        const tb = document.querySelector('[data-testid="mobile-top-bar"]').getBoundingClientRect();
        const cw = document.querySelector('[data-testid="center-workspace"]').getBoundingClientRect();
        return {
          innerWidth: window.innerWidth,
          topBar: { w: Math.round(tb.width), h: Math.round(tb.height) },
          center: { l: Math.round(cw.left), r: Math.round(cw.right), w: Math.round(cw.width), h: Math.round(cw.height) },
        };
      })()`
    );
    if (!(mobileMetrics.center.w > 300 && mobileMetrics.center.l >= 0 && mobileMetrics.center.r <= 395)) {
      throw new Error(`Mobile center-workspace clipped or negative width: ${JSON.stringify(mobileMetrics)}`);
    }

    // Resize back to desktop 1280x800 and verify 3-column layout restores cleanly.
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 800,
      deviceScaleFactor: 2,
      mobile: false,
    });
    await waitForCondition(
      cdp,
      "Studio Workspace restored 3-column desktop layout after resize back to 1280x800",
      () =>
        Boolean(document.querySelector('[data-testid="desktop-sidebar"]')) &&
        !document.querySelector('[data-testid="mobile-top-bar"]')
    );
    console.log(
      `    PASS Mobile Portrait Resize (393x852 mobile-top-bar=${mobileMetrics.topBar.w}x${mobileMetrics.topBar.h}, center=${mobileMetrics.center.w}x${mobileMetrics.center.h}, restored desktop)`
    );

    // =========================================================================
    // 3. Kitchen Sink Gallery (`/?app=kitchen-sink&ws=off`)
    // =========================================================================
    console.log("==> [3/7] Testing Kitchen Sink Gallery (?app=kitchen-sink&ws=off)...");
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=kitchen-sink&ws=off&reset=1` });
    await waitForCondition(
      cdp,
      "production kitchen-sink bundle loaded (kitchen-sink exports only: no studio or dev probes)",
      () => {
        const m = window.__nveWasmMod;
        return (
          window.__nveBundle === "kitchen-sink" &&
          typeof m?.start_kitchen_sink_app === "function" &&
          m.start_studio_workspace_app === undefined &&
          m.run_semantic_style_hydration_probe === undefined &&
          m.run_hostile_frame_probe === undefined &&
          m.panic_probe === undefined
        );
      }
    );

    await waitForCondition(
      cdp,
      "KitchenSinkApp mounted with Overview & Primitives and Count: 0",
      () => {
        const ctrl = window.__nveController;
        const text = document.getElementById("app")?.innerText || "";
        return (
          ctrl &&
          ctrl.app_name === "KitchenSinkApp" &&
          text.includes("NVE Kitchen Sink") &&
          text.includes("Overview & Primitives") &&
          text.includes("Count: 0") &&
          text.includes("Increment +") &&
          text.includes("Reset")
        );
      }
    );

    const findButtonByText = (label) =>
      `Array.from(document.querySelectorAll('#app button')).find((b) => b.innerText.trim() === ${JSON.stringify(label)})`;
    const findByTestId = (tid) => `document.querySelector('#app [data-testid="${tid}"]')`;

    // Click Increment + twice -> Count: 1 -> Count: 2
    await clickElement(cdp, findByTestId("ks-counter-inc"), "Kitchen Sink Increment + (1st click)");
    await waitForCondition(
      cdp,
      "Kitchen Sink counter incremented to Count: 1",
      () => (document.getElementById("app")?.innerText || "").includes("Count: 1")
    );

    await clickElement(cdp, findByTestId("ks-counter-inc"), "Kitchen Sink Increment + (2nd click)");
    await waitForCondition(
      cdp,
      "Kitchen Sink counter incremented to Count: 2",
      () => (document.getElementById("app")?.innerText || "").includes("Count: 2")
    );

    // Click Reset -> Count: 0
    await clickElement(cdp, findByTestId("ks-counter-reset"), "Kitchen Sink Reset");
    await waitForCondition(
      cdp,
      "Kitchen Sink counter reset to Count: 0",
      () => (document.getElementById("app")?.innerText || "").includes("Count: 0")
    );

    // Navigate to Overlays & fire a Success toast
    await clickElement(cdp, findByTestId("ks-nav-overlays"), "Navigate to Overlays section");
    await waitForCondition(
      cdp,
      "Overlays section mounted",
      () => (document.getElementById("app")?.innerText || "").includes("ToastProvider")
    );
    await clickElement(cdp, findByTestId("ks-toast-success"), "Fire Success Toast");
    await waitForCondition(
      cdp,
      "Success toast rendered in ToastViewport",
      () => (document.getElementById("app")?.innerText || "").includes("Build Verified")
    );

    // Navigate to Motion & Resilience -> Trigger ErrorBoundary -> Verify Fallback -> Retry recovery!
    await clickElement(cdp, findByTestId("ks-nav-motion"), "Navigate to Motion & Resilience section");
    await waitForCondition(
      cdp,
      "Motion & Resilience section mounted",
      () => (document.getElementById("app")?.innerText || "").includes("UiErrorBoundary Subtree Isolation")
    );
    await clickElement(cdp, findByTestId("ks-trigger-error"), "Trigger Subtree Error");
    await waitForCondition(
      cdp,
      "UiErrorBoundary caught subtree error without crashing gallery",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return (
          text.includes("Subtree Error Caught") &&
          text.includes("Origin:") &&
          text.includes("NVE Kitchen Sink")
        );
      }
    );
    await clickElement(cdp, findByTestId("ks-error-boundary-retry"), "Retry ErrorBoundary");
    await waitForCondition(
      cdp,
      "UiErrorBoundary recovered cleanly on Retry",
      () => (document.getElementById("app")?.innerText || "").includes("Protected subtree is mounted and operational.")
    );

    console.log("    PASS Kitchen Sink Gallery (Counter, Overlays/Toast, Motion/UiErrorBoundary recovery)");

    // =========================================================================
    // 4. Studio Docs View (`ActiveView::Docs`)
    // =========================================================================
    console.log("==> [4/7] Testing Studio Docs View (ActiveView::Docs)...");
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=studio&ws=off` });
    await waitForCondition(
      cdp,
      "StudioWorkspaceApp remounted with nav-docs and nav-workbench",
      () =>
        window.__nveController?.app_name === "StudioWorkspaceApp" &&
        Boolean(document.querySelector('[data-testid="nav-docs"]')) &&
        Boolean(document.querySelector('[data-testid="nav-workbench"]'))
    );

    await clickElement(cdp, `document.querySelector('[data-testid="nav-docs"]')`, "nav-docs");
    await waitForCondition(
      cdp,
      "Docs view rendered seeded RFC-042 and Release Checklist docs",
      () => {
        const hdr = document.querySelector('[data-testid="docs-header"]')?.innerText || "";
        const list = document.querySelector('[data-testid="docs-blocks-list"]')?.innerText || "";
        return (
          Boolean(document.querySelector('[data-testid="doc-tab-1"]')) &&
          Boolean(document.querySelector('[data-testid="doc-tab-2"]')) &&
          hdr.includes("7 blocks | 1/2 checks") &&
          list.includes("RFC-042: Zero-Copy NveSceneView") &&
          list.includes("Enable crash-safe Frame-0 NVE2 + StateFile local persistence in Studio")
        );
      }
    );

    // Switch to second seeded doc and back to RFC-042.
    await clickElement(cdp, `document.querySelector('[data-testid="doc-tab-2"]')`, "doc-tab-2");
    await waitForCondition(
      cdp,
      "Switched to Release Checklist: v1.0 seeded doc",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("Pre-Flight Gates") && text.includes("8 blocks | 2/3 checks");
      }
    );
    await clickElement(cdp, `document.querySelector('[data-testid="doc-tab-1"]')`, "doc-tab-1");
    await waitForCondition(
      cdp,
      "Switched back to RFC-042 seeded doc",
      () => (document.querySelector('[data-testid="docs-header"]')?.innerText || "").includes("7 blocks | 1/2 checks")
    );

    // Add a new block via the Docs composer.
    const addedBlockDraft = "Stream binary NveWireFrame deltas over WebSocket";
    await typeIntoInput(
      cdp,
      `document.querySelector('[data-testid="doc-new-block-input"]')`,
      addedBlockDraft,
      "doc-new-block-input"
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="doc-new-block-submit"]')`,
      "doc-new-block-submit"
    );
    await waitForCondition(
      cdp,
      "New block #16 appended to RFC-042 doc",
      () => {
        const hdr = document.querySelector('[data-testid="docs-header"]')?.innerText || "";
        const blk = document.querySelector('[data-testid="doc-block-16"]')?.innerText || "";
        return hdr.includes("8 blocks | 1/2 checks") && blk.includes("Stream binary NveWireFrame deltas over WebSocket");
      }
    );

    // Edit block #16 inline text and save.
    const editedBlockText = "Verified binary NveWireFrame WebSocket streaming";
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="doc-block-edit-16"]')`,
      "doc-block-edit-16"
    );
    await waitForCondition(
      cdp,
      "Inline editor opened for block #16",
      () => Boolean(document.querySelector('[data-testid="doc-block-input-16"]'))
    );
    await typeIntoInput(
      cdp,
      `document.querySelector('[data-testid="doc-block-input-16"]')`,
      editedBlockText,
      "doc-block-input-16"
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="doc-block-save-16"]')`,
      "doc-block-save-16"
    );
    await waitForCondition(
      cdp,
      "Edited block #16 saved and inline input closed",
      () =>
        !document.querySelector('[data-testid="doc-block-input-16"]') &&
        (document.querySelector('[data-testid="doc-block-16"]')?.innerText || "").includes(
          "Verified binary NveWireFrame WebSocket streaming"
        )
    );

    // Toggle unchecked checklist item #7 -> [x] (2/2 checks).
    await clickElement(cdp, `document.querySelector('[data-testid="doc-check-7"]')`, "doc-check-7");
    await waitForCondition(
      cdp,
      "Checklist item #7 toggled to [x] (2/2 checks)",
      () => {
        const hdr = document.querySelector('[data-testid="docs-header"]')?.innerText || "";
        const chk = document.querySelector('[data-testid="doc-check-7"]')?.innerText || "";
        return chk.includes("[x]") && hdr.includes("2/2 checks");
      }
    );

    // Reorder block #16 up above block #7 and verify DOM order changed.
    const orderBefore = await evalJs(
      cdp,
      `Array.from(document.querySelectorAll('[data-testid="docs-blocks-list"] [data-testid^="doc-block-"]'))
        .map((el) => el.getAttribute('data-testid'))
        .filter((id) => /^doc-block-\\d+$/.test(id))`
    );
    if (!(orderBefore.indexOf("doc-block-7") >= 0 && orderBefore.indexOf("doc-block-7") < orderBefore.indexOf("doc-block-16"))) {
      throw new Error(`Unexpected block order before reorder: ${JSON.stringify(orderBefore)}`);
    }
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="doc-block-up-16"]')`,
      "doc-block-up-16"
    );
    await waitForCondition(
      cdp,
      "Block #16 reordered above block #7",
      () => {
        const ids = Array.from(
          document.querySelectorAll('[data-testid="docs-blocks-list"] [data-testid^="doc-block-"]')
        )
          .map((el) => el.getAttribute("data-testid"))
          .filter((id) => /^doc-block-\d+$/.test(id));
        const idx16 = ids.indexOf("doc-block-16");
        const idx7 = ids.indexOf("doc-block-7");
        return idx16 >= 0 && idx7 >= 0 && idx16 < idx7;
      }
    );

    // Export Markdown and assert full structure, checked item, edited text, and reordered sequence.
    await clickElement(cdp, `document.querySelector('[data-testid="doc-export-md"]')`, "doc-export-md");
    await waitForCondition(
      cdp,
      "Markdown export preview rendered with edited & reordered blocks",
      () => {
        const md = document.querySelector('[data-testid="doc-markdown-preview"]')?.innerText || "";
        return (
          md.includes("MARKDOWN EXPORT") &&
          md.includes("# RFC-042: Zero-Copy NveSceneView") &&
          md.includes("- [x] Enable crash-safe Frame-0 NVE2 + StateFile local persistence in Studio") &&
          md.includes("Verified binary NveWireFrame WebSocket streaming")
        );
      }
    );
    const mdText = await evalJs(
      cdp,
      `document.querySelector('[data-testid="doc-markdown-preview"]')?.innerText || ''`
    );
    const editedPos = mdText.indexOf(editedBlockText);
    const checkPos = mdText.indexOf("- [x] Enable crash-safe Frame-0 NVE2");
    if (!(editedPos >= 0 && checkPos > editedPos)) {
      throw new Error(
        `Markdown export did not reflect block reorder (editedPos=${editedPos}, checkPos=${checkPos}):\n${mdText}`
      );
    }
    console.log(
      "    PASS Studio Docs View (seeded docs, tab switch, add/edit block, checklist toggle, reorder, Markdown export)"
    );

    // =========================================================================
    // 5. Studio Workbench View (`ActiveView::Workbench`)
    // =========================================================================
    console.log("==> [5/7] Testing Studio Workbench View (ActiveView::Workbench)...");
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="nav-workbench"]')`,
      "nav-workbench"
    );
    await waitForCondition(
      cdp,
      "Workbench view rendered with 3 viewport presets and 9 connected host cards",
      () => {
        const grid = document.querySelector('[data-testid="wb-viewport-grid"]');
        const hosts = document.querySelectorAll('[data-testid^="wb-host-"]');
        return (
          grid?.getAttribute("data-workbench-preset") === "all" &&
          Boolean(document.querySelector('[data-testid="wb-frame-desktop"]')) &&
          Boolean(document.querySelector('[data-testid="wb-frame-tablet"]')) &&
          Boolean(document.querySelector('[data-testid="wb-frame-mobile"]')) &&
          Boolean(document.querySelector('[data-testid="wb-telemetry-banner"]')) &&
          hosts.length === 9
        );
      }
    );

    const wbAllGeom = await evalJs(
      cdp,
      `(() => {
        const readRect = (el) => {
          const r = el.getBoundingClientRect();
          return {
            id: el.getAttribute('data-testid'),
            left: Math.round(r.left),
            top: Math.round(r.top),
            right: Math.round(r.right),
            bottom: Math.round(r.bottom),
            w: Math.round(r.width),
            h: Math.round(r.height),
          };
        };
        const frames = ['wb-frame-desktop', 'wb-frame-tablet', 'wb-frame-mobile']
          .map((id) => readRect(document.querySelector('[data-testid="' + id + '"]')));
        const hosts = Array.from(document.querySelectorAll('[data-testid^="wb-host-"]')).map(readRect);
        return { frames, hosts };
      })()`
    );
    assertNonCollapsedAndNonOverlapping(wbAllGeom.frames, 80, 60, "workbench viewport frames (all)");
    assertNonCollapsedAndNonOverlapping(wbAllGeom.hosts, 80, 35, "workbench 9 host-matrix cards");

    // Cycle viewport presets: Desktop -> Tablet -> Mobile -> All.
    for (const [btnId, presetName, expectedFrame, absentFrames] of [
      ["wb-preset-desktop", "desktop", "wb-frame-desktop", ["wb-frame-tablet", "wb-frame-mobile"]],
      ["wb-preset-tablet", "tablet", "wb-frame-tablet", ["wb-frame-desktop", "wb-frame-mobile"]],
      ["wb-preset-mobile", "mobile", "wb-frame-mobile", ["wb-frame-desktop", "wb-frame-tablet"]],
    ]) {
      await clickElement(cdp, `document.querySelector('[data-testid="${btnId}"]')`, btnId);
      await waitForCondition(
        cdp,
        `Workbench preset switched to ${presetName}`,
        `() => {
          const grid = document.querySelector('[data-testid="wb-viewport-grid"]');
          const active = document.querySelector('[data-testid="${expectedFrame}"]');
          const absentOk = ${JSON.stringify(absentFrames)}.every(
            (id) => !document.querySelector('[data-testid="' + id + '"]')
          );
          if (!grid || !active || !absentOk) return false;
          const r = active.getBoundingClientRect();
          return grid.getAttribute('data-workbench-preset') === ${JSON.stringify(presetName)} && r.width > 120 && r.height > 60;
        }`
      );
    }

    await clickElement(cdp, `document.querySelector('[data-testid="wb-preset-all"]')`, "wb-preset-all");
    await waitForCondition(
      cdp,
      "Workbench preset restored to all (3) frames",
      () =>
        document.querySelector('[data-testid="wb-viewport-grid"]')?.getAttribute("data-workbench-preset") ===
          "all" &&
        Boolean(document.querySelector('[data-testid="wb-frame-desktop"]')) &&
        Boolean(document.querySelector('[data-testid="wb-frame-tablet"]')) &&
        Boolean(document.querySelector('[data-testid="wb-frame-mobile"]'))
    );

    // Toggle device chrome off/on and select a host card to verify interactivity.
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="wb-toggle-chrome"]')`,
      "wb-toggle-chrome (off)"
    );
    await waitForCondition(
      cdp,
      "Workbench device chrome toggled off",
      () => !(document.querySelector('[data-testid="wb-frame-desktop"]')?.innerText || "").includes("1280x800")
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="wb-toggle-chrome"]')`,
      "wb-toggle-chrome (on)"
    );
    await waitForCondition(
      cdp,
      "Workbench device chrome restored",
      () => (document.querySelector('[data-testid="wb-frame-desktop"]')?.innerText || "").includes("1280x800")
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid^="wb-host-"]')`,
      "workbench first host card"
    );
    console.log(
      `    PASS Studio Workbench View (presets all/desktop/tablet/mobile, 3 frames + 9 host cards non-collapsed & non-overlapping)`
    );

    // =========================================================================
    // 6. Live `nve pair` Binary WebSocket Thin-Client (`?pair=ws://...&token=...`)
    // =========================================================================
    console.log("==> [6/7] Testing Live nve pair Binary WebSocket Thin-Client (?pair=ws://...&token=...)...");
    const { nveBin, kitchenSinkDylib } = ensurePairArtifacts();
    try {
      pairServer = await launchPairServer(nveBin, kitchenSinkDylib);
      assertIncludes(pairServer.stdout, "nve://pair?", "nve pair stdout URI");
      assertIncludes(
        pairServer.stdout,
        `?pair=${pairServer.wsUrl}&token=${pairServer.token}`,
        "nve pair stdout browser query"
      );
      console.log(
        `    Spawned nve pair (ws=${pairServer.wsUrl}, udp=${pairServer.udpAddr}, tcp=${pairServer.tcpAddr})`
      );

      const pairPageUrl = `${baseUrl}/?pair=${encodeURIComponent(pairServer.wsUrl)}&token=${encodeURIComponent(pairServer.token)}`;
      await cdp.send("Page.navigate", { url: pairPageUrl });
      await waitForCondition(cdp, "dev bundle loaded for ?pair=", () => window.__nveBundle === "dev");

      await waitForCondition(
        cdp,
        "Live paired thin-client connected with ZERO local WASM app and rendered Kitchen Sink Count: 0",
        () => {
          const host = window.__nvePairHost;
          const status = document.getElementById("nve-status");
          const app = document.getElementById("app");
          const text = app?.innerText || "";
          return (
            window.__nveController === undefined &&
            Boolean(host) &&
            host.is_connected() &&
            host.frames_received() >= 1 &&
            status?.dataset.state === "connected" &&
            status?.dataset.pairConnected === "true" &&
            app?.dataset.nveMode === "paired" &&
            text.includes("NVE Kitchen Sink") &&
            text.includes("Count: 0") &&
            text.includes("Increment +") &&
            text.includes("Reset")
          );
        }
      );

      // Click Increment + twice -> live EventBatch -> Rust KitchenSinkModule -> NveWireFrame delta -> DOM.
      await clickElement(cdp, findByTestId("ks-counter-inc"), "Paired Increment + (1st click)");
      await waitForCondition(
        cdp,
        "Paired Kitchen Sink incremented to Count: 1 over live binary WebSocket",
        () => (document.getElementById("app")?.innerText || "").includes("Count: 1")
      );

      await clickElement(cdp, findByTestId("ks-counter-inc"), "Paired Increment + (2nd click)");
      await waitForCondition(
        cdp,
        "Paired Kitchen Sink incremented to Count: 2 over live binary WebSocket",
        () =>
          (document.getElementById("app")?.innerText || "").includes("Count: 2") &&
          Number(window.__nvePairHost?.frames_received() || 0) >= 3
      );

      // Click Reset -> verify Count: 0 round-trip.
      await clickElement(cdp, findByTestId("ks-counter-reset"), "Paired Reset");
      await waitForCondition(
        cdp,
        "Paired Kitchen Sink reset to Count: 0 over live binary WebSocket",
        () => (document.getElementById("app")?.innerText || "").includes("Count: 0")
      );

      const pairStats = await evalJs(
        cdp,
        `(() => ({
          sessionId: window.__nvePairHost?.session_id(),
          framesReceived: Number(window.__nvePairHost?.frames_received() || 0),
          nodeCount: Number(window.__nvePairHost?.node_count() || 0),
          statusText: document.getElementById('nve-status')?.textContent,
          pairState: document.getElementById('nve-status')?.dataset.pairState,
        }))()`
      );
      console.log(
        `    PASS Live nve pair Thin-Client (session=${pairStats.sessionId}, frames=${pairStats.framesReceived}, nodes=${pairStats.nodeCount}, Count: 0 -> 1 -> 2 -> 0, status="${pairStats.statusText}")`
      );

      // Gracefully disconnect before terminating the nve pair subprocess so Chrome logs no close error.
      await evalJs(cdp, `window.__nvePairHost?.disconnect(); true`);
      await cdp.send("Page.navigate", { url: "about:blank" });
    } finally {
      await stopChildProcess(pairServer?.proc);
      pairServer = null;
    }

    // =========================================================================
    // 7. Option 5 Hybrid CSS + Semantic SSR + Zero-Flicker Hydration + Progressive Islands
    // =========================================================================
    console.log("==> [7/7] Testing Option 5 Hybrid CSS, Semantic SSR & Zero-Flicker Hydration (?app=studio&ws=off&ssr=1)...");
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=studio&ws=off&ssr=1` });
    await waitForCondition(cdp, "production studio bundle loaded for ?ssr=1", () => window.__nveBundle === "studio");

    await waitForCondition(
      cdp,
      "StudioWorkspaceApp hydrated over SSR DOM with 0 mismatches",
      () => {
        const ctrl = window.__nveController;
        const text = document.getElementById("app")?.innerText || "";
        return (
          Boolean(ctrl) &&
          ctrl.is_hydrated() &&
          Boolean(window.__nveSsrPreHydrate) &&
          text.includes("AETHER") &&
          text.includes("AET-101")
        );
      }
    );

    const hydrateStats = await evalJs(
      cdp,
      `(() => {
        const ctrl = window.__nveController;
        const pre = window.__nveSsrPreHydrate;
        const rootAfter = document.querySelector('#app [data-nve-id="1"]');
        const searchAfter = document.querySelector('[data-testid="board-search-input"]');
        const statusAfter = document.querySelector('[data-testid="task-status-1"]');
        const invalidStyleAttrs = document.querySelectorAll('[font-weight],[text-align],[box-shadow]').length;
        return {
          isHydrated: ctrl.is_hydrated(),
          mismatches: Number(ctrl.hydration_mismatch_count()),
          adopted: Number(ctrl.hydration_adopted_count()),
          replaced: Number(ctrl.hydration_replaced_count()),
          seededRules: Number(ctrl.hydration_seeded_rules()),
          sheetRules: Number(ctrl.stylesheet_rule_count()),
          rootIdentityPreserved: pre.rootEl === rootAfter,
          searchIdentityPreserved: pre.searchInput === searchAfter,
          statusIdentityPreserved: pre.statusBtn === statusAfter,
          invalidStyleAttrs,
          ssrHtmlBytes: pre.htmlBytes,
          ssrCssBytes: pre.cssBytes,
          legacyHtmlBytes: ctrl.render_html().length,
        };
      })()`
    );

    if (
      !hydrateStats.isHydrated ||
      hydrateStats.mismatches !== 0 ||
      hydrateStats.replaced !== 0 ||
      !(hydrateStats.adopted > 20) ||
      !(hydrateStats.seededRules > 0) ||
      !hydrateStats.rootIdentityPreserved ||
      !hydrateStats.searchIdentityPreserved ||
      !hydrateStats.statusIdentityPreserved ||
      hydrateStats.invalidStyleAttrs !== 0
    ) {
      throw new Error(`SSR hydration invariant failed: ${JSON.stringify(hydrateStats)}`);
    }

    // 7a. Verify preserved <input> focus & element identity across keyed SetChildren updates.
    await typeIntoInput(
      cdp,
      `document.querySelector('[data-testid="board-search-input"]')`,
      "AppKit",
      "hydrated board-search-input"
    );
    await waitForCondition(
      cdp,
      "Hydrated search filtered board to AET-102 while preserving <input> focus",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        const inp = document.querySelector('[data-testid="board-search-input"]');
        return (
          text.includes("AET-102") &&
          !text.includes("AET-101") &&
          document.activeElement === inp &&
          inp === window.__nveSsrPreHydrate.searchInput
        );
      }
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="board-search-clear"]')`,
      "hydrated board-search-clear"
    );
    await waitForCondition(
      cdp,
      "Hydrated board search cleared via root event delegation",
      () => (document.getElementById("app")?.innerText || "").includes("AET-101")
    );
    await clickElement(
      cdp,
      `document.querySelector('[data-testid="task-status-1"]')`,
      "hydrated task-status-1"
    );
    await waitForCondition(
      cdp,
      "Hydrated task-status-1 delegated click updated Done (2)",
      () => (document.getElementById("app")?.innerText || "").includes("Done (2)")
    );

    // 7b. Verify ProgressiveIslandHost (load, visible via IntersectionObserver, interaction + event replay, idle).
    const islandCheck = await evalJs(
      cdp,
      `(async () => {
        const box = document.createElement('div');
        box.id = 'nve-island-probe';
        box.innerHTML =
          '<div data-nve-id="20" data-nve-island="isl-load" data-nve-hydrate="load"></div>' +
          '<div data-nve-id="21" data-nve-island="isl-vis" data-nve-hydrate="visible" style="width:20px;height:20px;"></div>' +
          '<div data-nve-id="22" data-nve-island="isl-int" data-nve-hydrate="interaction"><button id="isl-btn" data-nve-id="23">Go</button></div>' +
          '<div data-nve-id="24" data-nve-island="isl-idle" data-nve-hydrate="idle"></div>';
        document.body.appendChild(box);
        let replayedClicks = 0;
        const intEl = box.querySelector('[data-nve-island="isl-int"]');
        intEl.addEventListener('nve:island-hydrated', () => {
          box.querySelector('#isl-btn').addEventListener('click', () => { replayedClicks += 1; });
        });
        const host = window.__nveWasmMod.ProgressiveIslandHost.attach('nve-island-probe');
        const loadImmediate = host.is_hydrated('isl-load');
        const intBeforeClick = host.is_hydrated('isl-int');
        box.querySelector('#isl-btn').click();
        await new Promise((r) => setTimeout(r, 120));
        const res = {
          loadImmediate,
          intBeforeClick,
          visHydrated: host.is_hydrated('isl-vis'),
          intHydrated: host.is_hydrated('isl-int'),
          idleHydrated: host.is_hydrated('isl-idle'),
          hydratedCount: Number(host.hydrated_count()),
          replayedEvents: Number(host.replayed_event_count()),
          replayedClicks,
        };
        host.free();
        box.remove();
        return res;
      })()`
    );
    if (
      !islandCheck.loadImmediate ||
      islandCheck.intBeforeClick ||
      !islandCheck.visHydrated ||
      !islandCheck.intHydrated ||
      !islandCheck.idleHydrated ||
      islandCheck.hydratedCount !== 4 ||
      islandCheck.replayedEvents < 1 ||
      islandCheck.replayedClicks < 1
    ) {
      throw new Error(`ProgressiveIslandHost E2E check failed: ${JSON.stringify(islandCheck)}`);
    }

    // The semantic-style probe is a dev-bundle export (`dev` feature).
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=studio&ws=off&bundle=dev` });
    await waitForCondition(
      cdp,
      "dev bundle (?bundle=dev) mounted StudioWorkspaceApp with the hydration probe export",
      () =>
        window.__nveBundle === "dev" &&
        typeof window.__nveWasmMod?.run_semantic_style_hydration_probe === "function" &&
        window.__nveController?.app_name === "StudioWorkspaceApp"
    );

    // 7c. Verify semantic HTML5 tags + non-empty rendered box-shadow / font-weight / text-align
    //     + zero-mismatch hydration identity preservation on the style probe.
    const styleProbe = await evalJs(
      cdp,
      `(() => {
        const host = document.createElement('div');
        host.id = 'nve-style-probe';
        document.body.appendChild(host);
        // First pass mounts SSR HTML + hydrates; capture pre-hydrate nodes from a second clean hydration pass.
        const firstMismatches = window.__nveWasmMod.run_semantic_style_hydration_probe('nve-style-probe');
        const cardBefore = host.querySelector('article[data-testid="probe-card"]');
        const headingBefore = host.querySelector('h1[data-testid="probe-heading"]');
        const linkBefore = host.querySelector('a[data-testid="probe-link"]');
        const secondMismatches = window.__nveWasmMod.run_semantic_style_hydration_probe('nve-style-probe');
        const cardAfter = host.querySelector('article[data-testid="probe-card"]');
        const headingAfter = host.querySelector('h1[data-testid="probe-heading"]');
        const linkAfter = host.querySelector('a[data-testid="probe-link"]');
        const cardStyle = cardAfter ? getComputedStyle(cardAfter) : null;
        const headingStyle = headingAfter ? getComputedStyle(headingAfter) : null;
        const res = {
          firstMismatches: Number(firstMismatches),
          secondMismatches: Number(secondMismatches),
          cardIdentity: Boolean(cardBefore && cardBefore === cardAfter),
          headingIdentity: Boolean(headingBefore && headingBefore === headingAfter),
          linkIdentity: Boolean(linkBefore && linkBefore === linkAfter),
          linkHref: linkAfter?.getAttribute('href'),
          boxShadow: cardStyle?.boxShadow || '',
          fontWeight: headingStyle?.fontWeight || '',
          textAlign: headingStyle?.textAlign || '',
          invalidAttrs: host.querySelectorAll('[box-shadow],[font-weight],[text-align]').length,
        };
        host.remove();
        return res;
      })()`
    );

    if (
      styleProbe.firstMismatches !== 0 ||
      styleProbe.secondMismatches !== 0 ||
      !styleProbe.cardIdentity ||
      !styleProbe.headingIdentity ||
      !styleProbe.linkIdentity ||
      styleProbe.linkHref !== "/docs/ssr" ||
      !styleProbe.boxShadow ||
      styleProbe.boxShadow === "none" ||
      styleProbe.fontWeight !== "700" ||
      styleProbe.textAlign !== "center" ||
      styleProbe.invalidAttrs !== 0
    ) {
      throw new Error(`Semantic SSR / RawStyleProps hydration probe failed: ${JSON.stringify(styleProbe)}`);
    }

    console.log(
      `    PASS SSR + Zero-Flicker Hydration (adopted=${hydrateStats.adopted}, mismatches=0, elBefore===elAfter, focus preserved, boxShadow=${styleProbe.boxShadow}, fontWeight=${styleProbe.fontWeight}, textAlign=${styleProbe.textAlign}, islands=4/4 replayed=${islandCheck.replayedClicks}, SSR=${hydrateStats.ssrHtmlBytes + hydrateStats.ssrCssBytes}B vs legacy=${hydrateStats.legacyHtmlBytes}B)`
    );

    // 7d. Hostile CommandBuffers against the live DOM path (root container, cycles, moves,
    //     reserved keys, CSS-only props, CSS injection, input type, draw ops, text vs children).
    const hostile = await evalJs(
      cdp,
      `(() => {
        const host = document.createElement('div');
        host.id = 'nve-hostile-probe';
        document.body.appendChild(host);
        window.__nveWasmMod.run_hostile_frame_probe('nve-hostile-probe');
        const all = (id) => host.querySelectorAll('[data-nve-id="' + id + '"]');
        const n = (id) => all(id)[0];
        const id = (el) => el?.getAttribute('data-nve-id');
        const root = host.firstElementChild;
        const res = {
          connected: host.isConnected && document.getElementById('nve-hostile-probe') === host,
          rootTag: root?.tagName,
          rootId: root?.getAttribute('data-nve-id'),
          rootChildren: root ? [...root.children].map(id).join(',') : '',
          counts: [2, 3, 4, 5, 6, 7, 8].map((i) => all(i).length).join(','),
          movedParent: id(n(6)?.parentElement),
          keptChildParent: id(n(7)?.parentElement),
          node6Text: n(6)?.textContent,
          node2Text: n(2)?.textContent,
          node2Elements: n(2)?.childElementCount,
          node2Class: n(2)?.getAttribute('class') || '',
          node2Style: n(2)?.getAttribute('style') || '',
          flexWrap: n(2) ? getComputedStyle(n(2)).flexWrap : '',
          fontWeight: n(2) ? getComputedStyle(n(2)).fontWeight : '',
          boxShadow: n(2) ? getComputedStyle(n(2)).boxShadow : '',
          hidden: [2, 3, 4, 6, 7, 8].filter((i) => n(i) && getComputedStyle(n(i)).display === 'none').join(','),
          inputType: n(3)?.getAttribute('type'),
          drawOps: n(4)?.getAttribute('data-nve-draw-ops'),
          rawAttrs: host.querySelectorAll('[flex-wrap],[font-weight],[box-shadow],[securetextentry],[role]').length,
        };
        host.remove();
        return res;
      })()`
    );
    const hostileExpected = {
      connected: true,
      rootTag: "DIV",
      rootId: "1",
      rootChildren: "2,3,4,8",
      counts: "1,1,1,0,1,1,1",
      movedParent: "8",
      keptChildParent: "6",
      node6Text: "kept child",
      node2Text: "restored text",
      node2Elements: 0,
      flexWrap: "wrap",
      fontWeight: "700",
      boxShadow: "none",
      hidden: "",
      inputType: null,
      drawOps: null,
      rawAttrs: 0,
    };
    const hostileDiff = Object.entries(hostileExpected).filter(([k, v]) => hostile[k] !== v);
    if (hostileDiff.length || hostile.node2Class.includes("evil") || hostile.node2Style.includes("255, 0, 0")) {
      throw new Error(`Hostile frame probe failed (${JSON.stringify(hostileDiff)}): ${JSON.stringify(hostile)}`);
    }
    console.log("    PASS hostile CommandBuffers on the live DOM (root kept, moves/cycles/repeats, reserved keys, CSS-only props)");

    // 7d2. Track 2A live Chrome DOM behavioral probe:
    //      - P0#1 + P1#6: post-flush requested_scroll_seq on Frame 1 mount & 1 -> 0 -> 1 reset
    //      - P0#2: Overlay-only open/close driving real <dialog>:modal and [popover]:popover-open
    //      - P0#3 + P1#7-web: OverlayAnchorFallbackManager inline style cleanup + anchor-name strip
    //      - P0#5: disabled capture exclusion + live CDP pointer capture pruned on mid-drag removal
    const t2aSetup = await evalJs(
      cdp,
      `(() => {
        const host = document.createElement('div');
        host.id = 'nve-track2a-probe';
        host.style.cssText = 'position:fixed;inset:0;z-index:99999;background:#fff;';
        document.body.appendChild(host);
        const probe = new window.__nveWasmMod.Track2aProbe('nve-track2a-probe');
        window.__nveTrack2aProbe = probe;
        window.__nveTrack2aHost = host;
        const by = (tid) => host.querySelector('[data-testid="' + tid + '"]');
        const sc = by('t2a-scroller');
        const dlg = by('t2a-dialog');
        const pop = by('t2a-popover');
        const anc = by('t2a-anchor');
        const ov = by('t2a-anchored-ov');
        const dcap = by('t2a-disabled-cap');
        const lcap = by('t2a-live-cap');
        const supportsAnchor = CSS.supports('anchor-name', '--a') && CSS.supports('position-area', 'bottom center');
        const f1 = {
          scrollTop1: sc?.scrollTop ?? -1,
          dlgModal1: dlg?.matches(':modal') ?? null,
          popOpen1: pop?.matches(':popover-open') ?? null,
          ovLeft1: ov?.style.getPropertyValue('left') || '',
          ovTop1: ov?.style.getPropertyValue('top') || '',
          ovPosArea1: ov?.style.getPropertyValue('position-area') || '',
          anchorName1: supportsAnchor ? getComputedStyle(anc).getPropertyValue('anchor-name') : '--nve-a-t2a-anchor',
          disabledCapAttr: dcap?.getAttribute('data-nve-capture'),
        };
        probe.step_open_overlays_and_reset_scroll_zero();
        const f2 = {
          dlgModal2: dlg?.matches(':modal') ?? null,
          popOpen2: pop?.matches(':popover-open') ?? null,
        };
        if (sc) sc.scrollTop = 0;
        const scrollTopReset = sc?.scrollTop ?? -1;
        probe.step_scroll_seq_one_and_close_overlays();
        const leakedProps = ['position', 'position-area', 'margin', 'left', 'top', 'max-width', 'max-height']
          .filter((p) => (ov?.style.getPropertyValue(p) || '') !== '');
        const f3 = {
          scrollTopReset,
          scrollTop3: sc?.scrollTop ?? -1,
          dlgOpen3: dlg?.open ?? null,
          dlgModal3: dlg?.matches(':modal') ?? null,
          popOpen3: pop?.matches(':popover-open') ?? null,
          leakedProps,
          anchorName3: supportsAnchor ? getComputedStyle(anc).getPropertyValue('anchor-name') : 'none',
        };
        const dRect = dcap.getBoundingClientRect();
        const lRect = lcap.getBoundingClientRect();
        probe.drain_events_summary();
        return {
          f1,
          f2,
          f3,
          dCenter: { x: dRect.left + dRect.width / 2, y: dRect.top + dRect.height / 2 },
          lCenter: { x: lRect.left + lRect.width / 2, y: lRect.top + lRect.height / 2 },
        };
      })()`
    );

    // Real Chromium pointer input on disabled capture target: must NOT capture!
    await cdp.send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: t2aSetup.dCenter.x,
      y: t2aSetup.dCenter.y,
      button: "left",
      buttons: 1,
      clickCount: 1,
    });
    const dDownEv = await evalJs(cdp, `window.__nveTrack2aProbe.drain_events_summary()`);
    await cdp.send("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: 340,
      y: 340,
      buttons: 1,
    });
    const dMoveEv = await evalJs(cdp, `window.__nveTrack2aProbe.drain_events_summary()`);
    await cdp.send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: 340,
      y: 340,
      button: "left",
      buttons: 0,
      clickCount: 1,
    });
    await evalJs(cdp, `window.__nveTrack2aProbe.drain_events_summary()`);

    // Real Chromium pointer input on enabled capture target + mid-drag DOM removal!
    await cdp.send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: t2aSetup.lCenter.x,
      y: t2aSetup.lCenter.y,
      button: "left",
      buttons: 1,
      clickCount: 1,
    });
    const lDownEv = await evalJs(cdp, `window.__nveTrack2aProbe.drain_events_summary()`);
    await cdp.send("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: 360,
      y: 360,
      buttons: 1,
    });
    const lMoveEv = await evalJs(cdp, `window.__nveTrack2aProbe.drain_events_summary()`);
    await evalJs(cdp, `window.__nveTrack2aProbe.step_remove_live_cap()`);
    await cdp.send("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: 380,
      y: 380,
      buttons: 1,
    });
    await cdp.send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: 380,
      y: 380,
      button: "left",
      buttons: 0,
      clickCount: 1,
    });
    const lAfterRemoveEv = await evalJs(
      cdp,
      `(() => {
        const summary = window.__nveTrack2aProbe.drain_events_summary();
        window.__nveTrack2aProbe.free();
        delete window.__nveTrack2aProbe;
        window.__nveTrack2aHost.remove();
        delete window.__nveTrack2aHost;
        return summary;
      })()`
    );

    if (
      t2aSetup.f1.scrollTop1 < 100 ||
      t2aSetup.f1.dlgModal1 !== false ||
      t2aSetup.f1.popOpen1 !== false ||
      !t2aSetup.f1.ovLeft1 ||
      !t2aSetup.f1.ovTop1 ||
      t2aSetup.f1.ovPosArea1 !== "none" ||
      !t2aSetup.f1.anchorName1.includes("--nve-a-t2a-anchor") ||
      t2aSetup.f1.disabledCapAttr !== null ||
      t2aSetup.f2.dlgModal2 !== true ||
      t2aSetup.f2.popOpen2 !== true ||
      t2aSetup.f3.scrollTopReset !== 0 ||
      t2aSetup.f3.scrollTop3 < 100 ||
      t2aSetup.f3.dlgOpen3 !== false ||
      t2aSetup.f3.dlgModal3 !== false ||
      t2aSetup.f3.popOpen3 !== false ||
      t2aSetup.f3.leakedProps.length !== 0 ||
      t2aSetup.f3.anchorName3.includes("--nve-a-t2a-anchor") ||
      dDownEv.includes("cap=1") ||
      dMoveEv.includes("8:") ||
      !lDownEv.includes("9:down:cap=1;") ||
      !lMoveEv.includes("9:move:cap=1;") ||
      lAfterRemoveEv.includes("9:")
    ) {
      throw new Error(
        `Track 2A live DOM probe failed: ${JSON.stringify({
          t2aSetup,
          dDownEv,
          dMoveEv,
          lDownEv,
          lMoveEv,
          lAfterRemoveEv,
        })}`
      );
    }
    console.log(
      `    PASS Track 2A live Chrome DOM probe (post-flush scrollTop=${t2aSetup.f1.scrollTop1}->0->${t2aSetup.f3.scrollTop3}, Overlay :modal & :popover-open, fallback inline strip, anchor-name strip, disabled/mid-drag capture hardened)`
    );

    // =========================================================================
    // 8. Generic Web SSG (Cold Visit) + 0ms Instant Snapshot & State Restore (Returning Visit)
    // =========================================================================
    console.log("==> [8/8] Testing Cold SSG + 0ms Instant Snapshot & State Restore (.wasm blocked & unblocked, hydration, replay, ?reset=1)...");
    await cdp.send("Network.enable");

    // Stage real pre-rendered `/examples/_runtime/{studio,kitchen-sink}.html` via `nve docs build`
    // using the freshly built `hosts/web-shell/pkg/` bundles, served on a clean origin (empty localStorage).
    const ssgTmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "nve-web-ssg-e2e-"));
    const ssgBuild = spawnSync(
      nveBin,
      [
        "docs",
        "build",
        "--content",
        "docs/content",
        "--skip-wasm",
        "--web-shell-dist",
        WEB_SHELL_DIR,
        "--out",
        ssgTmpDir,
      ],
      { cwd: REPO_ROOT, encoding: "utf8" }
    );
    if (ssgBuild.status !== 0) {
      throw new Error(
        `nve docs build for SSG E2E failed (exit=${ssgBuild.status}):\n${ssgBuild.stdout}\n${ssgBuild.stderr}`
      );
    }
    const { server: ssgServer, port: ssgPort } = await startStaticServer(ssgTmpDir);
    const ssgBaseUrl = `http://127.0.0.1:${ssgPort}`;
    const studioSsgUrl = `${ssgBaseUrl}/examples/_runtime/studio.html?app=studio&ws=off`;
    const kitchenSinkSsgUrl = `${ssgBaseUrl}/examples/_runtime/kitchen-sink.html?app=kitchen-sink&ws=off`;

    try {
      // 8a. COLD FIRST VISIT (empty localStorage, .wasm COMPLETELY BLOCKED) -> full initial
      //     Studio AND Kitchen Sink UIs must ALREADY be visibly rendered at 0ms from embedded SSG HTML+CSS!
      allowExpectedOfflineWasmError = true;
      await cdp.send("Network.setBlockedURLs", { urls: ["*.wasm"] });
      try {
        await cdp.send("Page.navigate", { url: studioSsgUrl });
        await waitForCondition(
          cdp,
          "Cold first-visit Studio SSG rendered full initial UI at 0ms with empty localStorage and .wasm blocked",
          () => {
            const app = document.getElementById("app");
            const status = document.getElementById("nve-status");
            const text = app?.innerText || "";
            return (
              localStorage.length === 0 &&
              app?.dataset.nveRestored === undefined &&
              window.__nveWasmMod === undefined &&
              window.__nveController === undefined &&
              Boolean(document.querySelector("style[data-nve-base]")) &&
              Boolean(document.querySelector("style[data-nve-ssr][data-nve-hashes]")) &&
              Boolean(app?.querySelector("[data-nve-ssr-hash]")) &&
              typeof window.__nveReplayDrain === "function" &&
              text.includes("AETHER") &&
              text.includes("Issue Board") &&
              text.includes("AET-101") &&
              text.includes("Done (1)") &&
              status?.textContent === "Snapshot (offline)"
            );
          }
        );

        await cdp.send("Page.navigate", { url: kitchenSinkSsgUrl });
        await waitForCondition(
          cdp,
          "Cold first-visit Kitchen Sink SSG rendered full initial UI at 0ms with empty localStorage and .wasm blocked",
          () => {
            const app = document.getElementById("app");
            const status = document.getElementById("nve-status");
            const text = app?.innerText || "";
            return (
              localStorage.length === 0 &&
              app?.dataset.nveRestored === undefined &&
              window.__nveWasmMod === undefined &&
              window.__nveController === undefined &&
              Boolean(document.querySelector("style[data-nve-base]")) &&
              Boolean(document.querySelector("style[data-nve-ssr][data-nve-hashes]")) &&
              Boolean(app?.querySelector("[data-nve-ssr-hash]")) &&
              text.includes("NVE Kitchen Sink") &&
              text.includes("Count: 0") &&
              text.includes("Increment +") &&
              status?.textContent === "Snapshot (offline)"
            );
          }
        );
        console.log("    PASS Cold First-Visit SSG with .wasm COMPLETELY BLOCKED (Studio + Kitchen Sink painted at 0ms with empty localStorage)");
      } finally {
        await cdp.send("Network.setBlockedURLs", { urls: [] });
        allowExpectedOfflineWasmError = false;
      }

      // 8b. COLD FIRST-VISIT ZERO-MISMATCH HYDRATION (.wasm unblocked, empty localStorage) for BOTH Studio & Counter!
      const verifyColdSsgHydration = async (url, label, expectedAppName, minAdopted) => {
        const paused = [];
        const onPaused = (p) => paused.push(p.requestId);
        cdp.on("Fetch.requestPaused", onPaused);
        await cdp.send("Fetch.enable", {
          patterns: [{ urlPattern: "*.wasm", requestStage: "Request" }],
        });
        try {
          await cdp.send("Page.navigate", { url });
          await waitForCondition(
            cdp,
            `${label} cold SSG DOM present before .wasm resolves`,
            () =>
              localStorage.length === 0 &&
              Boolean(document.querySelector('#app [data-nve-id="1"][data-nve-ssr-hash]')) &&
              window.__nveController === undefined
          );
          await evalJs(
            cdp,
            `(() => { window.__nveColdSsgRootBefore = document.querySelector('#app [data-nve-id="1"]'); return Boolean(window.__nveColdSsgRootBefore); })()`
          );
        } finally {
          for (const requestId of paused) {
            await cdp.send("Fetch.continueRequest", { requestId }).catch(() => {});
          }
          await cdp.send("Fetch.disable").catch(() => {});
        }
        await waitForCondition(
          cdp,
          `${label} cold SSG hydrated with 0 mismatches`,
          `() => window.__nveController?.app_name === ${JSON.stringify(expectedAppName)} && window.__nveController?.is_hydrated() === true`
        );
        const stats = await evalJs(
          cdp,
          `(() => {
            const ctrl = window.__nveController;
            return {
              hydrated: ctrl.is_hydrated(),
              restored: ctrl.restored_from_snapshot(),
              mismatches: Number(ctrl.hydration_mismatch_count()),
              replaced: Number(ctrl.hydration_replaced_count()),
              adopted: Number(ctrl.hydration_adopted_count()),
              rootIdentityPreserved: window.__nveColdSsgRootBefore === document.querySelector('#app [data-nve-id="1"]'),
            };
          })()`
        );
        if (
          !stats.hydrated ||
          stats.restored !== false ||
          stats.mismatches !== 0 ||
          stats.replaced !== 0 ||
          !(stats.adopted >= minAdopted) ||
          !stats.rootIdentityPreserved
        ) {
          throw new Error(`${label} cold SSG hydration failed: ${JSON.stringify(stats)}`);
        }
        return stats;
      };

      const coldKitchenSinkStats = await verifyColdSsgHydration(kitchenSinkSsgUrl, "Kitchen Sink", "KitchenSinkApp", 20);
      await evalJs(cdp, `window.__nveController?.clear_snapshot(); localStorage.clear(); true`);
      const coldStudioStats = await verifyColdSsgHydration(studioSsgUrl, "Studio", "StudioWorkspaceApp", 20);
      console.log(
        `    PASS Cold First-Visit SSG Hydration (Studio adopted=${coldStudioStats.adopted} mismatches=0 rootId=true; Kitchen Sink adopted=${coldKitchenSinkStats.adopted} mismatches=0 rootId=true)`
      );

      // 8c. RETURNING USER VISIT: Mutate Studio on the SSG page, persist NVWS1 + NWS1,
      //     then block .wasm and assert <script data-nve-snapshot-boot> overrides default Board SSG with mutated Docs at 0ms!
      await typeIntoInput(
        cdp,
        `document.querySelector('[data-testid="board-new-task-input"]')`,
        "Zero-MS Snapshot Task",
        "snapshot board-new-task-input"
      );
      await pressKey(cdp, { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
      await waitForCondition(
        cdp,
        "Mutated Studio added AET-105 Zero-MS Snapshot Task",
        () => (document.getElementById("app")?.innerText || "").includes("Zero-MS Snapshot Task")
      );
      await clickElement(cdp, `document.querySelector('[data-testid="task-status-1"]')`, "snapshot task-status-1");
      await waitForCondition(
        cdp,
        "Mutated Studio advanced AET-101 to Done (2)",
        () => (document.getElementById("app")?.innerText || "").includes("Done (2)")
      );
      await clickElement(cdp, `document.querySelector('[data-testid="nav-docs"]')`, "snapshot nav-docs");
      await waitForCondition(
        cdp,
        "Mutated Studio switched to Docs view",
        () => (document.getElementById("app")?.innerText || "").includes("RFC-042: Zero-Copy NveSceneView")
      );
      await waitForCondition(
        cdp,
        "Studio NVWS1 frame snapshot and NVB1 NWS1 state persisted automatically to localStorage",
        () => {
          const snap = localStorage.getItem("nve:snap:studio:v3") || "";
          const state = localStorage.getItem("nve:state:studio:v3") || "";
          return (
            snap.startsWith("NVWS1|3|") &&
            snap.includes("RFC-042: Zero-Copy NveSceneView") &&
            state.startsWith("NVB1|3|")
          );
        }
      );

      // BLOCK ALL `.wasm` requests and reload Studio SSG page -> returning user's mutated Docs view MUST override default Board SSG at 0ms!
      allowExpectedOfflineWasmError = true;
      await cdp.send("Network.setBlockedURLs", { urls: ["*.wasm"] });
      try {
        await cdp.send("Page.navigate", { url: studioSsgUrl });
        await waitForCondition(
          cdp,
          "Returning-user 0ms snapshot overrode default Board SSG with mutated Docs view while .wasm was blocked",
          () => {
            const app = document.getElementById("app");
            const status = document.getElementById("nve-status");
            const text = app?.innerText || "";
            return (
              app?.dataset.nveRestored === "1" &&
              window.__nveWasmMod === undefined &&
              window.__nveController === undefined &&
              Boolean(document.querySelector("style[data-nve-base]")) &&
              Boolean(document.querySelector("style[data-nve-ssr][data-nve-hashes]")) &&
              typeof window.__nveReplayDrain === "function" &&
              text.includes("AETHER") &&
              text.includes("RFC-042: Zero-Copy NveSceneView") &&
              !text.includes("Issue Board") &&
              status?.textContent === "Snapshot (offline)"
            );
          }
        );
        console.log("    PASS Returning-User 0ms Snapshot Override with .wasm COMPLETELY BLOCKED (mutated Docs overrode cold Board SSG)");
      } finally {
        await cdp.send("Network.setBlockedURLs", { urls: [] });
        allowExpectedOfflineWasmError = false;
      }

      // 8d. Returning-User Hydration + Pre-Hydration Click Replay (pause .wasm mid-flight, click doc-tab-2, release .wasm).
      const pausedWasmRequests = [];
      const onFetchPaused = (params) => {
        pausedWasmRequests.push(params.requestId);
      };
      cdp.on("Fetch.requestPaused", onFetchPaused);
      await cdp.send("Fetch.enable", {
        patterns: [{ urlPattern: "*.wasm", requestStage: "Request" }],
      });
      try {
        await cdp.send("Page.navigate", { url: studioSsgUrl });
        await waitForCondition(
          cdp,
          "0ms snapshot painted while .wasm request is paused mid-flight",
          () =>
            document.getElementById("app")?.dataset.nveRestored === "1" &&
            Boolean(document.querySelector('[data-testid="doc-tab-2"]')) &&
            window.__nveController === undefined
        );
        await evalJs(
          cdp,
          `(() => {
            window.__nveSnapRootBefore = document.querySelector('#app [data-nve-id="1"]');
            return Boolean(window.__nveSnapRootBefore);
          })()`
        );
        await clickElement(cdp, `document.querySelector('[data-testid="doc-tab-2"]')`, "pre-hydration doc-tab-2");
      } finally {
        for (const requestId of pausedWasmRequests) {
          await cdp.send("Fetch.continueRequest", { requestId }).catch(() => {});
        }
        await cdp.send("Fetch.disable").catch(() => {});
      }

      await waitForCondition(
        cdp,
        "WASM hydrated restored mutated Studio snapshot and replayed pre-hydration doc-tab-2 click",
        () => {
          const ctrl = window.__nveController;
          const text = document.getElementById("app")?.innerText || "";
          return (
            Boolean(ctrl) &&
            ctrl.restored_from_snapshot() === true &&
            ctrl.is_hydrated() === true &&
            text.includes("Pre-Flight Gates")
          );
        }
      );

      const snapHydrateStats = await evalJs(
        cdp,
        `(() => {
          const ctrl = window.__nveController;
          const rootAfter = document.querySelector('#app [data-nve-id="1"]');
          return {
            restored: ctrl.restored_from_snapshot(),
            hydrated: ctrl.is_hydrated(),
            mismatches: Number(ctrl.hydration_mismatch_count()),
            replaced: Number(ctrl.hydration_replaced_count()),
            adopted: Number(ctrl.hydration_adopted_count()),
            replayed: Number(ctrl.hydration_replayed_events()),
            rootIdentityPreserved: window.__nveSnapRootBefore === rootAfter,
          };
        })()`
      );
      if (
        !snapHydrateStats.restored ||
        !snapHydrateStats.hydrated ||
        snapHydrateStats.mismatches !== 0 ||
        snapHydrateStats.replaced !== 0 ||
        !(snapHydrateStats.adopted > 20) ||
        !(snapHydrateStats.replayed >= 1) ||
        !snapHydrateStats.rootIdentityPreserved
      ) {
        throw new Error(`Snapshot hydration invariant failed: ${JSON.stringify(snapHydrateStats)}`);
      }

      await clickElement(cdp, `document.querySelector('[data-testid="nav-board"]')`, "restored nav-board");
      await waitForCondition(
        cdp,
        "Restored Studio NWS1 preserved Board task AET-105 and Done (2)",
        () => {
          const text = document.getElementById("app")?.innerText || "";
          return text.includes("Zero-MS Snapshot Task") && text.includes("Done (2)");
        }
      );
      console.log(
        `    PASS Returning-User Studio Hydration + Replay (adopted=${snapHydrateStats.adopted}, mismatches=0, replayed=${snapHydrateStats.replayed}, rootIdentity=true, NWS1 Board intact)`
      );

      // 8e. Kitchen Sink Returning-User 0ms Snapshot + Binary State Restore + Zero-Mismatch Hydration.
      await cdp.send("Page.navigate", { url: `${kitchenSinkSsgUrl}&reset=1` });
      await waitForCondition(
        cdp,
        "Kitchen Sink SSG reset to Count: 0 with 0-mismatch cold SSG hydration",
        () =>
          window.__nveController?.app_name === "KitchenSinkApp" &&
          window.__nveController?.is_hydrated() === true &&
          Number(window.__nveController?.hydration_mismatch_count()) === 0 &&
          (document.getElementById("app")?.innerText || "").includes("Count: 0")
      );
      await cdp.send("Page.navigate", { url: kitchenSinkSsgUrl });
      await waitForCondition(
        cdp,
        "Kitchen Sink SSG mounted for mutation",
        () =>
          window.__nveController?.app_name === "KitchenSinkApp" &&
          (document.getElementById("app")?.innerText || "").includes("Count: 0")
      );
      for (let i = 1; i <= 3; i++) {
        await clickElement(cdp, findByTestId("ks-counter-inc"), `Snapshot Kitchen Sink Increment #${i}`);
        await waitForCondition(
          cdp,
          `Kitchen Sink incremented to Count: ${i}`,
          `() => (document.getElementById("app")?.innerText || "").includes("Count: ${i}")`
        );
      }
      await waitForCondition(
        cdp,
        "Kitchen Sink snapshot + binary state persisted to localStorage",
        () =>
          (localStorage.getItem("nve:snap:kitchen-sink:v2") || "").startsWith("NVWS1|2|") &&
          (localStorage.getItem("nve:state:kitchen-sink:v2") || "").startsWith("NVB1|2|")
      );

      // Verify returning-user 0ms instant restore even with .wasm BLOCKED!
      allowExpectedOfflineWasmError = true;
      await cdp.send("Network.setBlockedURLs", { urls: ["*.wasm"] });
      try {
        await cdp.send("Page.navigate", { url: kitchenSinkSsgUrl });
        await waitForCondition(
          cdp,
          "Kitchen Sink returning-user 0ms instant snapshot painted Count: 3 with .wasm blocked",
          () => {
            const app = document.getElementById("app");
            const status = document.getElementById("nve-status");
            return (
              app?.dataset.nveRestored === "1" &&
              window.__nveWasmMod === undefined &&
              (app?.innerText || "").includes("Count: 3") &&
              status?.textContent === "Snapshot (offline)"
            );
          }
        );
      } finally {
        await cdp.send("Network.setBlockedURLs", { urls: [] });
        allowExpectedOfflineWasmError = false;
      }

      await cdp.send("Page.navigate", { url: kitchenSinkSsgUrl });
      await waitForCondition(
        cdp,
        "Kitchen Sink hydrated restored Count: 3 snapshot with 0 mismatches",
        () => {
          const ctrl = window.__nveController;
          return (
            Boolean(ctrl) &&
            ctrl.app_name === "KitchenSinkApp" &&
            ctrl.restored_from_snapshot() === true &&
            ctrl.is_hydrated() === true &&
            Number(ctrl.hydration_mismatch_count()) === 0 &&
            (document.getElementById("app")?.innerText || "").includes("Count: 3")
          );
        }
      );
      console.log("    PASS Kitchen Sink 0ms Instant Snapshot + Binary State Restore (Count: 3, mismatches=0)");

      // 8f. `?reset=1` clears localStorage, preserves clean build-time SSG Board HTML with 0-mismatch hydration,
      //     and corrupt storage falls back cleanly with zero panics.
      await cdp.send("Page.navigate", { url: `${studioSsgUrl}&reset=1` });
      await waitForCondition(
        cdp,
        "?reset=1 cleared localStorage and hydrated clean build-time SSG Studio Board with 0 mismatches",
        () => {
          const ctrl = window.__nveController;
          const text = document.getElementById("app")?.innerText || "";
          return (
            ctrl?.app_name === "StudioWorkspaceApp" &&
            ctrl?.restored_from_snapshot() === false &&
            ctrl?.is_hydrated() === true &&
            Number(ctrl?.hydration_mismatch_count()) === 0 &&
            localStorage.getItem("nve:snap:studio:v3") === null &&
            localStorage.getItem("nve:state:studio:v3") === null &&
            text.includes("Issue Board") &&
            text.includes("Done (1)") &&
            !text.includes("Zero-MS Snapshot Task")
          );
        }
      );
      await evalJs(
        cdp,
        `(() => {
          localStorage.setItem('nve:snap:studio:v3', 'NVWS1|3|0000000000000000||0|0|0|15\\n<div data-nve-id="1">corrupt</div>');
          localStorage.setItem('nve:state:studio:v3', 'NVB1|3|!!!CORRUPT!!!');
          return true;
        })()`
      );
      await cdp.send("Page.navigate", { url: studioSsgUrl });
      await waitForCondition(
        cdp,
        "Corrupt localStorage fell back cleanly to default Studio Board with 0 errors",
        () => {
          const text = document.getElementById("app")?.innerText || "";
          return (
            window.__nveController?.app_name === "StudioWorkspaceApp" &&
            window.__nveController?.restored_from_snapshot() === false &&
            text.includes("Issue Board") &&
            text.includes("AET-101")
          );
        }
      );
      console.log("    PASS ?reset=1 clean clear (0-mismatch cold SSG fallback) + corrupt localStorage automatic recovery");
    } finally {
      ssgServer.closeAllConnections?.();
      await new Promise((resolve) => ssgServer.close(() => resolve()));
      fs.rmSync(ssgTmpDir, { recursive: true, force: true });
    }

    // 8g. Host Services (`NVE_IFACE_HOST = 5`) Browser E2E:
    //     - 5 mount/unmount cycles assert 0 leaked Rust & JS closure/listener slab entries
    //     - Live `prefers-color-scheme` dark <-> light transition flushes via `wasm_drain` microtask
    //     - Live DPR change re-arms `(resolution: <dpr>dppx)` watcher
    //     - Live `languagechange` pushes `EVENT_LOCALE_CHANGED` and re-renders Studio with `de-DE`
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 800,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await cdp.send("Emulation.setEmulatedMedia", {
      features: [{ name: "prefers-color-scheme", value: "dark" }],
    });
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=studio&ws=off&bundle=dev` });
    await waitForCondition(
      cdp,
      "dev bundle mounted Studio in Dark mode with host_services_slab_counts probe",
      () =>
        window.__nveBundle === "dev" &&
        typeof window.__nveWasmMod?.host_services_slab_counts === "function" &&
        (document.getElementById("app")?.innerText || "").includes("Dark |")
    );

    // Capture baseline slab counts with 1 mounted primary surface, then mount/unmount a secondary
    // Studio surface 5 times and verify every Rust + JS closure/listener count returns to baseline.
    const baselineSlabs = await evalJs(
      cdp,
      `Array.from(window.__nveWasmMod.host_services_slab_counts())`
    );
    const cycleResult = await evalJs(
      cdp,
      `(() => {
        const mod = window.__nveWasmMod;
        let el = document.getElementById('nve-host-cycle-mount');
        if (!el) {
          el = document.createElement('div');
          el.id = 'nve-host-cycle-mount';
          el.style.cssText = 'position:fixed;left:-9999px;top:0;width:800px;height:600px;';
          document.body.appendChild(el);
        }
        const beforeShells = mod.active_shell_count();
        let peakWatchers = 0;
        for (let i = 0; i < 5; i++) {
          const ctrl = mod.start_studio_workspace_app_on_surface(90 + i, 'nve-host-cycle-mount', null);
          const mid = Array.from(mod.host_services_slab_counts());
          peakWatchers = Math.max(peakWatchers, mid[7]);
          ctrl.unmount();
          ctrl.free();
        }
        el.remove();
        const afterShells = mod.active_shell_count();
        const afterSlabs = Array.from(mod.host_services_slab_counts());
        return { beforeShells, afterShells, peakWatchers, afterSlabs };
      })()`
    );
    if (
      cycleResult.afterShells !== cycleResult.beforeShells ||
      cycleResult.peakWatchers < baselineSlabs[7] + 1 ||
      JSON.stringify(cycleResult.afterSlabs) !== JSON.stringify(baselineSlabs)
    ) {
      throw new Error(
        `Host services closure/listener leak after 5 mount/unmount cycles: baseline=${JSON.stringify(
          baselineSlabs
        )}, got=${JSON.stringify(cycleResult)}`
      );
    }
    console.log(
      `    PASS Host Services 5x mount/unmount closure/listener cleanup (baseline=${JSON.stringify(
        baselineSlabs
      )}, after=${JSON.stringify(cycleResult.afterSlabs)})`
    );

    // Remounting on an existing surface_id while JS still holds ctrl1 (before ctrl1.free()) MUST
    // unbind and drop old WebClipboard DOM listeners immediately via teardown_active_shell().
    const remountResult = await evalJs(
      cdp,
      `(() => {
        const mod = window.__nveWasmMod;
        const el = document.createElement('div');
        el.id = 'nve-host-remount-test';
        el.style.cssText = 'position:fixed;left:-9999px;top:0;width:640px;height:480px;';
        document.body.appendChild(el);
        const ctrl1 = mod.start_studio_workspace_app_on_surface(95, 'nve-host-remount-test', null);
        const afterFirst = Array.from(mod.host_services_slab_counts());
        // Replace surface 95 BEFORE ctrl1.free()!
        const ctrl2 = mod.start_studio_workspace_app_on_surface(95, 'nve-host-remount-test', null);
        const afterReplace = Array.from(mod.host_services_slab_counts());
        ctrl2.unmount();
        ctrl2.free();
        ctrl1.free();
        el.remove();
        const afterAll = Array.from(mod.host_services_slab_counts());
        return { afterFirst, afterReplace, afterAll };
      })()`
    );
    if (
      remountResult.afterReplace[4] !== remountResult.afterFirst[4] ||
      remountResult.afterReplace[6] !== remountResult.afterFirst[6] ||
      JSON.stringify(remountResult.afterAll) !== JSON.stringify(baselineSlabs)
    ) {
      throw new Error(
        `Remount on same surface_id leaked DOM listeners before ctrl1.free(): baseline=${JSON.stringify(
          baselineSlabs
        )}, got=${JSON.stringify(remountResult)}`
      );
    }
    console.log("    PASS Host Services same-surface remount immediate DOM listener cleanup");

    // Real WASM Pending navigator.clipboard.readText() UAF + automatic deferred-reap E2E:
    //   (a) Start Pending read -> unmount BEFORE Promise settles -> settle Promise -> verify 0 errors & baseline slabs
    //   (b) Start Pending read -> arm synchronous re-entrant unmount inside on_ready -> settle Promise
    //       WHILE Rust Closure is on WASM call stack -> wait microtasks -> verify automatic reap & 0 UAF errors!
    const asyncClipResult = await evalJs(
      cdp,
      `(async () => {
        const mod = window.__nveWasmMod;
        const origClip = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
        let pendingResolvers = [];
        Object.defineProperty(navigator, 'clipboard', {
          configurable: true,
          value: {
            readText: () => new Promise((resolve) => { pendingResolvers.push(resolve); }),
            writeText: () => Promise.resolve(),
          },
        });
        try {
          const el = document.createElement('div');
          el.id = 'nve-host-clip-async';
          el.style.cssText = 'position:fixed;left:-9999px;top:0;width:640px;height:480px;';
          document.body.appendChild(el);

          // (a) Unmount BEFORE Promise resolves
          const c1 = mod.start_studio_workspace_app_on_surface(98, 'nve-host-clip-async', null);
          const req1 = mod.host_services_start_clipboard_read(98);
          const mid1 = Array.from(mod.host_services_slab_counts());
          c1.unmount();
          c1.free();
          pendingResolvers.shift()('settled-after-unmount');
          await Promise.resolve();
          await Promise.resolve();
          await new Promise((r) => setTimeout(r, 0));
          const after1 = Array.from(mod.host_services_slab_counts());

          // (b) Re-entrant unmount INSIDE live Promise settlement callback (WASM UAF + auto-reap guard!)
          const c2 = mod.start_studio_workspace_app_on_surface(99, 'nve-host-clip-async', null);
          const armed = mod.host_services_arm_reentrant_unmount(99);
          const req2 = mod.host_services_start_clipboard_read(99);
          const mid2 = Array.from(mod.host_services_slab_counts());
          pendingResolvers.shift()('settled-with-reentrant-unmount');
          // Wait microtasks + macrotask for automatic deferred reap (debug_slab_counts does NOT reap!)
          await Promise.resolve();
          await Promise.resolve();
          await new Promise((r) => setTimeout(r, 0));
          c2.free();
          el.remove();
          const after2 = Array.from(mod.host_services_slab_counts());
          return { req1: Number(req1), mid1, after1, armed, req2: Number(req2), mid2, after2 };
        } finally {
          if (origClip) {
            Object.defineProperty(navigator, 'clipboard', origClip);
          } else {
            delete navigator.clipboard;
          }
        }
      })()`
    );
    if (
      asyncClipResult.req1 === 0 ||
      asyncClipResult.mid1[3] < 1 ||
      asyncClipResult.mid1[5] < 1 ||
      JSON.stringify(asyncClipResult.after1) !== JSON.stringify(baselineSlabs) ||
      !asyncClipResult.armed ||
      asyncClipResult.req2 === 0 ||
      asyncClipResult.mid2[3] < 1 ||
      asyncClipResult.mid2[5] < 1 ||
      JSON.stringify(asyncClipResult.after2) !== JSON.stringify(baselineSlabs)
    ) {
      throw new Error(
        `Async clipboard Pending / re-entrant WASM unmount failed: baseline=${JSON.stringify(
          baselineSlabs
        )}, got=${JSON.stringify(asyncClipResult)}`
      );
    }
    console.log(
      "    PASS Host Services real WASM Pending clipboard unmount & re-entrant callback auto-reap (0 UAF)"
    );

    // Live WASM `NotAllowedError` (must return PermissionDenied = -3, NEVER stale shadow) AND
    // missing `navigator.clipboard` fallback (must synchronously return shadow text).
    const clipErrFallbackResult = await evalJs(
      cdp,
      `(async () => {
        const mod = window.__nveWasmMod;
        const origClip = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
        const el = document.createElement('div');
        el.id = 'nve-host-clip-err';
        el.style.cssText = 'position:fixed;left:-9999px;top:0;width:640px;height:480px;';
        document.body.appendChild(el);
        try {
          // (a) NotAllowedError with populated shadow buffer MUST resolve PermissionDenied (-3).
          Object.defineProperty(navigator, 'clipboard', {
            configurable: true,
            value: {
              readText: () => Promise.reject(new DOMException('denied', 'NotAllowedError')),
              writeText: () => Promise.resolve(),
            },
          });
          const cDeny = mod.start_studio_workspace_app_on_surface(94, 'nve-host-clip-err', null);
          const seededDeny = mod.host_services_set_clipboard_shadow(94, 'stale-secret-must-not-leak');
          const probeDeny = mod.host_services_clipboard_read_probe(94);
          let denyRes = 'not-pending';
          if (probeDeny.startsWith('pending:')) {
            const reqId = BigInt(probeDeny.slice('pending:'.length));
            await Promise.resolve();
            await Promise.resolve();
            await new Promise((r) => setTimeout(r, 0));
            denyRes = mod.host_services_take_clipboard_result(94, reqId);
          }
          cDeny.unmount();
          cDeny.free();

          // (b) Missing navigator.clipboard falls back synchronously to shadow text.
          Object.defineProperty(navigator, 'clipboard', {
            configurable: true,
            value: undefined,
          });
          const cMiss = mod.start_studio_workspace_app_on_surface(93, 'nve-host-clip-err', null);
          const seededMiss = mod.host_services_set_clipboard_shadow(93, 'shadow-fallback-ok');
          const probeMiss = mod.host_services_clipboard_read_probe(93);
          cMiss.unmount();
          cMiss.free();

          await Promise.resolve();
          await new Promise((r) => setTimeout(r, 0));
          const afterSlabs = Array.from(mod.host_services_slab_counts());
          return { seededDeny, probeDeny, denyRes, seededMiss, probeMiss, afterSlabs };
        } finally {
          el.remove();
          if (origClip) {
            Object.defineProperty(navigator, 'clipboard', origClip);
          } else {
            delete navigator.clipboard;
          }
        }
      })()`
    );
    if (
      !clipErrFallbackResult.seededDeny ||
      !clipErrFallbackResult.probeDeny.startsWith("pending:") ||
      clipErrFallbackResult.denyRes !== "err:-3" ||
      !clipErrFallbackResult.seededMiss ||
      clipErrFallbackResult.probeMiss !== "sync:shadow-fallback-ok" ||
      JSON.stringify(clipErrFallbackResult.afterSlabs) !== JSON.stringify(baselineSlabs)
    ) {
      throw new Error(
        `WASM clipboard NotAllowedError / missing navigator.clipboard fallback failed: baseline=${JSON.stringify(
          baselineSlabs
        )}, got=${JSON.stringify(clipErrFallbackResult)}`
      );
    }
    console.log(
      "    PASS Host Services live WASM NotAllowedError -> PermissionDenied (-3) & missing navigator.clipboard shadow fallback"
    );

    // Live prefers-color-scheme Dark -> Light transition via CDP Emulation.setEmulatedMedia.
    await cdp.send("Emulation.setEmulatedMedia", {
      features: [{ name: "prefers-color-scheme", value: "light" }],
    });
    await waitForCondition(
      cdp,
      "Studio re-rendered to Light mode via wasm_drain microtask on prefers-color-scheme change",
      () => (document.getElementById("app")?.innerText || "").includes("Light |")
    );
    await cdp.send("Emulation.setEmulatedMedia", {
      features: [{ name: "prefers-color-scheme", value: "dark" }],
    });
    await waitForCondition(
      cdp,
      "Studio re-rendered back to Dark mode via wasm_drain microtask",
      () => (document.getElementById("app")?.innerText || "").includes("Dark |")
    );

    // Live DPR matchMedia re-arming test WITHOUT any `resize` event:
    // Wraps `window.matchMedia` to capture each `(resolution: <dpr>dppx)` MediaQueryList created
    // by `WebThemeWatcher`, then dispatches real `MediaQueryListEvent('change')` across 1x -> 2x -> 3x
    // while overriding `window.devicePixelRatio`. Proves:
    //   1. `onDprMqlChange` fires with zero `resize` events (`dpr_mql_fires` increments +2),
    //   2. `armDpr()` creates a fresh `(resolution: 2dppx)` then `(resolution: 3dppx)` query,
    //   3. old MQL listeners are detached on re-arm (re-firing stale MQL does NOT increment),
    //   4. unmount detaches the final MQL listener and leaves zero leaked slab entries.
    const dprMqlResult = await evalJs(
      cdp,
      `(async () => {
        const mod = window.__nveWasmMod;
        const origMatchMedia = window.matchMedia.bind(window);
        const origDprDesc = Object.getOwnPropertyDescriptor(window, 'devicePixelRatio');
        const captured = [];
        window.matchMedia = (q) => {
          const mql = origMatchMedia(q);
          if (typeof q === 'string' && q.includes('dppx')) {
            captured.push({ query: q, mql });
          }
          return mql;
        };
        const setDpr = (v) => {
          Object.defineProperty(window, 'devicePixelRatio', {
            configurable: true,
            get: () => v,
          });
        };
        try {
          setDpr(1);
          const el = document.createElement('div');
          el.id = 'nve-host-dpr-mql';
          el.style.cssText = 'position:fixed;left:-9999px;top:0;width:640px;height:480px;';
          document.body.appendChild(el);

          const startFires = mod.host_services_dpr_mql_fires();
          const ctrl = mod.start_studio_workspace_app_on_surface(97, 'nve-host-dpr-mql', null);
          const dpr0 = mod.host_services_cached_dpr_x100(97);
          const q0 = captured[0]?.query || null;

          // 1x -> 2x via MediaQueryListEvent('change') with ZERO resize event.
          setDpr(2);
          captured[0].mql.dispatchEvent(new MediaQueryListEvent('change', { matches: false, media: q0 }));
          const fires1 = mod.host_services_dpr_mql_fires() - startFires;
          const dpr1 = mod.host_services_cached_dpr_x100(97);
          const q1 = captured[1]?.query || null;

          // Stale mql0 must already be detached by armDpr().
          captured[0].mql.dispatchEvent(new MediaQueryListEvent('change', { matches: false, media: q0 }));
          const firesAfterStale = mod.host_services_dpr_mql_fires() - startFires;

          // 2x -> 3x via re-armed mql1 with ZERO resize event.
          setDpr(3);
          captured[1].mql.dispatchEvent(new MediaQueryListEvent('change', { matches: false, media: q1 }));
          const fires2 = mod.host_services_dpr_mql_fires() - startFires;
          const dpr2 = mod.host_services_cached_dpr_x100(97);
          const q2 = captured[2]?.query || null;

          ctrl.unmount();
          ctrl.free();
          el.remove();

          // Post-unmount dispatch on mql2 must be detached (no fire, no error).
          captured[2].mql.dispatchEvent(new MediaQueryListEvent('change', { matches: false, media: q2 }));
          const firesAfterUnmount = mod.host_services_dpr_mql_fires() - startFires;
          await Promise.resolve();
          const afterSlabs = Array.from(mod.host_services_slab_counts());
          return { dpr0, q0, fires1, dpr1, q1, firesAfterStale, fires2, dpr2, q2, firesAfterUnmount, afterSlabs };
        } finally {
          window.matchMedia = origMatchMedia;
          if (origDprDesc) {
            Object.defineProperty(window, 'devicePixelRatio', origDprDesc);
          } else {
            delete window.devicePixelRatio;
          }
        }
      })()`
    );
    if (
      dprMqlResult.dpr0 !== 100 ||
      dprMqlResult.q0 !== "(resolution: 1dppx)" ||
      dprMqlResult.fires1 !== 1 ||
      dprMqlResult.dpr1 !== 200 ||
      dprMqlResult.q1 !== "(resolution: 2dppx)" ||
      dprMqlResult.firesAfterStale !== 1 ||
      dprMqlResult.fires2 !== 2 ||
      dprMqlResult.dpr2 !== 300 ||
      dprMqlResult.q2 !== "(resolution: 3dppx)" ||
      dprMqlResult.firesAfterUnmount !== 2 ||
      JSON.stringify(dprMqlResult.afterSlabs) !== JSON.stringify(baselineSlabs)
    ) {
      throw new Error(
        `DPR matchMedia re-arming / stale-listener detachment failed: baseline=${JSON.stringify(
          baselineSlabs
        )}, got=${JSON.stringify(dprMqlResult)}`
      );
    }

    // Also verify real CDP Emulation.setDeviceMetricsOverride exercises the live `resize` DPR
    // fallback on surface 1 (Blink CDP does not fire MediaQueryList.change when only
    // deviceScaleFactor changes between identical-size overrides).
    await waitForCondition(
      cdp,
      "WebSystemEnv cached scale_factor_x100 starts at 100 on 1x DPR",
      () => window.__nveWasmMod.host_services_cached_dpr_x100(1) === 100
    );
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 801,
      deviceScaleFactor: 2,
      mobile: false,
    });
    await waitForCondition(
      cdp,
      "WebSystemEnv cached scale_factor_x100 updated to 200 on CDP DPR change (1x -> 2x)",
      () => window.__nveWasmMod.host_services_cached_dpr_x100(1) === 200
    );
    await cdp.send("Emulation.clearDeviceMetricsOverride");

    // Live languagechange dispatches EVENT_LOCALE_CHANGED and updates Studio DOM to de-DE.
    await evalJs(
      cdp,
      `(() => {
        Object.defineProperty(navigator, 'language', { value: 'de-DE', configurable: true });
        window.dispatchEvent(new Event('languagechange'));
        return navigator.language;
      })()`
    );
    await waitForCondition(
      cdp,
      "Studio re-rendered with de-DE locale via EVENT_LOCALE_CHANGED push",
      () => (document.getElementById("app")?.innerText || "").includes("Dark | de-DE")
    );
    console.log(
      "    PASS Host Services live prefers-color-scheme, DPR re-arm, and languagechange E2E via wasm_drain"
    );

    if (errors.length > 0) {
      throw new Error(
        `Browser console/runtime errors detected (${errors.length}):\n${errors.join("\n")}`
      );
    }

    // 7e. A real Rust panic prints its message and file:line (checked last: the trap poisons the
    //     instance). The trigger exists only in the dev bundle.
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=studio&ws=off&bundle=dev` });
    await waitForCondition(
      cdp,
      "dev bundle remounted for deliberate panic probe",
      () =>
        window.__nveBundle === "dev" &&
        typeof window.__nveWasmMod?.panic_probe === "function"
    );
    const panicLine = lineOf(path.join(WEB_SHELL_DIR, "src/wasm_probe.rs"), 'panic!("{message}")');
    const logsBefore = consoleLogs.length;
    const panicOutcome = await evalJs(
      cdp,
      `(() => { try { window.__nveWasmMod.panic_probe('nve-e2e-panic-probe'); return 'returned'; } catch (e) { return String(e); } })()`
    );
    const panicLogs = consoleLogs.slice(logsBefore).filter((l) => l.includes("[NVE Web Panic]"));
    const location = `hosts/web-shell/src/wasm_probe.rs:${panicLine}:`;
    if (
      panicLogs.length !== 1 ||
      !panicLogs[0].includes("nve-e2e-panic-probe") ||
      !panicLogs[0].includes(location) ||
      !/RuntimeError|unreachable/.test(panicOutcome)
    ) {
      throw new Error(
        `Panic probe: expected one "[NVE Web Panic]" log with the message and ${location}, then a trap; got ${JSON.stringify({ panicLogs, panicOutcome })}`
      );
    }
    console.log(`    PASS Rust panic message + location in the console (${location}) in the ${devBuild ? "debug" : "release"} dev bundle`);

    console.log(`==> ALL 8 WEB WASM + LIVE PAIR + SSR/HYDRATION + 0MS SNAPSHOT E2E SUITES PASSED (0 console errors and 0 WASM panics before the deliberate panic probe)`);
  } finally {
    await cleanup();
  }
}

main().catch((err) => {
  console.error("E2E FAILED:", err?.stack || err);
  process.exit(1);
});
