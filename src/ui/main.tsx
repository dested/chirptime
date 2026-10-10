import { createRoot } from "react-dom/client";
import "@fontsource-variable/schibsted-grotesk";
import "@fontsource-variable/geist-mono";
import "./app.css";

const params = new URLSearchParams(window.location.search);
const isViewer = params.get("mode") === "viewer";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("index.html has no #root");
const root = createRoot(rootEl);

async function boot() {
  // Browser dev harness: answer invoke() from the fixtures. Never in a build.
  if (import.meta.env.DEV && params.get("mock") === "1") {
    const { installMockIpc } = await import("./dev/mockIpc");
    await installMockIpc(isViewer ? "viewer" : "main");
  }
  // Each window loads only its own module. The widget touches Tauri window APIs
  // at import time, so the app window must never evaluate it.
  if (isViewer) {
    const { App } = await import("./app/App");
    root.render(<App />);
  } else {
    const { CompactApp } = await import("./CompactApp");
    root.render(<CompactApp />);
  }
}

void boot();
