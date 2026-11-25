import React from "react";

export default function App({ data, rooms, plugins, audit }) {
  if (!data) return <div className="loading">Loading Monibuca Admin…</div>;
  return (
    <div className="admin">
      <h1>Monibuca Admin</h1>
      <p>Node: {data.node_id} · Role: {data.role || "admin"} · Actor: {data.actor || "api-token"}</p>
      <ul>
        <li>Streams: {data.streams}</li>
        <li>Subscribers: {data.subscribers}</li>
        <li>Rooms: {data.rooms}</li>
        <li>Plugins: {data.plugins.enabled}/{data.plugins.total}</li>
        <li>DB: {data.db ? "enabled" : "disabled"}</li>
      </ul>
      <h2>Rooms</h2>
      <ul>{(rooms || []).map((r) => <li key={r.name}>{r.name} ({r.members})</li>)}</ul>
      <h2>Plugins</h2>
      <ul>{(plugins || []).map((p) => <li key={p.name}>{p.name}: {p.status}</li>)}</ul>
      <h2>Audit</h2>
      <ul>
        {(audit || []).map((a, i) => (
          <li key={i}>{a.action} {a.target} by {a.actor} ({a.role}) {a.ok ? "ok" : "failed"}</li>
        ))}
      </ul>
    </div>
  );
}
