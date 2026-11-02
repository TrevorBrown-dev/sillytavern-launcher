import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { api } from "./lib/api";

// Anything the UI throws and nobody catches lands in the session log, so a "the button
// does nothing" report comes with the actual error.
window.addEventListener("error", (ev) => {
  const where = ev.filename ? ` (${ev.filename}:${ev.lineno}:${ev.colno})` : "";
  void api.uiError(`${ev.message}${where}${ev.error?.stack ? `\n${ev.error.stack}` : ""}`);
});
window.addEventListener("unhandledrejection", (ev) => {
  const reason = ev.reason instanceof Error ? (ev.reason.stack ?? ev.reason.message) : String(ev.reason);
  void api.uiError(`unhandled promise rejection: ${reason}`);
});

const app = mount(App, { target: document.getElementById("app")! });

export default app;
