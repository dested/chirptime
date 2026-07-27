import React from "react";
import { createRoot } from "react-dom/client";
import "./app.css";

const params = new URLSearchParams(window.location.search);
const isViewer = params.get("mode") === "viewer";

const root = createRoot(document.getElementById("root")!);

// Each window loads only its own module. The widget touches Tauri window APIs at
// import time, so the viewer must never evaluate it.
if (isViewer) {
  void import("./ViewerApp").then(({ ViewerApp }) => root.render(<ViewerApp />));
} else {
  void import("./CompactApp").then(({ CompactApp }) => root.render(<CompactApp />));
}
