/**
 * Build-time drawing.
 *
 * Everything the page shows — the day ribbon in the hero, the player, the shelf
 * of past days — is generated here and baked into the HTML, so the page draws
 * itself in full before a single line of script runs.
 */

import {
  CATALOG,
  clock,
  DAY_END,
  DAY_SECONDS,
  DAY_START,
  hourLabel,
  LANES,
  MOMENTS,
  RULER_HOURS,
  seeded,
  type Lane,
} from "./day";

const pct = (n: number) => `${(n * 100).toFixed(3)}%`;

/* ─────────────────────────────────────────────────────── the day's light ──
   Every strip of frames on the page is coloured by the hour it was taken:
   grey-blue first light, cream through the morning, bright at noon, gold in
   the afternoon, terracotta at dusk. It is the one thing that makes a row of
   little rectangles read as a day rather than a chart. */

const LIGHT: { at: number; rgb: [number, number, number] }[] = [
  { at: 0.0, rgb: [196, 183, 172] },
  { at: 0.14, rgb: [233, 216, 196] },
  { at: 0.4, rgb: [250, 239, 222] },
  { at: 0.6, rgb: [245, 224, 196] },
  { at: 0.78, rgb: [223, 168, 124] },
  { at: 0.91, rgb: [196, 124, 86] },
  { at: 1.0, rgb: [150, 104, 82] },
];

function daylight(t: number, jitter: number): string {
  const x = Math.min(1, Math.max(0, t));
  let a = LIGHT[0]!;
  let b = LIGHT[LIGHT.length - 1]!;
  for (let i = 0; i < LIGHT.length - 1; i++) {
    const lo = LIGHT[i]!;
    const hi = LIGHT[i + 1]!;
    if (x >= lo.at && x <= hi.at) {
      a = lo;
      b = hi;
      break;
    }
  }
  const k = b.at === a.at ? 0 : (x - a.at) / (b.at - a.at);
  const shift = (jitter - 0.5) * 26;
  const c = a.rgb.map((v, i) => Math.round(Math.min(255, Math.max(60, v + (b.rgb[i]! - v) * k + shift))));
  return `rgb(${c[0]} ${c[1]} ${c[2]})`;
}

/** A strip of tiny frames — the visual shorthand for "a whole day". */
function frames(count: number, seed: number, from = 0, to = 1): string {
  const rand = seeded(seed);
  let out = "";
  for (let i = 0; i < count; i++) {
    const t = from + ((i + 0.5) / count) * (to - from);
    out += `<i style="background:${daylight(t, rand())}"></i>`;
  }
  return out;
}

/* ───────────────────────────────────────────────────────── hero ribbon ── */

export function renderRibbon(): string {
  const marks = MOMENTS.map(
    (m) =>
      `<span class="ribbon__mark" style="left:${pct(m.at)}"><svg viewBox="0 0 10 10" aria-hidden="true"><path d="M5 0 10 5 5 10 0 5Z"/></svg></span>`,
  ).join("");

  return `
  <figure class="ribbon" aria-labelledby="ribbon-cap">
    <div class="ribbon__strip">
      <div class="ribbon__frames">${frames(112, 0x51a7)}</div>
      ${marks}
      <span class="ribbon__head" aria-hidden="true"></span>
    </div>
    <figcaption class="ribbon__foot" id="ribbon-cap">
      <span>Tue 14 Oct</span>
      <span class="ribbon__span">7:12 AM <em>&mdash; one day, kept whole &mdash;</em> 11:04 PM</span>
      <span>15h 52m</span>
    </figcaption>
  </figure>`;
}

/* ─────────────────────────────────────────────────────── the picture ──
   Two monitors and a camera, drawn out of focus on purpose: enough shape to
   recognise your own afternoon, not enough to pretend it is a screenshot. */

/** Somebody, out of focus. Every one is proportioned a little differently so a
    row of them reads as people rather than as the same placeholder repeated. */
function figure(cls: string, seed: number): string {
  const rand = seeded(seed);
  const cx = 60 + (rand() - 0.5) * 30;
  const cy = 32 + rand() * 9;
  const r = 13 + rand() * 6;
  const tilt = (rand() - 0.5) * 14;
  const rx = r * (2.05 + rand() * 0.6);
  const ry = r * (1.6 + rand() * 0.6);
  const sy = cy + r * (1.9 + rand() * 0.4) + ry;
  return (
    `<svg class="${cls}" viewBox="0 0 120 90" preserveAspectRatio="xMidYMid slice" aria-hidden="true">` +
    `<g transform="rotate(${tilt.toFixed(1)} ${cx.toFixed(1)} ${(cy + r).toFixed(1)})">` +
    `<ellipse cx="${cx.toFixed(1)}" cy="${sy.toFixed(1)}" rx="${rx.toFixed(1)}" ry="${ry.toFixed(1)}"/>` +
    `<circle cx="${cx.toFixed(1)}" cy="${cy.toFixed(1)}" r="${r.toFixed(1)}"/>` +
    `</g></svg>`
  );
}

function renderStage(): string {
  const rand = seeded(0x2b7d);

  let doc = "";
  for (let i = 0; i < 36; i++) {
    if (i === 7) {
      doc += `<span class="doc__pic"></span>`;
      continue;
    }
    const w = 26 + rand() * 68;
    const accent = rand() < 0.22 ? " class=\"is-accent\"" : "";
    doc += `<i${accent} style="width:${w.toFixed(1)}%"></i>`;
  }

  let side = "";
  for (let i = 0; i < 14; i++) {
    side += `<i style="width:${(34 + rand() * 58).toFixed(1)}%;margin-left:${rand() < 0.45 ? 14 : 0}%"></i>`;
  }

  const faceTints = ["#f7e0c8,#e6c19c", "#f0dcd0,#d6b39c", "#f6e7cd,#dfbf99", "#ecd9c6,#d2b399"];
  const faces = faceTints
    .map(
      (tint, i) =>
        `<span class="face${i === 1 ? " is-live" : ""}" style="--tint:linear-gradient(158deg,${tint})">${figure("face__fig", 0x3f1 + i * 271)}</span>`,
    )
    .join("");

  return `
    <div class="stage">
      <div class="tile tile--main">
        <span class="tile__bar"><b></b><b></b><b></b><em></em></span>
        <div class="tile__work">
          <div class="tile__side">${side}</div>
          <div class="tile__doc">${doc}</div>
        </div>
      </div>

      <div class="tile tile--call">
        <span class="tile__bar"><b></b><b></b><b></b><em></em></span>
        <div class="call">${faces}</div>
        <div class="call__bar"><i></i><i></i><i class="is-accent"></i></div>
      </div>

      <div class="cam" data-role="cam">
        ${figure("cam__fig", 0x8ac3)}
        <span class="cam__off">camera was off</span>
        <span class="cam__dot" aria-hidden="true"></span>
      </div>

      <div class="stage__dawn" aria-hidden="true"></div>
      <div class="stage__dusk" aria-hidden="true"></div>
      <div class="stage__sheen" aria-hidden="true"></div>
    </div>`;
}

/* ────────────────────────────────────────────────────────── the timeline ── */

function laneBody(lane: Lane, index: number): string {
  return lane.runs
    .map(([from, to], r) => {
      const width = to - from;
      const style = `left:${pct(from)};width:${pct(width)}`;
      const seed = 0x9e37 + index * 7919 + r * 131;

      if (lane.kind === "voice") {
        return `<span class="run run--voice" style="${style}">${wave(Math.max(24, Math.round(260 * width)), seed, index)}</span>`;
      }
      if (lane.kind === "camera") {
        // A smooth wash of the day's light, so the face lane warms up toward
        // evening the same way the screens do.
        const stops = [0.02, 0.35, 0.68, 1]
          .map((k) => daylight(from + (to - from) * k, 0.5))
          .join(",");
        return `<span class="run run--camera" style="${style};background-image:linear-gradient(90deg,${stops})"></span>`;
      }
      return `<span class="run run--screen" style="${style}">${frames(Math.max(6, Math.round(96 * width)), seed, from, to)}</span>`;
    })
    .join("");
}

/** A stretch of talking, drawn as a mirrored waveform. */
function wave(n: number, seed: number, phase: number): string {
  const rand = seeded(seed);
  const top: string[] = [];
  const bottom: string[] = [];
  for (let i = 0; i <= n; i++) {
    const x = ((i / n) * 100).toFixed(3);
    const env = 0.3 + 0.7 * Math.abs(Math.sin(i * 0.21 + phase)) ** 1.6;
    const a = Math.min(1, env * (0.42 + rand() * 0.66));
    top.push(`${x},${(10 - a * 9).toFixed(2)}`);
    bottom.unshift(`${x},${(10 + a * 9).toFixed(2)}`);
  }
  return `<svg viewBox="0 0 100 20" preserveAspectRatio="none" aria-hidden="true"><polygon points="${top.join(" ")} ${bottom.join(" ")}"/></svg>`;
}

/** The wide voice card in section 02. */
export function renderVoiceCard(): string {
  return `<div class="card__wave">${wave(150, 0x77c1, 2)}</div>`;
}

export function renderPlayer(): string {
  const start = 0.565; // opens in the middle of the afternoon

  const rows = LANES.map(
    (lane, i) => `
      <div class="tl__name" data-lane="${lane.id}">${lane.name}</div>
      <div class="tl__lane tl__lane--${lane.kind}">${laneBody(lane, i)}</div>`,
  ).join("");

  const ruler = RULER_HOURS.map((h) => {
    const at = (h * 3600 - DAY_START) / DAY_SECONDS;
    return `<span class="tl__hour" style="left:${pct(at)}"><i></i>${hourLabel(h)}</span>`;
  }).join("");

  const marks = MOMENTS.map(
    (m, i) => `
      <span class="tl__mark tl__mark--${i % 2 ? "down" : "up"}" style="left:${pct(m.at)}">
        <b>${m.label}</b>
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M5 0 10 5 5 10 0 5Z"/></svg>
      </span>`,
  ).join("");

  const legend = MOMENTS.map(
    (m) =>
      `<li><svg viewBox="0 0 10 10" aria-hidden="true"><path d="M5 0 10 5 5 10 0 5Z"/></svg>${m.label}<span>${clock(DAY_START + m.at * DAY_SECONDS)}</span></li>`,
  ).join("");

  return `
  <figure class="player" id="player" data-start="${DAY_START}" data-seconds="${DAY_SECONDS}" data-at="${start}">
    <div class="player__screen">
      ${renderStage()}
      <div class="player__bar">
        <p class="readout">
          <span class="readout__clock" data-role="clock">${clock(DAY_START + start * DAY_SECONDS)}</span>
          <span class="readout__date">Tuesday 14 October</span>
        </p>
        <div class="speeds" role="group" aria-label="Playback speed">
          <button type="button" class="speed" data-speed="0.5">0.5&times;</button>
          <button type="button" class="speed is-on" data-speed="1" aria-pressed="true">1&times;</button>
          <button type="button" class="speed" data-speed="6">6&times;</button>
        </div>
      </div>
    </div>

    <div class="tl">
      <div class="tl__grid">
        <div class="tl__marks">${marks}</div>
        ${rows}
        <div class="tl__ruler">${ruler}</div>
        <div class="tl__scrub"
             role="slider"
             tabindex="0"
             aria-label="Scrub through Tuesday 14 October"
             aria-valuemin="0"
             aria-valuemax="100"
             aria-valuenow="${Math.round(start * 100)}"
             aria-valuetext="${clock(DAY_START + start * DAY_SECONDS)}">
          <span class="tl__head"><i></i></span>
        </div>
      </div>
      <div class="tl__ends" aria-hidden="true">
        <span>${clock(DAY_START)}</span>
        <span>${clock(DAY_END)}</span>
      </div>
      <ul class="tl__legend">${legend}</ul>
    </div>
  </figure>`;
}

/* ───────────────────────────────────────────────────────────── catalog ── */

export function renderCatalog(): string {
  return CATALOG.map((day, i) => {
    const breaks = day.breaks
      .map(([from, to]) => `<span class="day__break" style="left:${pct(from)};width:${pct(to - from)}"></span>`)
      .join("");
    const marks = day.marks
      .map(
        (m) =>
          `<span class="day__mark" style="left:${pct(m.at)}"><svg viewBox="0 0 10 10" aria-hidden="true"><path d="M5 0 10 5 5 10 0 5Z"/></svg></span>`,
      )
      .join("");
    const kept = day.marks.map((m) => `<li>${m.label}</li>`).join("");

    return `
    <li class="day" data-reveal="${i % 3}">
      <a class="day__link" href="#playback">
        <span class="day__head">
          <span class="day__date"><em>${day.weekday}</em> ${day.date}</span>
          <span class="day__hours">${day.hours}</span>
        </span>
        <span class="day__strip">${frames(38, 0x4d21 + i * 613)}${breaks}${marks}</span>
        <span class="day__span">${day.span}</span>
        <ul class="day__kept">${kept}</ul>
      </a>
    </li>`;
  }).join("");
}
