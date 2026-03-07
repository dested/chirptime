import React from "react";
import { createRoot } from "react-dom/client";
import { CompactApp } from "./CompactApp";
import { ViewerApp } from "./ViewerApp";

const params = new URLSearchParams(window.location.search);
const isViewer = params.get("mode") === "viewer";

const root = createRoot(document.getElementById("root")!);
root.render(isViewer ? <ViewerApp /> : <CompactApp />);
