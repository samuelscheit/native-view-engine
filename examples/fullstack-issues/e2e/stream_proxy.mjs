/**
 * Same-origin HTTP proxy that cuts ONE armed streaming SSR document at an exact byte offset:
 * everything before the cut is flushed to the browser, then the response is either paused until
 * `release()` (a TCP-level split mid-markup) or ended without the stream-end marker via
 * `truncate()` (an upstream connection dropped mid-stream). Every other request (boot.js, WASM,
 * `/rpc/*`, `/_test/*`) passes through untouched, except that the browser's `Origin` / `Referer`
 * naming the proxy are rewritten to the upstream origin (`changeOrigin`), so the server's
 * single-canonical-origin CSRF policy sees the transparent front it would in production.
 */

import http from "node:http";

/** Bytes kept unforwarded while the marker may still be arriving across chunk boundaries. */
const markerWindow = (plan) =>
  plan.marker.length + (plan.cutAfter?.length ?? 0) + (plan.cutBefore?.length ?? 0) + 512;

export function startStreamProxy(upstreamPort) {
  let armed = null;
  let active = null;
  let proxyOrigin = "";
  const upstreamOrigin = `http://127.0.0.1:${upstreamPort}`;

  const server = http.createServer((req, res) => {
    req.socket.setNoDelay(true);
    const url = new URL(req.url || "/", "http://127.0.0.1");
    const plan = req.method === "GET" && url.pathname === armed?.path ? armed : null;
    if (plan) armed = null;
    const headers = { ...req.headers, host: `127.0.0.1:${upstreamPort}` };
    if (headers.origin === proxyOrigin) headers.origin = upstreamOrigin;
    if (headers.referer?.startsWith(`${proxyOrigin}/`)) {
      headers.referer = upstreamOrigin + headers.referer.slice(proxyOrigin.length);
    }
    // Byte offsets must index the identity-encoded HTML.
    delete headers["accept-encoding"];
    const upstream = http.request(
      { host: "127.0.0.1", port: upstreamPort, method: req.method, path: req.url, headers },
      (up) => {
        if (!plan) {
          res.writeHead(up.statusCode || 502, up.headers);
          up.pipe(res);
          return;
        }
        const outHeaders = { ...up.headers };
        delete outHeaders["content-length"];
        delete outHeaders["transfer-encoding"];
        delete outHeaders.connection;
        res.writeHead(up.statusCode || 502, outHeaders);
        res.flushHeaders();
        active = cutStream(plan, up, upstream, res);
      }
    );
    upstream.on("error", (err) => {
      if (!res.headersSent) {
        res.writeHead(502);
        res.end(String(err));
      } else {
        res.destroy();
      }
    });
    req.pipe(upstream);
  });

  return new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      proxyOrigin = `http://127.0.0.1:${server.address().port}`;
      resolve({
        server,
        origin: proxyOrigin,
        /**
         * Arms the next `GET <path>` document: the cut lands right after `cutAfter` or right
         * before `cutBefore` (both searched from `marker`), or right after `marker` itself.
         * Resolves once the cut is reached; rejects after `timeoutMs` so a marker that drifted
         * out of the markup fails fast instead of hanging the suite.
         */
        arm({ path = "/issues", marker, cutAfter = "", cutBefore = "", timeoutMs = 15_000 }) {
          let onCut;
          let timer;
          const held = new Promise((resolve, reject) => {
            onCut = () => {
              clearTimeout(timer);
              resolve();
            };
            timer = setTimeout(() => {
              if (armed?.onCut === onCut) armed = null;
              reject(new Error(`stream proxy cut not reached in ${timeoutMs} ms: ${JSON.stringify({ marker, cutAfter, cutBefore })}`));
            }, timeoutMs);
          });
          armed = { path, marker, cutAfter, cutBefore, onCut };
          return held;
        },
        isHeld: () => Boolean(active?.held),
        release: () => active?.release(),
        truncate: () => active?.truncate(),
      });
    });
  });
}

function cutStream(plan, up, upstreamReq, res) {
  let buf = Buffer.alloc(0);
  let sent = 0;
  let cut = -1;
  let upstreamEnded = false;
  let truncated = false;
  const state = { held: false, release, truncate };

  const forwardTo = (end) => {
    if (!truncated && end > sent) {
      res.write(buf.subarray(sent, end));
      sent = end;
    }
  };
  const locateCut = () => {
    // latin1 maps bytes 1:1 onto UTF-16 code units, so string indices are byte offsets.
    const text = buf.toString("latin1");
    const markerAt = text.indexOf(plan.marker);
    if (markerAt < 0) return { cut: -1, safe: Math.max(0, buf.length - markerWindow(plan)) };
    if (plan.cutBefore) {
      const beforeAt = text.indexOf(plan.cutBefore, markerAt);
      return beforeAt < 0 ? { cut: -1, safe: markerAt } : { cut: beforeAt, safe: markerAt };
    }
    if (!plan.cutAfter) return { cut: markerAt + plan.marker.length, safe: markerAt };
    const afterAt = text.indexOf(plan.cutAfter, markerAt);
    return afterAt < 0
      ? { cut: -1, safe: markerAt }
      : { cut: afterAt + plan.cutAfter.length, safe: markerAt };
  };

  up.on("data", (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    if (cut >= 0) {
      if (!state.held) forwardTo(buf.length);
      return;
    }
    const at = locateCut();
    if (at.cut < 0) {
      forwardTo(at.safe);
      return;
    }
    cut = at.cut;
    forwardTo(cut);
    state.held = true;
    plan.onCut();
  });
  up.on("end", () => {
    upstreamEnded = true;
    if (!state.held && !truncated) {
      forwardTo(buf.length);
      res.end();
    }
  });

  function release() {
    if (!state.held) return;
    state.held = false;
    forwardTo(buf.length);
    if (upstreamEnded) res.end();
  }

  /** Ends the document gracefully at the cut (no stream-end marker) and drops the upstream. */
  function truncate() {
    if (!state.held) return;
    state.held = false;
    truncated = true;
    res.end();
    upstreamReq.destroy();
    up.destroy();
  }

  return state;
}
