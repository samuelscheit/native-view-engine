// Node check of counter.wasm: mount html, increment x3, reset, negative-safe itoa.
import { readFileSync } from "node:fs";
const d = new TextDecoder(); let x, html = "", text = "";
const g = (p, l) => d.decode(new Uint8Array(x.m.buffer, p, l));
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("counter.opt.wasm", import.meta.url)), { d: { h: (p, l) => (html = g(p, l)), t: (p, l) => (text = g(p, l)) } });
x = instance.exports; x.s();
const seen = [];
for (const id of ["1", "1", "1", "2", "1"]) { x.e(id); seen.push(text); }
for (let i = 0; i < 1234; i++) x.e(1);
seen.push(text);
const ok = html.includes("Increment +") && html.includes("<u>0</u>") && JSON.stringify(seen) === JSON.stringify(["1","2","3","0","1","1235"]);
console.log(ok ? "OK" : "FAIL", seen, html.length);
process.exit(ok ? 0 : 1);
