#!/usr/bin/env node
// Publish N real FLV streams through FFmpeg tee to one or more m7s-rs nodes.
// Usage: node scripts/multi_push.cjs <streams> <seconds> <nodes> [workers] [sample.flv]
// nodes: "127.0.0.1:1935,127.0.0.1:1936"
const { spawn } = require("child_process");
const fs = require("fs");
const http = require("http");

const total = Number(process.argv[2] || 20);
const seconds = Number(process.argv[3] || 8);
const nodes = (process.argv[4] || "127.0.0.1:1935").split(",");
const workers = Number(process.argv[5] || 2);
const sample = process.argv[6] || "target/edge1.flv";
const ffmpeg = process.env.M7S_FFMPEG || "node_modules/ffmpeg-static/ffmpeg.exe";

if (!fs.existsSync(sample)) {
  console.error(`missing sample ${sample}`);
  process.exit(2);
}

function httpPortFor(rtmpPort) {
  const map = { 1935: 8180, 1936: 8280, 1938: 8380 };
  return map[Number(rtmpPort)] || 8180;
}

function apiStreams(node) {
  const [host, port] = node.split(":");
  const httpPort = httpPortFor(port);
  return new Promise((resolve) => {
    http
      .get({ host, port: httpPort, path: "/api/streams" }, (res) => {
        let b = "";
        res.on("data", (c) => (b += c));
        res.on("end", () => {
          try {
            resolve(JSON.parse(b).data.streams.length);
          } catch {
            resolve(0);
          }
        });
      })
      .on("error", () => resolve(0));
  });
}

function runWorker(w, perWorker) {
  const first = w * perWorker;
  const last = Math.min(total, first + perWorker);
  if (first >= last) return Promise.resolve({ count: 0, code: 0 });
  const outs = [];
  for (let i = first; i < last; i += 1) {
    const [host, port] = nodes[i % nodes.length].split(":");
    outs.push(`[f=flv]rtmp://${host}:${port}/live/s${i}`);
  }
  const args = [
    "-hide_banner",
    "-loglevel",
    "error",
    "-re",
    "-i",
    sample,
    "-map",
    "0",
    "-c",
    "copy",
    "-t",
    String(seconds),
    "-f",
    "tee",
    outs.join("|"),
  ];
  if (process.env.DEBUG_PUSH) console.error(args.join(" "));
  return new Promise((resolve) => {
    const p = spawn(ffmpeg, args, { stdio: ["ignore", "ignore", "inherit"] });
    p.on("error", (e) => {
      console.error("spawn error", e.message);
      resolve({ count: last - first, code: 1 });
    });
    p.on("exit", (code) => resolve({ count: last - first, code: code ?? 1 }));
  });
}

(async () => {
  console.log(`total=${total} workers=${workers} nodes=${nodes.join(",")}`);
  const perWorker = Math.ceil(total / workers);
  const start = Date.now();
  const jobs = [];
  for (let w = 0; w < workers; w += 1) jobs.push(runWorker(w, perWorker));
  await new Promise((r) => setTimeout(r, 2500));
  const counts = [];
  for (const n of nodes) counts.push(await apiStreams(n));
  const results = await Promise.all(jobs);
  const active = counts.reduce((a, b) => a + b, 0);
  const ok = results.reduce((a, r) => a + (r.code === 0 ? r.count : 0), 0);
  console.log(
    `published=${ok}/${total} active_at_check=${active} elapsed_s=${((Date.now() - start) / 1000).toFixed(2)}`
  );
  if (ok !== total) process.exitCode = 1;
})();
