import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App.jsx";

async function load() {
  const [o, r, p, a] = await Promise.all([
    fetch("/api/admin/overview").then((x) => x.json()),
    fetch("/api/live/rooms").then((x) => x.json()),
    fetch("/api/plugins").then((x) => x.json()),
    fetch("/api/audit").then((x) => x.json()),
  ]);
  createRoot(document.getElementById("root")).render(
    <App data={o} rooms={r.rooms} plugins={p.plugins} audit={a.entries} />
  );
}
load();
