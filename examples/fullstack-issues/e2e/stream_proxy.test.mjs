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
