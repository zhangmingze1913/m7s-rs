// Probe m7s-rs HTTP API with Node.js 18+ built-in fetch.
const base = process.argv[2] ?? "http://127.0.0.1:8180";

for (const path of ["/health", "/api/streams", "/cluster/api/status/local", "/cluster/api/sessions"]) {
  try {
    const response = await fetch(base + path);
    console.log(path, response.status, await response.json());
  } catch (error) {
    console.log(path, "ERROR", error.message);
  }
}
