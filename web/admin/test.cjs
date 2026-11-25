const React = require("react");
const { renderToString } = require("react-dom/server");
const App = require("./dist/App.node.js").default;
const html = renderToString(React.createElement(App, {
  data: { node_id: "n1", streams: 2, subscribers: 3, rooms: 1, plugins: { enabled: 26, total: 26 }, db: true },
  rooms: [{ name: "r1", members: 2 }],
  plugins: [{ name: "rtmp", status: "enabled" }],
  audit: [{ action: "plugin.toggle", target: "rtmp", actor: "admin", role: "admin", ok: true }],
}));
if (!html.includes("Monibuca Admin") || !html.includes("r1") || !html.includes("plugin.toggle")) {
  console.error(html);
  process.exit(1);
}
console.log("react_admin_ok");
