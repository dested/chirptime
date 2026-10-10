/* Shared helpers for the mocks. Not app code: a shot harness plus a few
   formatters so every mock renders the same real manifest the same way. */

/* Shot harness: ?z=1.25&ox=640&oy=400 zooms the page and shifts the window so
   a 1024px-wide screenshot can capture any quadrant at 1:1. */
(function () {
  const p = new URLSearchParams(location.search);
  const z = p.get("z");
  if (z) document.documentElement.style.zoom = z;
  const ox = Number(p.get("ox") || 0), oy = Number(p.get("oy") || 0);
  if (ox || oy) {
    document.addEventListener("DOMContentLoaded", () => {
      const w = document.querySelector(".win");
      if (w) w.style.transform = `translate(${-ox}px, ${-oy}px)`;
    });
  }
})();

const CT = {
  p2: (n) => String(n).padStart(2, "0"),
  clock(ms, secs = true) {
    const d = new Date(ms);
    let h = d.getHours();
    const ap = h >= 12 ? "PM" : "AM";
    h = h % 12 || 12;
    return `${h}:${CT.p2(d.getMinutes())}${secs ? ":" + CT.p2(d.getSeconds()) : ""} ${ap}`;
  },
  clockShort(ms) {
    const d = new Date(ms);
    let h = d.getHours();
    const ap = h >= 12 ? "p" : "a";
    h = h % 12 || 12;
    return d.getMinutes() === 0 ? `${h}${ap}` : `${h}:${CT.p2(d.getMinutes())}`;
  },
  dur(ms) {
    const m = Math.round(ms / 60000);
    const h = Math.floor(m / 60);
    return h ? `${h}h ${CT.p2(m % 60)}m` : `${m}m`;
  },
  elapsed(ms) {
    const s = Math.floor(ms / 1000);
    const h = Math.floor(s / 3600);
    return `${h}:${CT.p2(Math.floor((s % 3600) / 60))}:${CT.p2(s % 60)}`;
  },
  gb(bytes) {
    return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
  },
  /* Deterministic pseudo-waveform. The mocks have no audio peaks yet; the real
     build would read a per-segment peaks file written when the segment closes. */
  wave(seed, n, quietFrom = Infinity, quietTo = -Infinity) {
    let x = seed * 9301 + 49297;
    const out = [];
    for (let i = 0; i < n; i++) {
      x = (x * 9301 + 49297) % 233280;
      const r = x / 233280;
      const burst = Math.sin(i / 7 + seed) * 0.5 + 0.5;
      let v = 0.08 + r * 0.55 * burst + (r > 0.93 ? 0.35 : 0);
      if (i >= quietFrom && i < quietTo) v *= 0.15;
      out.push(Math.min(1, v));
    }
    return out;
  },
  el(html) {
    const t = document.createElement("template");
    t.innerHTML = html.trim();
    return t.content.firstChild;
  },
};

/* The ChirpTime mark: a record dot that chirps. The dot is the status. */
CT.mark = (size = 18, dot = "var(--ink)", arcs = "var(--ink-3)") => `
<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" aria-hidden="true">
  <circle cx="9" cy="12" r="5.2" fill="${dot}"/>
  <path d="M16.2 8.6a5 5 0 010 6.8" stroke="${arcs}" stroke-width="2" stroke-linecap="round"/>
  <path d="M19.4 5.8a9 9 0 010 12.4" stroke="${arcs}" stroke-width="2" stroke-linecap="round" opacity=".55"/>
</svg>`;

/* Placeholder mode (?ph=1): every footage still becomes a neutral frame, for
   anything shared beyond this machine. Screens get drawn window chrome and a
   label; the camera gets a silhouette. */
(function () {
  if (!new URLSearchParams(location.search).get("ph")) return;
  const NAMES = { "screen-0": ["Screen 1", "editor"], "screen-1": ["Screen 2", "browser"], "screen-2": ["Screen 3", "terminal"], "screen-3": ["Screen 4", "chat"] };
  const kindOf = (src) => {
    if (/webcam/.test(src)) return ["Camera", ""];
    const m = src.match(/screen-[0-3]/);
    return m ? NAMES[m[0]] : ["Screen", "app"];
  };
  const bars = (n, seed, w, color, x0, y0, gapY, h) => {
    let out = "", s = seed;
    for (let i = 0; i < n; i++) {
      s = (s * 9301 + 49297) % 233280;
      const len = 0.35 + (s / 233280) * 0.55;
      out += `<span style="position:absolute;left:${x0}px;top:${y0 + i * gapY}px;width:${Math.round(w * len)}px;height:${h}px;border-radius:${h / 2}px;background:${color}"></span>`;
    }
    return out;
  };
  const screenTile = (w, h, name, hint, seed) => {
    const bar = Math.max(3, Math.round(h * 0.07));
    let html = `<span style="position:absolute;left:0;top:0;width:${w}px;height:${bar}px;background:#3A3934"></span>`;
    if (w >= 160) html += [0, 1, 2].map((i) => `<span style="position:absolute;left:${6 + i * 7}px;top:${Math.round(bar / 2) - 2}px;width:4px;height:4px;border-radius:2px;background:rgba(243,240,232,.28)"></span>`).join("");
    const lineH = Math.min(6, Math.max(1, Math.round(h * 0.035))), gapY = Math.min(22, Math.max(3, Math.round(h * 0.09)));
    const rows = Math.max(1, Math.floor((h - bar - 10) / gapY) - 1);
    if (hint === "editor") {
      const side = Math.round(w * 0.18);
      html += `<span style="position:absolute;left:0;top:${bar}px;width:${side}px;height:${h - bar}px;background:#2C2B27"></span>`;
      html += bars(rows, seed, w - side - 16, "rgba(243,240,232,.13)", side + 8, bar + 8, gapY, lineH);
    } else if (hint === "browser") {
      const pad = Math.round(w * 0.14);
      html += `<span style="position:absolute;left:${pad}px;top:${bar + Math.round(h * 0.08)}px;width:${w - pad * 2}px;height:${h - bar - Math.round(h * 0.16)}px;border-radius:3px;background:#3A3934"></span>`;
      html += bars(Math.max(1, rows - 2), seed, w - pad * 2 - 20, "rgba(243,240,232,.16)", pad + 10, bar + Math.round(h * 0.08) + 8, gapY, lineH);
    } else if (hint === "terminal") {
      html += bars(rows, seed, w * 0.7, "rgba(190,214,170,.20)", 8, bar + 8, gapY, lineH);
    } else {
      const col = Math.round(w * 0.3);
      html += `<span style="position:absolute;left:0;top:${bar}px;width:${col}px;height:${h - bar}px;background:#2C2B27"></span>`;
      html += bars(rows, seed, w - col - 24, "rgba(243,240,232,.13)", col + 10, bar + 8, gapY, lineH);
    }
    if (w >= 180) html += `<span style="position:absolute;left:8px;bottom:6px;font:600 ${w >= 400 ? 13 : 10.5}px 'Schibsted Grotesk',sans-serif;color:rgba(243,240,232,.62);white-space:nowrap">${name} · ${hint}</span>`;
    return html;
  };
  // frames where the chair was empty on Aug 1 (10-minute index from 10:30)
  const AWAY = (src) => { const m = src.match(/webcam-(\d\d)/); if (!m) return false; const k = +m[1]; return (k >= 19 && k <= 28) || (k >= 39 && k <= 47); };
  const camTile = (w, h, empty) => {
    const s = Math.min(w, h) * 0.62;
    if (empty) return `<svg width="${Math.round(s)}" height="${Math.round(s)}" viewBox="0 0 24 24" style="position:absolute;left:${Math.round((w - s) / 2)}px;top:${Math.round(h - s * 0.95)}px" fill="none" stroke="rgba(243,240,232,.16)" stroke-width="1.6" stroke-linecap="round"><path d="M7 4h10v9H7z"/><path d="M5 13h14M8 13v9M16 13v9"/></svg>`;
    let html = `<svg width="${Math.round(s)}" height="${Math.round(s)}" viewBox="0 0 24 24" style="position:absolute;left:${Math.round((w - s) / 2)}px;top:${Math.round(h - s * 0.92)}px" fill="rgba(243,240,232,.16)"><circle cx="12" cy="8" r="4.2"/><path d="M3.5 24c0-5.2 3.8-8.6 8.5-8.6s8.5 3.4 8.5 8.6z"/></svg>`;
    return html;
  };
  window.addEventListener("load", () => {
    let seed = 7;
    for (const img of [...document.querySelectorAll('img[src*="assets/"]')]) {
      const w = img.offsetWidth, h = img.offsetHeight;
      const [name, hint] = kindOf(img.getAttribute("src"));
      const d = document.createElement("div");
      d.style.cssText = img.style.cssText;
      const cs = getComputedStyle(img);
      if (cs.position === "static") d.style.position = "relative";
      d.style.width = w + "px"; d.style.height = h + "px";
      d.style.overflow = "hidden"; d.style.display = "block"; d.style.flexShrink = "0";
      d.style.borderRadius = cs.borderRadius;
      d.style.background = name === "Camera" ? "#2A2925" : "#24231F";
      d.innerHTML = name === "Camera" ? camTile(w, h, AWAY(img.getAttribute("src"))) : screenTile(w, h, name, hint, seed += 13);
      img.replaceWith(d);
    }
    window.__PH_DONE = true;
  });
})();
