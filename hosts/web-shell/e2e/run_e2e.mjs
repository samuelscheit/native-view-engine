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
      height: 800,
      deviceScaleFactor: 2,
      mobile: false,
    });

    // =========================================================================
    // 1. Aether Workspace + NVE Studio Desktop 1280x800 (`/?app=studio&ws=off`)
    // =========================================================================
    console.log("==> [1/3] Testing Aether Workspace + NVE Studio Desktop 1280x800 (?app=studio&ws=off)...");
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
    console.log("==> [2/3] Testing Responsive Mobile Portrait Resize (393x852 -> 1280x800)...");
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
    console.log("==> [3/3] Testing Counter App (?app=counter&ws=off)...");
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
