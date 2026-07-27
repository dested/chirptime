/**
 * One ordinary Tuesday, described just well enough to draw it.
 *
 * Everything here exists so the page can *show* what a kept day looks like —
 * when it started, when it ended, where the camera was off, and the handful of
 * moments somebody marked while they happened. It is an illustration, not a
 * reading of a real recording.
 */

export const DAY_START = 7 * 3600 + 12 * 60; // 07:12
export const DAY_END = 23 * 3600 + 4 * 60; // 23:04
export const DAY_SECONDS = DAY_END - DAY_START;

export type LaneKind = "screen" | "camera" | "voice";

export interface Lane {
  id: string;
  name: string;
  kind: LaneKind;
  /** Stretches of footage, as [from, to] fractions of the day. */
  runs: [number, number][];
}

export interface Moment {
  /** Fraction of the day. */
  at: number;
  label: string;
}

/** Seconds past midnight → fraction of the visible day. */
function at(hour: number, minute: number): number {
  return (hour * 3600 + minute * 60 - DAY_START) / DAY_SECONDS;
}

export const LANES: Lane[] = [
  { id: "screen-a", name: "Main screen", kind: "screen", runs: [[0, 1]] },
  {
    id: "screen-b",
    name: "Second screen",
    kind: "screen",
    runs: [
      [0, at(12, 41)],
      [at(13, 26), 1],
    ],
  },
  {
    id: "camera",
    name: "Camera",
    kind: "camera",
    runs: [
      [0, at(12, 41)],
      [at(13, 26), at(19, 5)],
      [at(19, 48), 1],
    ],
  },
  { id: "voice", name: "Voice", kind: "voice", runs: [[0, 1]] },
];

/** The moments somebody marked as the day went past. */
export const MOMENTS: Moment[] = [
  { at: at(9, 38), label: "call went well" },
  { at: at(15, 2), label: "bug finally broke" },
  { at: at(17, 26), label: "kid wandered in" },
];

/** Hours to label along the ruler. */
export const RULER_HOURS = [8, 10, 12, 14, 16, 18, 20, 22];

/** Deterministic PRNG, so every build draws exactly the same day. */
export function seeded(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function clock(secondsPastMidnight: number): string {
  const s = Math.floor(secondsPastMidnight) % 86400;
  const h24 = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const h = h24 % 12 === 0 ? 12 : h24 % 12;
  return `${h}:${String(m).padStart(2, "0")} ${h24 < 12 ? "AM" : "PM"}`;
}

export function hourLabel(hour: number): string {
  const h = hour % 12 === 0 ? 12 : hour % 12;
  return `${h} ${hour < 12 ? "AM" : "PM"}`;
}

/** The shelf in the catalog: recent days, each with what was marked in it. */
export interface CatalogDay {
  weekday: string;
  date: string;
  span: string;
  hours: string;
  /** Fractions of the strip where footage is missing. */
  breaks: [number, number][];
  marks: { at: number; label: string }[];
}

export const CATALOG: CatalogDay[] = [
  {
    weekday: "Tue",
    date: "14 Oct",
    span: "7:12 AM — 11:04 PM",
    hours: "15h 52m",
    breaks: [[0.36, 0.4]],
    marks: [
      { at: 0.16, label: "call went well" },
      { at: 0.49, label: "bug finally broke" },
      { at: 0.64, label: "kid wandered in" },
    ],
  },
  {
    weekday: "Mon",
    date: "13 Oct",
    span: "8:03 AM — 9:41 PM",
    hours: "13h 38m",
    breaks: [[0.52, 0.56]],
    marks: [{ at: 0.28, label: "the good idea" }],
  },
  {
    weekday: "Sun",
    date: "12 Oct",
    span: "10:22 AM — 6:15 PM",
    hours: "7h 53m",
    breaks: [],
    marks: [
      { at: 0.42, label: "she read the whole page" },
      { at: 0.78, label: "pancakes" },
    ],
  },
  {
    weekday: "Sat",
    date: "11 Oct",
    span: "9:47 AM — 11:58 PM",
    hours: "14h 11m",
    breaks: [
      [0.22, 0.29],
      [0.61, 0.64],
    ],
    marks: [{ at: 0.71, label: "both of them singing" }],
  },
  {
    weekday: "Fri",
    date: "10 Oct",
    span: "7:41 AM — 8:12 PM",
    hours: "12h 31m",
    breaks: [],
    marks: [
      { at: 0.34, label: "he said yes" },
      { at: 0.88, label: "first steps?" },
    ],
  },
  {
    weekday: "Thu",
    date: "9 Oct",
    span: "8:19 AM — 10:26 PM",
    hours: "14h 07m",
    breaks: [[0.44, 0.47]],
    marks: [{ at: 0.55, label: "worth keeping" }],
  },
];
