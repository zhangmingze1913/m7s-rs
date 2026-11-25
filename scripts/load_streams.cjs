#!/usr/bin/env node
// Create N streams and open one HTTP-FLV viewer per stream.
// Usage: node scripts/load_streams.cjs 10000 10 [host:port]
const http = require("http");
const net = require("net");

const count = Number(process.argv[2] || 1000);
const seconds = Number(process.argv[3] || 10);
const target = process.argv[4] || "127.0.0.1:8180";
const [host, portText] = target.split(":");
const port = Number(portText || 8180);

function create(i) {
  return new Promise((resolve) => {
    const req = http.get(
      { host, port, path: `/api/debug/stream?key=live/s${i}`, agent: false },
      (res) => {
        res.resume();
        res.on("end", resolve);
      }
    );
    req.on("error", () => resolve());
  });
}

async function createAll() {
  const batch = 200;
  let done = 0;
  for (let i = 0; i < count; i += batch) {
    await Promise.all(
      Array.from({ length: Math.min(batch, count - i) }, (_, k) => create(i + k))
    );
    done += Math.min(batch, count - i);
    if (done % 2000 === 0) process.stderr.write(`created ${done}\n`);
  }
}

let ok = 0;
let failed = 0;

function viewer(i) {
  return new Promise((resolve) => {
    const s = net.connect({ host, port });
    let buf = "";
    let counted = false;
    const finish = () => {
      if (!counted) failed += 1;
      s.destroy();
      resolve();
    };
    const timer = setTimeout(finish, (seconds + 5) * 1000);
    s.on("connect", () => {
      s.write(
        `GET /flv/live/s${i}.flv HTTP/1.1\r\nHost: ${host}\r\nConnection: close\r\n\r\n`
      );
    });
    s.on("data", (c) => {
      buf += c.toString("latin1");
      if (!counted && buf.includes("200")) {
        counted = true;
        ok += 1;
        clearTimeout(timer);
        s.destroy();
        resolve();
      }
    });
    s.on("error", finish);
  });
}

(async () => {
  const start = Date.now();
  await createAll();
  const created = Date.now();
  const batch = 200;
  for (let i = 0; i < count; i += batch) {
    await Promise.all(
      Array.from({ length: Math.min(batch, count - i) }, (_, k) => viewer(i + k))
    );
  }
  console.log(
    `streams=${count} created_ms=${created - start} viewers_ok=${ok} failed=${failed} elapsed_s=${(
      (Date.now() - start) / 1000
    ).toFixed(2)}`
  );
})();
