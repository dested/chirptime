import { defineConfig, type Plugin } from "vite";
import { REPO_URL } from "./src/config";
import { renderCatalog, renderPlayer, renderRibbon, renderVoiceCard } from "./src/render";

/**
 * Fills the placeholders in index.html at build time:
 *  - `{{REPO_URL}}` / `{{REPO_LABEL}}` → the single repository constant in
 *    src/config.ts, so changing the repo is a one-line edit.
 *  - `<!--@ribbon-->`, `<!--@player-->`, `<!--@catalog-->` → the day drawings,
 *    baked in as static markup so the page is complete without JavaScript.
 */
function staticContent(): Plugin {
  const label = REPO_URL.replace(/^https?:\/\//, "").replace(/\/$/, "");
  return {
    name: "chirptime-static-content",
    transformIndexHtml: {
      order: "pre",
      handler(html) {
        return html
          .replace("<!--@ribbon-->", renderRibbon())
          .replace("<!--@player-->", renderPlayer())
          .replace("<!--@catalog-->", renderCatalog())
          .replace("<!--@voicecard-->", renderVoiceCard())
          .replaceAll("{{REPO_URL}}", REPO_URL)
          .replaceAll("{{REPO_LABEL}}", label);
      },
    },
  };
}

export default defineConfig({
  plugins: [staticContent()],
  build: {
    outDir: "dist",
    assetsInlineLimit: 0,
  },
});
