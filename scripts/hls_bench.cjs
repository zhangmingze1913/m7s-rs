#!/usr/bin/env node
// HLS segmenter benchmark: concurrent playlist + segment GETs.
// Usage: node scripts/hls_bench.cjs <seconds> <concurrency> <http_port>
const http = require("http");

const seconds = Number(process.argv[2] || 10);
const concurrency = Number(process.argv[3] || 20);
const port = Number(process.argv[4] || 8180);
const host = "127.0.0.1";
let ok = 0;
let failed = 0;
const samples = [];

function get(path) {
  return new Promise((resolve) => {
    const started = process.hrtime.bigint();
    const req = http.get({ host, port, path, agent: false }, (res) => {
      let body = "";
      res.on("data", (c) => (body += c));
      res.on("end", () => {
        const ms = Number(process.hrtime.bigint() - started) / 1e6;
        samples.push(ms);
        if (res.statusCode === 200) ok += 1;
        else failed += 1;
        resolve({ body, status: res.statusCode });
      });
    });
    req.on("error", () => {
      failed += 1;
      resolve({ body: "", status: 0 });
    });
  });
}

function firstSegment(playlist) {
  const line = playlist.split("\n").find((l) => l.trim().endsWith(".ts"));
  return line ? line.trim() : "";
}

async function worker(until) {
  while (Date.now() < until) {
    const list = await get("/hls/live/test/index.m3u8");
    const seg = firstSegment(list.body);
    if (seg) await get(`/hls/live/test/${seg}`);
  }
}

(async () => {
  await get("/hls/live/test/index.m3u8");
  const until = Date.now() + seconds * 1000;
  await Promise.all(Array.from({ length: concurrency }, () => worker(until)));
  samples.sort((a, b) => a - b);
  const pct = (p) => samples[Math.min(samples.length - 1, Math.floor(samples.length * p))] || 0;
  console.log(
    JSON.stringify({
      ok,
      failed,
      requests: samples.length,
      rps: Number((samples.length / seconds).toFixed(1)),
      p50_ms: Number(pct(0.5).toFixed(2)),
      p99_ms: Number(pct(0.99).toFixed(2)),
    })
  );
})();
