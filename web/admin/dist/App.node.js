var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
  // If the importer is in node compatibility mode or this is not an ESM
  // file that has been converted to a CommonJS file using a Babel-
  // compatible transform (i.e. "__esModule" has not been set), then set
  // "default" to the CommonJS "module.exports" for node compatibility.
  isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
  mod
));
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// src/App.jsx
var App_exports = {};
__export(App_exports, {
  default: () => App
});
module.exports = __toCommonJS(App_exports);
var import_react = __toESM(require("react"));
function App({ data, rooms, plugins, audit }) {
  if (!data) return /* @__PURE__ */ import_react.default.createElement("div", { className: "loading" }, "Loading Monibuca Admin\u2026");
  return /* @__PURE__ */ import_react.default.createElement("div", { className: "admin" }, /* @__PURE__ */ import_react.default.createElement("h1", null, "Monibuca Admin"), /* @__PURE__ */ import_react.default.createElement("p", null, "Node: ", data.node_id, " \xB7 Role: ", data.role || "admin", " \xB7 Actor: ", data.actor || "api-token"), /* @__PURE__ */ import_react.default.createElement("ul", null, /* @__PURE__ */ import_react.default.createElement("li", null, "Streams: ", data.streams), /* @__PURE__ */ import_react.default.createElement("li", null, "Subscribers: ", data.subscribers), /* @__PURE__ */ import_react.default.createElement("li", null, "Rooms: ", data.rooms), /* @__PURE__ */ import_react.default.createElement("li", null, "Plugins: ", data.plugins.enabled, "/", data.plugins.total), /* @__PURE__ */ import_react.default.createElement("li", null, "DB: ", data.db ? "enabled" : "disabled")), /* @__PURE__ */ import_react.default.createElement("h2", null, "Rooms"), /* @__PURE__ */ import_react.default.createElement("ul", null, (rooms || []).map((r) => /* @__PURE__ */ import_react.default.createElement("li", { key: r.name }, r.name, " (", r.members, ")"))), /* @__PURE__ */ import_react.default.createElement("h2", null, "Plugins"), /* @__PURE__ */ import_react.default.createElement("ul", null, (plugins || []).map((p) => /* @__PURE__ */ import_react.default.createElement("li", { key: p.name }, p.name, ": ", p.status))), /* @__PURE__ */ import_react.default.createElement("h2", null, "Audit"), /* @__PURE__ */ import_react.default.createElement("ul", null, (audit || []).map((a, i) => /* @__PURE__ */ import_react.default.createElement("li", { key: i }, a.action, " ", a.target, " by ", a.actor, " (", a.role, ") ", a.ok ? "ok" : "failed"))));
}
