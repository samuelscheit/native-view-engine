import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { startStreamProxy } from "./stream_proxy.mjs";

function listen(server) {
  return new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => resolve(server.address().port));
  });
}

function closeServer(server) {
  return new Promise((resolve) => server.close(() => resolve()));
}

function httpGetText(url) {
  return new Promise((resolve, reject) => {
    http
      .get(url, (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () =>
          resolve({ status: res.statusCode, body: Buffer.concat(chunks).toString("utf8") })
        );
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

test("missed marker rejected before await cut triggers zero unhandled rejections", async () => {
  const unhandled = [];
  const onUnhandled = (err) => unhandled.push(err);
  process.on("unhandledRejection", onUnhandled);

  const upstream = http.createServer((_req, res) => {
    res.writeHead(200, { "content-type": "text/html" });
    res.end("<html><body>no marker here</body></html>");
  });
  const port = await listen(upstream);
  const proxy = await startStreamProxy(port);
  try {
    const cut = proxy.arm({ marker: "MISSING_MARKER", timeoutMs: 15_000 });
    const res = await httpGetText(`${proxy.origin}/issues`);
    assert.equal(res.status, 200);
    // Simulate caller performing async work (CDP navigation / assertions) before `await cut`.
    await new Promise((r) => setTimeout(r, 40));
    assert.equal(unhandled.length, 0, "must not emit unhandledRejection before await cut");
    await assert.rejects(cut, /marker never appeared before EOF/);
  } finally {
    process.off("unhandledRejection", onUnhandled);
    await closeServer(proxy.server);
    await closeServer(upstream);
  }
});

test("upstream connect error rejects armed cut promptly without waiting for timeout", async () => {
  const probe = http.createServer();
  const deadPort = await listen(probe);
  await closeServer(probe);

  const proxy = await startStreamProxy(deadPort);
  try {
    const t0 = Date.now();
    const cut = proxy.arm({ marker: "X", timeoutMs: 15_000 });
    const res = await httpGetText(`${proxy.origin}/issues`);
    assert.equal(res.status, 502);
    await assert.rejects(cut, /upstream request error/);
    assert.ok(Date.now() - t0 < 2_000, "must reject immediately on upstream connect error");
  } finally {
    await closeServer(proxy.server);
  }
});

test("upstream abrupt abort before marker rejects promptly without waiting for timeout", async () => {
  const upstream = http.createServer((_req, res) => {
    res.writeHead(200, { "content-type": "text/html" });
    res.write("<html><body>partial");
    setTimeout(() => res.socket.destroy(), 15);
  });
  const port = await listen(upstream);
  const proxy = await startStreamProxy(port);
  try {
    const t0 = Date.now();
    const cut = proxy.arm({ marker: "NEVER_ARRIVES", timeoutMs: 15_000 });
    const res = await httpGetText(`${proxy.origin}/issues`);
    assert.equal(res.status, 200);
    await assert.rejects(cut, /upstream response (error|closed before EOF)/);
    assert.ok(Date.now() - t0 < 2_000, "must reject immediately on upstream abort");
  } finally {
    await closeServer(proxy.server);
    await closeServer(upstream);
  }
});

test("release delivers full payload across cut and truncate drops trailing bytes cleanly", async () => {
  const payload = "<html><head></head><body>BEFORE<marker/>AFTER</body></html>";
  const upstream = http.createServer((_req, res) => {
    res.writeHead(200, { "content-type": "text/html" });
    res.end(payload);
  });
  const port = await listen(upstream);
  const proxy = await startStreamProxy(port);
  try {
    // 1. Hold at marker, then release.
    const cut1 = proxy.arm({ marker: "<marker/>", timeoutMs: 5_000 });
    const req1 = httpGetText(`${proxy.origin}/issues`);
    await cut1;
    assert.equal(proxy.isHeld(), true);
    proxy.release();
    const res1 = await req1;
    assert.equal(res1.body, payload);

    // 2. Hold at marker, then truncate.
    const cut2 = proxy.arm({ marker: "<marker/>", timeoutMs: 5_000 });
    const req2 = httpGetText(`${proxy.origin}/issues`);
    await cut2;
    assert.equal(proxy.isHeld(), true);
    proxy.truncate();
    const res2 = await req2;
    assert.equal(res2.body, "<html><head></head><body>BEFORE<marker/>");
  } finally {
    await closeServer(proxy.server);
    await closeServer(upstream);
  }
});

test("superseding arm, timeout expiry, and upstream abort while held are handled cleanly", async () => {
  let activeRes = null;
  const upstream = http.createServer((_req, res) => {
    activeRes = res;
    res.writeHead(200, { "content-type": "text/html" });
    res.write("<html><body>BEFORE<marker/>TAIL");
    // Leave open until destroyed mid-hold.
  });
  const port = await listen(upstream);
  const proxy = await startStreamProxy(port);
  try {
    // 1. Re-arming rejects the superseded promise cleanly without unhandled rejection.
    const superseded = proxy.arm({ marker: "OLD", timeoutMs: 5_000 });
    const timedOut = proxy.arm({ marker: "NEVER", timeoutMs: 30 });
    await assert.rejects(superseded, /superseded by a newer arm\(\) call/);
    await assert.rejects(timedOut, /not reached in 30 ms/);

    // 2. Upstream aborts while held: stream stays held (not destroyed early), and release()
    // surfaces the abort rather than faking a clean EOF.
    const cut = proxy.arm({ marker: "<marker/>", timeoutMs: 5_000 });
    let clientChunk = "";
    const clientDone = new Promise((resolve) => {
      const req = http.get(`${proxy.origin}/issues`, (res) => {
        res.on("data", (c) => {
          clientChunk += c.toString("utf8");
        });
        res.on("end", () => resolve("end"));
        res.on("close", () => resolve("close"));
        res.on("error", () => resolve("error"));
      });
      req.on("error", () => resolve("req-error"));
    });
    await cut;
    assert.equal(proxy.isHeld(), true);
    // Destroy upstream socket while held; cut already resolved (single settlement) and held stays true.
    activeRes.socket.destroy();
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(proxy.isHeld(), true, "upstream RST while held must not tear down held response");
    assert.equal(clientChunk, "<html><body>BEFORE<marker/>");
    proxy.release();
    const outcome = await clientDone;
    assert.notEqual(outcome, "end", "abort after release must not look like a clean EOF");
  } finally {
    activeRes?.socket?.destroy();
    await closeServer(proxy.server);
    await closeServer(upstream);
  }
});
