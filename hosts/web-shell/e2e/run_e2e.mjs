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
const BUNDLES = ["studio", "counter", "dev"];

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
      viewportMode: document.querySelector('[testid="workspace-root"]')?.getAttribute('viewport-mode'),
      hasMobileTopBar: Boolean(document.querySelector('[testid="mobile-top-bar"]')),
      hasDesktopSidebar: Boolean(document.querySelector('[testid="desktop-sidebar"]')),
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
  const counterDylib = path.join(targetDir, "debug", `${libPrefix}nve_example_counter.${libExt}`);
  return { nveBin, counterDylib };
}

function ensurePairArtifacts() {
  const { nveBin, counterDylib } = resolvePairArtifacts();
  if (fs.existsSync(nveBin) && fs.existsSync(counterDylib)) {
    return { nveBin, counterDylib };
  }
  console.log("==> Building nve-cli + nve-example-counter for live pair E2E...");
  const res = spawnSync("cargo", ["build", "-p", "nve-example-counter", "-p", "nve-cli"], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });
  if (res.status !== 0) {
    throw new Error(`cargo build -p nve-example-counter -p nve-cli failed with exit code ${res.status}`);
  }
  if (!fs.existsSync(nveBin) || !fs.existsSync(counterDylib)) {
    throw new Error(
      `Expected live pair artifacts after build:\n  nveBin=${nveBin} (exists=${fs.existsSync(nveBin)})\n  counterDylib=${counterDylib} (exists=${fs.existsSync(counterDylib)})`
    );
  }
  return { nveBin, counterDylib };
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
        /^NVE_PAIR_READY\s+ws=(\S+)\s+token=(\S+)\s+udp=(\S+)\s+tcp=(\S+)\s*$/m
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
      "production studio bundle loaded (studio exports only: no counter, dev probe or scene view)",
      () => {
        const m = window.__nveWasmMod;
        return (
          window.__nveBundle === "studio" &&
          typeof m?.start_studio_workspace_app === "function" &&
          m.start_counter_app === undefined &&
          m.run_semantic_style_hydration_probe === undefined &&
          m.run_hostile_frame_probe === undefined &&
          m.panic_probe === undefined &&
          m.nve_web_scene_view_ptr === undefined
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
        const img = document.querySelector('img[testid="task-avatar-1"]');
        if (!img) return false;
        const style = getComputedStyle(img);
        return img.complete && img.naturalWidth > 0 && style.objectFit === "cover" && Boolean(img.alt);
      }
    );
    const avatarInfo = await evalJs(
      cdp,
      `(() => {
        const img = document.querySelector('img[testid="task-avatar-1"]');
        return { naturalWidth: img.naturalWidth, naturalHeight: img.naturalHeight, objectFit: getComputedStyle(img).objectFit, alt: img.alt };
      })()`
    );
    console.log(`    PASS Image avatar <img> (${avatarInfo.naturalWidth}x${avatarInfo.naturalHeight}, object-fit=${avatarInfo.objectFit}, alt=${avatarInfo.alt})`);

    // 1b. Board TextInput live search filter + clear
    await typeIntoInput(
      cdp,
      `document.querySelector('[testid="board-search-input"]')`,
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
      `document.querySelector('[testid="board-search-clear"]')`,
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
      `document.querySelector('[testid="board-new-task-input"]')`,
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
      `document.querySelector('[testid="task-status-1"]')`,
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
      () => Boolean(document.querySelector('[testid="command-palette"]'))
    );
    await typeIntoInput(
      cdp,
      `document.querySelector('[testid="palette-input"]')`,
      "Accent:",
      "palette-input"
    );
    await waitForCondition(
      cdp,
      "Command palette filtered to Accent options",
      () => (document.querySelector('[testid="command-palette"]')?.innerText || "").includes("Violet")
    );
    await pressKey(cdp, { key: "ArrowDown", code: "ArrowDown", windowsVirtualKeyCode: 40 });
    await pressKey(cdp, { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await waitForCondition(
      cdp,
      "Command palette ArrowDown + Enter applied Accent -> Violet and closed",
      () => {
        const pal = document.querySelector('[testid="command-palette"]');
        const text = document.getElementById("app")?.innerText || "";
        return !pal && text.includes("Accent -> Violet");
      }
    );
    // Re-open with Ctrl+K and close with Escape
    await pressKey(cdp, { key: "k", code: "KeyK", windowsVirtualKeyCode: 75, modifiers: 2 });
    await waitForCondition(
      cdp,
      "Ctrl+K re-opened command palette",
      () => Boolean(document.querySelector('[testid="command-palette"]'))
    );
    await pressKey(cdp, { key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await waitForCondition(
      cdp,
      "Escape closed command palette",
      () => !document.querySelector('[testid="command-palette"]')
    );
    console.log("    PASS Cmd+K / Ctrl+K palette (open, filter, ArrowDown, Enter, Escape)");

    // 1f. `# engineering` channel view + chat composer TextInput Enter submit
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
        return text.includes("Single ABI drives all 4 hosts!") && text.includes("QUICK REPLY");
      }
    );
    await typeIntoInput(
      cdp,
      `document.querySelector('[testid="chat-composer-input"]')`,
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
    await clickElement(cdp, `document.querySelector('[testid="nav-islands"]')`, "nav-islands");
    await waitForCondition(
      cdp,
      "Islands view rendered",
      () => Boolean(document.querySelector('[testid="islands-summary"]'))
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="island-bus-theme"]')`,
      "island-bus-theme"
    );
    await waitForCondition(
      cdp,
      "Islands NveChannelBus theme broadcast recorded",
      () => (document.querySelector('[testid="islands-summary"]')?.innerText || "").includes("Bus: 1 tx")
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="island-gesture-claim"]')`,
      "island-gesture-claim"
    );
    await waitForCondition(
      cdp,
      "Islands GestureArbitrator Claim recorded",
      () => (document.querySelector('[testid="islands-gesture-readout"]')?.innerText || "").includes("Claim | winner:")
    );
    console.log("    PASS Islands view (NveChannelBus broadcast + GestureArbitrator Claim)");

    // 1h. 100k Stream view jump #50k and #99,999
    await clickElement(cdp, `document.querySelector('[testid="nav-stream"]')`, "nav-stream");
    await waitForCondition(
      cdp,
      "100k Stream view rendered",
      () => Boolean(document.querySelector('[testid="stream-viewport"]'))
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="stream-jump-50k"]')`,
      "stream-jump-50k"
    );
    await waitForCondition(
      cdp,
      "100k Stream jumped to #50k (STR-50000)",
      () => (document.getElementById("app")?.innerText || "").includes("STR-50000")
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="stream-jump-end"]')`,
      "stream-jump-end"
    );
    await waitForCondition(
      cdp,
      "100k Stream jumped to #99,999 (STR-99999)",
      () => (document.getElementById("app")?.innerText || "").includes("STR-99999")
    );
    console.log("    PASS 100k Stream view (#50k -> STR-50000, #99,999 -> STR-99999)");

    // 1i. Metrics view `<canvas>` NVE_DRAW1 non-blank painted pixel verification
    await clickElement(cdp, `document.querySelector('[testid="nav-metrics"]')`, "nav-metrics");
    await waitForCondition(
      cdp,
      "Metrics <canvas> rendered with NVE_DRAW1 draw ops",
      () => {
        const c = document.querySelector('canvas[testid="metrics-sparkline-canvas"]');
        return Boolean(c && c.width > 0 && c.height > 0 && Number(c.getAttribute("data-nve-draw-ops")) > 0);
      }
    );
    const canvasCheck = await evalJs(
      cdp,
      `(() => {
        const c = document.querySelector('canvas[testid="metrics-sparkline-canvas"]');
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
    await clickElement(cdp, `document.querySelector('[testid="toggle-xray"]')`, "toggle-xray");
    await waitForCondition(
      cdp,
      "X-Ray HUD toggled ON",
      () => (document.getElementById("app")?.innerText || "").includes("X-Ray HUD: ON")
    );
    await clickElement(cdp, `document.querySelector('[testid="tt-undo"]')`, "tt-undo");
    await waitForCondition(
      cdp,
      "Time-travel Undo restored X-Ray HUD: OFF",
      () => (document.getElementById("app")?.innerText || "").includes("X-Ray HUD: OFF")
    );
    await clickElement(cdp, `document.querySelector('[testid="tt-first"]')`, "tt-first");
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
        const topBar = document.querySelector('[testid="mobile-top-bar"]');
        const desktopSidebar = document.querySelector('[testid="desktop-sidebar"]');
        const center = document.querySelector('[testid="center-workspace"]');
        if (!topBar || desktopSidebar || !center) return false;
        const tbRect = topBar.getBoundingClientRect();
        const cwRect = center.getBoundingClientRect();
        return tbRect.width > 0 && tbRect.height > 0 && cwRect.width > 300 && cwRect.left >= 0 && cwRect.right <= 395;
      }
    );
    const mobileMetrics = await evalJs(
      cdp,
      `(() => {
        const tb = document.querySelector('[testid="mobile-top-bar"]').getBoundingClientRect();
        const cw = document.querySelector('[testid="center-workspace"]').getBoundingClientRect();
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
        Boolean(document.querySelector('[testid="desktop-sidebar"]')) &&
        !document.querySelector('[testid="mobile-top-bar"]')
    );
    console.log(
      `    PASS Mobile Portrait Resize (393x852 mobile-top-bar=${mobileMetrics.topBar.w}x${mobileMetrics.topBar.h}, center=${mobileMetrics.center.w}x${mobileMetrics.center.h}, restored desktop)`
    );

    // =========================================================================
    // 3. Counter App (`/?app=counter&ws=off`)
    // =========================================================================
    console.log("==> [3/7] Testing Counter App (?app=counter&ws=off)...");
    await cdp.send("Page.navigate", { url: `${baseUrl}/?app=counter&ws=off` });
    await waitForCondition(
      cdp,
      "production counter bundle loaded (counter exports only: no studio, dev probe or scene view)",
      () => {
        const m = window.__nveWasmMod;
        return (
          window.__nveBundle === "counter" &&
          typeof m?.start_counter_app === "function" &&
          m.start_studio_workspace_app === undefined &&
          m.run_semantic_style_hydration_probe === undefined &&
          m.run_hostile_frame_probe === undefined &&
          m.panic_probe === undefined &&
          m.nve_web_scene_view_ptr === undefined
        );
      }
    );

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
        Boolean(document.querySelector('[testid="nav-docs"]')) &&
        Boolean(document.querySelector('[testid="nav-workbench"]'))
    );

    await clickElement(cdp, `document.querySelector('[testid="nav-docs"]')`, "nav-docs");
    await waitForCondition(
      cdp,
      "Docs view rendered seeded RFC-042 and Release Checklist docs",
      () => {
        const hdr = document.querySelector('[testid="docs-header"]')?.innerText || "";
        const list = document.querySelector('[testid="docs-blocks-list"]')?.innerText || "";
        return (
          Boolean(document.querySelector('[testid="doc-tab-1"]')) &&
          Boolean(document.querySelector('[testid="doc-tab-2"]')) &&
          hdr.includes("7 blocks | 1/2 checks") &&
          list.includes("RFC-042: Zero-Copy NveSceneView") &&
          list.includes("Enable crash-safe NVE_SNAP + .nvewal local persistence in Studio")
        );
      }
    );

    // Switch to second seeded doc and back to RFC-042.
    await clickElement(cdp, `document.querySelector('[testid="doc-tab-2"]')`, "doc-tab-2");
    await waitForCondition(
      cdp,
      "Switched to Release Checklist: v1.0 seeded doc",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        return text.includes("Pre-Flight Gates") && text.includes("8 blocks | 2/3 checks");
      }
    );
    await clickElement(cdp, `document.querySelector('[testid="doc-tab-1"]')`, "doc-tab-1");
    await waitForCondition(
      cdp,
      "Switched back to RFC-042 seeded doc",
      () => (document.querySelector('[testid="docs-header"]')?.innerText || "").includes("7 blocks | 1/2 checks")
    );

    // Add a new block via the Docs composer.
    const addedBlockDraft = "Stream binary NVD1 deltas over WebSocket";
    await typeIntoInput(
      cdp,
      `document.querySelector('[testid="doc-new-block-input"]')`,
      addedBlockDraft,
      "doc-new-block-input"
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="doc-new-block-submit"]')`,
      "doc-new-block-submit"
    );
    await waitForCondition(
      cdp,
      "New block #16 appended to RFC-042 doc",
      () => {
        const hdr = document.querySelector('[testid="docs-header"]')?.innerText || "";
        const blk = document.querySelector('[testid="doc-block-16"]')?.innerText || "";
        return hdr.includes("8 blocks | 1/2 checks") && blk.includes("Stream binary NVD1 deltas over WebSocket");
      }
    );

    // Edit block #16 inline text and save.
    const editedBlockText = "Verified zero-JSON binary NVD1 WebSocket streaming";
    await clickElement(
      cdp,
      `document.querySelector('[testid="doc-block-edit-16"]')`,
      "doc-block-edit-16"
    );
    await waitForCondition(
      cdp,
      "Inline editor opened for block #16",
      () => Boolean(document.querySelector('[testid="doc-block-input-16"]'))
    );
    await typeIntoInput(
      cdp,
      `document.querySelector('[testid="doc-block-input-16"]')`,
      editedBlockText,
      "doc-block-input-16"
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="doc-block-save-16"]')`,
      "doc-block-save-16"
    );
    await waitForCondition(
      cdp,
      "Edited block #16 saved and inline input closed",
      () =>
        !document.querySelector('[testid="doc-block-input-16"]') &&
        (document.querySelector('[testid="doc-block-16"]')?.innerText || "").includes(
          "Verified zero-JSON binary NVD1 WebSocket streaming"
        )
    );

    // Toggle unchecked checklist item #7 -> [x] (2/2 checks).
    await clickElement(cdp, `document.querySelector('[testid="doc-check-7"]')`, "doc-check-7");
    await waitForCondition(
      cdp,
      "Checklist item #7 toggled to [x] (2/2 checks)",
      () => {
        const hdr = document.querySelector('[testid="docs-header"]')?.innerText || "";
        const chk = document.querySelector('[testid="doc-check-7"]')?.innerText || "";
        return chk.includes("[x]") && hdr.includes("2/2 checks");
      }
    );

    // Reorder block #16 up above block #7 and verify DOM order changed.
    const orderBefore = await evalJs(
      cdp,
      `Array.from(document.querySelectorAll('[testid="docs-blocks-list"] [testid^="doc-block-"]'))
        .map((el) => el.getAttribute('testid'))
        .filter((id) => /^doc-block-\\d+$/.test(id))`
    );
    if (!(orderBefore.indexOf("doc-block-7") >= 0 && orderBefore.indexOf("doc-block-7") < orderBefore.indexOf("doc-block-16"))) {
      throw new Error(`Unexpected block order before reorder: ${JSON.stringify(orderBefore)}`);
    }
    await clickElement(
      cdp,
      `document.querySelector('[testid="doc-block-up-16"]')`,
      "doc-block-up-16"
    );
    await waitForCondition(
      cdp,
      "Block #16 reordered above block #7",
      () => {
        const ids = Array.from(
          document.querySelectorAll('[testid="docs-blocks-list"] [testid^="doc-block-"]')
        )
          .map((el) => el.getAttribute("testid"))
          .filter((id) => /^doc-block-\d+$/.test(id));
        const idx16 = ids.indexOf("doc-block-16");
        const idx7 = ids.indexOf("doc-block-7");
        return idx16 >= 0 && idx7 >= 0 && idx16 < idx7;
      }
    );

    // Export Markdown and assert full structure, checked item, edited text, and reordered sequence.
    await clickElement(cdp, `document.querySelector('[testid="doc-export-md"]')`, "doc-export-md");
    await waitForCondition(
      cdp,
      "Markdown export preview rendered with edited & reordered blocks",
      () => {
        const md = document.querySelector('[testid="doc-markdown-preview"]')?.innerText || "";
        return (
          md.includes("MARKDOWN EXPORT") &&
          md.includes("# RFC-042: Zero-Copy NveSceneView") &&
          md.includes("- [x] Enable crash-safe NVE_SNAP + .nvewal local persistence in Studio") &&
          md.includes("Verified zero-JSON binary NVD1 WebSocket streaming")
        );
      }
    );
    const mdText = await evalJs(
      cdp,
      `document.querySelector('[testid="doc-markdown-preview"]')?.innerText || ''`
    );
    const editedPos = mdText.indexOf(editedBlockText);
    const checkPos = mdText.indexOf("- [x] Enable crash-safe NVE_SNAP");
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
      `document.querySelector('[testid="nav-workbench"]')`,
      "nav-workbench"
    );
    await waitForCondition(
      cdp,
      "Workbench view rendered with 3 viewport presets and 9 connected host cards",
      () => {
        const grid = document.querySelector('[testid="wb-viewport-grid"]');
        const hosts = document.querySelectorAll('[testid^="wb-host-"]');
        return (
          grid?.getAttribute("workbench-preset") === "all" &&
          Boolean(document.querySelector('[testid="wb-frame-desktop"]')) &&
          Boolean(document.querySelector('[testid="wb-frame-tablet"]')) &&
          Boolean(document.querySelector('[testid="wb-frame-mobile"]')) &&
          Boolean(document.querySelector('[testid="wb-telemetry-banner"]')) &&
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
            id: el.getAttribute('testid'),
            left: Math.round(r.left),
            top: Math.round(r.top),
            right: Math.round(r.right),
            bottom: Math.round(r.bottom),
            w: Math.round(r.width),
            h: Math.round(r.height),
          };
        };
        const frames = ['wb-frame-desktop', 'wb-frame-tablet', 'wb-frame-mobile']
          .map((id) => readRect(document.querySelector('[testid="' + id + '"]')));
        const hosts = Array.from(document.querySelectorAll('[testid^="wb-host-"]')).map(readRect);
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
      await clickElement(cdp, `document.querySelector('[testid="${btnId}"]')`, btnId);
      await waitForCondition(
        cdp,
        `Workbench preset switched to ${presetName}`,
        `() => {
          const grid = document.querySelector('[testid="wb-viewport-grid"]');
          const active = document.querySelector('[testid="${expectedFrame}"]');
          const absentOk = ${JSON.stringify(absentFrames)}.every(
            (id) => !document.querySelector('[testid="' + id + '"]')
          );
          if (!grid || !active || !absentOk) return false;
          const r = active.getBoundingClientRect();
          return grid.getAttribute('workbench-preset') === ${JSON.stringify(presetName)} && r.width > 120 && r.height > 60;
        }`
      );
    }

    await clickElement(cdp, `document.querySelector('[testid="wb-preset-all"]')`, "wb-preset-all");
    await waitForCondition(
      cdp,
      "Workbench preset restored to all (3) frames",
      () =>
        document.querySelector('[testid="wb-viewport-grid"]')?.getAttribute("workbench-preset") ===
          "all" &&
        Boolean(document.querySelector('[testid="wb-frame-desktop"]')) &&
        Boolean(document.querySelector('[testid="wb-frame-tablet"]')) &&
        Boolean(document.querySelector('[testid="wb-frame-mobile"]'))
    );

    // Toggle device chrome off/on and select a host card to verify interactivity.
    await clickElement(
      cdp,
      `document.querySelector('[testid="wb-toggle-chrome"]')`,
      "wb-toggle-chrome (off)"
    );
    await waitForCondition(
      cdp,
      "Workbench device chrome toggled off",
      () => !(document.querySelector('[testid="wb-frame-desktop"]')?.innerText || "").includes("1280x800")
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="wb-toggle-chrome"]')`,
      "wb-toggle-chrome (on)"
    );
    await waitForCondition(
      cdp,
      "Workbench device chrome restored",
      () => (document.querySelector('[testid="wb-frame-desktop"]')?.innerText || "").includes("1280x800")
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid^="wb-host-"]')`,
      "workbench first host card"
    );
    console.log(
      `    PASS Studio Workbench View (presets all/desktop/tablet/mobile, 3 frames + 9 host cards non-collapsed & non-overlapping)`
    );

    // =========================================================================
    // 6. Live `nve pair` Binary WebSocket Thin-Client (`?pair=ws://...&token=...`)
    // =========================================================================
    console.log("==> [6/7] Testing Live nve pair Binary WebSocket Thin-Client (?pair=ws://...&token=...)...");
    const { nveBin, counterDylib } = ensurePairArtifacts();
    try {
      pairServer = await launchPairServer(nveBin, counterDylib);
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
        "Live paired thin-client connected with ZERO local WASM app and rendered Counter Count: 0",
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
            text.includes("NATIVE VIEW ENGINE") &&
            text.includes("Count: 0") &&
            text.includes("Increment +") &&
            text.includes("Reset")
          );
        }
      );

      // Click Increment + twice -> live InputEvents=11 -> Rust CounterModule -> NVD1 delta -> DOM.
      await clickElement(cdp, findButtonByText("Increment +"), "Paired Increment + (1st click)");
      await waitForCondition(
        cdp,
        "Paired Counter incremented to Count: 1 over live binary WebSocket",
        () => (document.getElementById("app")?.innerText || "").includes("Count: 1")
      );

      await clickElement(cdp, findButtonByText("Increment +"), "Paired Increment + (2nd click)");
      await waitForCondition(
        cdp,
        "Paired Counter incremented to Count: 2 over live binary WebSocket",
        () =>
          (document.getElementById("app")?.innerText || "").includes("Count: 2") &&
          Number(window.__nvePairHost?.frames_received() || 0) >= 3
      );

      // Click Reset -> verify Count: 0 round-trip.
      await clickElement(cdp, findButtonByText("Reset"), "Paired Reset");
      await waitForCondition(
        cdp,
        "Paired Counter reset to Count: 0 over live binary WebSocket",
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
        const searchAfter = document.querySelector('[testid="board-search-input"]');
        const statusAfter = document.querySelector('[testid="task-status-1"]');
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
      `document.querySelector('[testid="board-search-input"]')`,
      "AppKit",
      "hydrated board-search-input"
    );
    await waitForCondition(
      cdp,
      "Hydrated search filtered board to AET-102 while preserving <input> focus",
      () => {
        const text = document.getElementById("app")?.innerText || "";
        const inp = document.querySelector('[testid="board-search-input"]');
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
      `document.querySelector('[testid="board-search-clear"]')`,
      "hydrated board-search-clear"
    );
    await waitForCondition(
      cdp,
      "Hydrated board search cleared via root event delegation",
      () => (document.getElementById("app")?.innerText || "").includes("AET-101")
    );
    await clickElement(
      cdp,
      `document.querySelector('[testid="task-status-1"]')`,
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
        const cardBefore = host.querySelector('article[testid="probe-card"]');
        const headingBefore = host.querySelector('h1[testid="probe-heading"]');
        const linkBefore = host.querySelector('a[testid="probe-link"]');
        const secondMismatches = window.__nveWasmMod.run_semantic_style_hydration_probe('nve-style-probe');
        const cardAfter = host.querySelector('article[testid="probe-card"]');
        const headingAfter = host.querySelector('h1[testid="probe-heading"]');
        const linkAfter = host.querySelector('a[testid="probe-link"]');
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
        const res = {
          connected: host.isConnected && document.getElementById('nve-hostile-probe') === host,
          rootTag: host.tagName,
          rootId: host.getAttribute('data-nve-id'),
          rootChildren: [...host.children].map(id).join(','),
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

    if (errors.length > 0) {
      throw new Error(
        `Browser console/runtime errors detected (${errors.length}):\n${errors.join("\n")}`
      );
    }

    // 7e. A real Rust panic prints its message and file:line (checked last: the trap poisons the
    //     instance). The trigger exists only in the dev bundle.
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

    console.log(`==> ALL 7 WEB WASM + LIVE PAIR + SSR/HYDRATION E2E SUITES PASSED (0 console errors and 0 WASM panics before the deliberate panic probe)`);
  } finally {
    await cleanup();
  }
}

main().catch((err) => {
  console.error("E2E FAILED:", err?.stack || err);
  process.exit(1);
});
