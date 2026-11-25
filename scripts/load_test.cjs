#!/usr/bin/env node
// Open N HTTP-FLV viewers and report success. Node.js 16+.
// Start server: m7s-rs --config config/server.toml --demo --duration 30
// Run:          node scripts/load_test.cjs 1000 10
const net = require("net");

const first = process.argv[2] || "100";
const numeric = /^\d+$/.test(first);
const target = numeric
  ? process.argv[4] || "127.0.0.1:8180/live"
  : first;
const count = Number(numeric ? first : process.argv[3] || 100);
const seconds = Number(numeric ? process.argv[3] || 10 : process.argv[4] || 10);
const [hostPort, ...rest] = target.split("/");
const [host, portText] = hostPort.split(":");
const port = Number(portText || 8180);
const app = rest.join("/") || "live";
const path = `/flv/${app.replace(/^\//, "")}/test.flv`;

let sourceIps = Number(process.env.LOAD_SOURCE_IPS || 1);
const targetList = (process.env.LOAD_TARGETS || `${host}:${port}`).split(",");
let targetIdx = 0;
let nextIp = 0;
const ipBase = Number(process.env.LOAD_IP_BASE || 2);
let ok = 0;
let failed = 0;

function viewer(attempt = 0) {
  return new Promise((resolve) => {
    const localAddress = sourceIps > 1 ? `127.0.0.${ipBase + (nextIp++ % sourceIps)}` : undefined;
    const socket = net.connect({ host, port, localAddress });
    let header = "";
    let counted = false;
    let failedCounted = false;
    let settled = false;
    let timer;

    const fail = () => {
      if (!failedCounted) {
        failedCounted = true;
        failed += 1;
      }
    };
    const done = () => {
      if (settled) return;
      settled = true;
      if (!counted) fail();
      clearTimeout(timer);
      socket.destroy();
      resolve();
    };

    timer = setTimeout(done, (seconds + 5) * 1000);
    socket.on("connect", () => {
      socket.write(
        `GET ${path} HTTP/1.1\r\nHost: ${host}\r\nConnection: close\r\n\r\n`
      );
    });
    socket.on("data", (chunk) => {
      header += chunk.toString("latin1");
      if (!header.includes("200")) {
        if (process.env.LOAD_DEBUG) {
          console.error("bad header:", JSON.stringify(header.slice(0, 80)));
        }
        fail();
        done();
        return;
      }
      if (!header.includes("FLV")) return;
      if (!counted) {
        counted = true;
        ok += 1;
        setTimeout(done, seconds * 1000);
      }
    });
    socket.on("error", (err) => {
      if (process.env.LOAD_DEBUG) console.error("viewer error:", err.message);
      done();
    });
    socket.on("close", done);
  });
}

async function main() {
  const start = Date.now();
  const concurrency = Math.min(
    Number(process.env.LOAD_CONCURRENCY || count),
    count
  );
  let next = 0;
  const workers = Array.from({ length: concurrency }, async () => {
    while (next < count) {
      next += 1;
      await viewer();
    }
  });
  await Promise.all(workers);
  console.log(
    `viewers=${count} ok=${ok} failed=${failed} ` +
      `elapsed=${((Date.now() - start) / 1000).toFixed(2)}s ` +
      `url=http://${host}:${port}${path}`
  );
}

main();
