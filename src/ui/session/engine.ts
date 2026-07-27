/**
 * Multi-track synchronised playback engine.
 *
 * Layer 3 of the sync strategy (plans/2026-07-27-continuous-recording.md §4):
 * a single master virtual clock in epoch-ms drives N independent <video>/<audio>
 * elements. Each element is held at
 *
 *     currentTime = (masterEpochMs - segment.start_epoch_ms) / 1000
 *
 * by a resync loop that runs every RESYNC_INTERVAL_MS. N media elements each
 * have their own clock and WILL drift apart without this, no matter how well the
 * files line up on disk.
 *
 * Each track owns TWO media elements. One is visible and playing; the other
 * pre-rolls the next 60s segment so a boundary crossing is a swap, not a load.
 */
import type { SessionManifest, SessionTrack } from "../types";
import { mediaUrl } from "../hooks/mediaUrl";

export const RESYNC_INTERVAL_MS = 250;
/** |drift| above this → hard seek */
export const HARD_SEEK_MS = 150;
/** |drift| above this (and below HARD_SEEK_MS) → playbackRate nudge */
export const SOFT_NUDGE_MS = 30;
/** how hard to nudge, as a fraction of the current rate */
export const NUDGE_RATIO = 0.03;
/** start pre-rolling the next segment this long before the boundary */
export const PRELOAD_LEAD_MS = 8000;
/** while paused, snap rather than nudge */
export const PAUSED_SNAP_MS = 20;
/**
 * Minimum wall-clock gap between hard seeks on one element, so a seek issued at
 * one resync tick is not re-issued at the next before the decoder has answered.
 */
export const HARD_SEEK_COOLDOWN_MS = 300;
/** how often React state is refreshed from the engine */
export const STATE_INTERVAL_MS = 200;

export type TrackState = "pending" | "playing" | "gap" | "ended" | "unreadable";

export interface TrackDebug {
  id: string;
  state: TrackState;
  /** index into track.segments, or -1 */
  segIndex: number;
  /** el.currentTime - desired, in ms; positive = element is ahead */
  driftMs: number;
  /** worst |drift| seen while playing since the last reset */
  maxAbsDriftMs: number;
  /** where this element believes it is, in session epoch-ms */
  displayedEpochMs: number | null;
  readyState: number;
  segmentSwaps: number;
  hardSeeks: number;
  nudges: number;
  /** stalls: swap happened with the standby not ready */
  coldSwitches: number;
}

export interface EngineSnapshot {
  masterEpochMs: number;
  playing: boolean;
  rate: number;
  /** max - min of displayedEpochMs across attached VIDEO tracks, in ms */
  spreadMs: number;
  maxSpreadMs: number;
  tracks: Record<string, TrackDebug>;
}

function emptyDebug(id: string): TrackDebug {
  return {
    id,
    state: "pending",
    segIndex: -1,
    driftMs: 0,
    maxAbsDriftMs: 0,
    displayedEpochMs: null,
    readyState: 0,
    segmentSwaps: 0,
    hardSeeks: 0,
    nudges: 0,
    coldSwitches: 0,
  };
}

export interface SegmentHit {
  index: number;
  offsetMs: number;
}

/**
 * A seam shorter than this is not a gap — it is FFmpeg landing a segment
 * boundary on a keyframe. Real recordings leave a hole at most seams (measured
 * across 6 sessions: 60 of 108 seams were 0–50ms, and 41 actually OVERLAPPED by
 * up to 4.2s). Treating a 33ms hole as missing footage blanks the tile for a
 * frame at every boundary; clamping to the adjacent segment is both invisible
 * and more truthful. Only a hole long enough to see is rendered as a gap.
 */
export const SEAM_TOLERANCE_MS = 250;

/**
 * Last segment whose [start, start+duration) contains epochMs, tolerating short
 * seams. Binary search, so it stays cheap over a 6-hour session.
 *
 * Everything is derived from each segment's own `start_epoch_ms` and
 * `duration_ms`. Nothing here assumes segments are uniform, contiguous, or
 * ordered-without-overlap in time — real durations are ragged (4s/14s/16s for a
 * nominal 15s) and consecutive segments can overlap.
 */
export function segmentAt(track: SessionTrack, epochMs: number): SegmentHit | null {
  const segs = track.segments;
  if (!segs.length) return null;

  let lo = 0;
  let hi = segs.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (segs[mid].start_epoch_ms <= epochMs) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }

  // before the track's first segment
  if (found < 0) {
    return segs[0].start_epoch_ms - epochMs <= SEAM_TOLERANCE_MS ? { index: 0, offsetMs: 0 } : null;
  }

  const seg = segs[found];
  const off = epochMs - seg.start_epoch_ms;
  if (off < seg.duration_ms) return { index: found, offsetMs: off };

  // Past this segment's end. If the next one is about to start, roll onto it —
  // an overlap would already have been picked up by the search above, so this
  // only fires for genuine holes.
  const next = segs[found + 1];
  if (next && next.start_epoch_ms - epochMs <= SEAM_TOLERANCE_MS) {
    return { index: found + 1, offsetMs: 0 };
  }
  return null; // a real gap, or past the end of the track
}

/** First segment starting at or after epochMs, or -1. */
export function nextSegmentIndex(track: SessionTrack, epochMs: number): number {
  const segs = track.segments;
  for (let i = 0; i < segs.length; i++) {
    if (segs[i].start_epoch_ms + segs[i].duration_ms > epochMs) return i;
  }
  return -1;
}

export function trackStartEpochMs(track: SessionTrack): number {
  return track.segments.length ? track.segments[0].start_epoch_ms : track.start_epoch_ms;
}

export function trackEndEpochMs(track: SessionTrack): number {
  if (!track.segments.length) return track.start_epoch_ms;
  const last = track.segments[track.segments.length - 1];
  return last.start_epoch_ms + last.duration_ms;
}

export function sessionBounds(m: SessionManifest): { start: number; end: number } {
  const starts = m.tracks.filter((t) => t.segments.length).map(trackStartEpochMs);
  const ends = m.tracks.filter((t) => t.segments.length).map(trackEndEpochMs);
  const start = starts.length ? Math.min(m.started_epoch_ms, ...starts) : m.started_epoch_ms;
  const end = ends.length
    ? Math.max(m.ended_epoch_ms ?? 0, ...ends)
    : (m.ended_epoch_ms ?? m.started_epoch_ms);
  return { start, end: Math.max(end, start + 1) };
}

export function isVideoTrack(t: SessionTrack): boolean {
  return t.kind === "screen" || t.kind === "webcam";
}

export function segmentUrl(sessionId: string, track: SessionTrack, segIndex: number): string {
  return mediaUrl(`sessions/${sessionId}/${track.dir}/${track.segments[segIndex].file}`);
}

/* ------------------------------------------------------------------ */

class TrackController {
  readonly track: SessionTrack;
  readonly debug: TrackDebug;
  private sessionId: string;
  private els: [HTMLMediaElement, HTMLMediaElement] | null = null;
  /** which segment index each element currently holds; -1 = nothing */
  private elSeg: [number, number] = [-1, -1];
  private activeIdx: 0 | 1 = 0;
  private activeSeg = -1;
  private muted = true;
  private volume = 1;
  private endEpoch: number;
  private startEpoch: number;
  private lastHardSeekAt = 0;

  constructor(sessionId: string, track: SessionTrack) {
    this.sessionId = sessionId;
    this.track = track;
    this.debug = emptyDebug(track.id);
    this.endEpoch = trackEndEpochMs(track);
    this.startEpoch = trackStartEpochMs(track);
  }

  attach(a: HTMLMediaElement, b: HTMLMediaElement) {
    this.els = [a, b];
    this.elSeg = [-1, -1];
    this.activeIdx = 0;
    this.activeSeg = -1;
    for (const el of this.els) {
      el.preload = "auto";
      el.muted = this.muted;
      el.volume = this.volume;
      // never let the browser's own end-of-media handling fight the clock
      el.loop = false;
    }
    this.applyVisibility();
  }

  detach() {
    if (this.els) for (const el of this.els) el.pause();
    this.els = null;
    this.elSeg = [-1, -1];
    this.activeSeg = -1;
    this.debug.segIndex = -1;
    this.debug.displayedEpochMs = null;
  }

  get attached() {
    return this.els !== null;
  }

  setAudio(muted: boolean, volume: number) {
    this.muted = muted;
    this.volume = volume;
    if (this.els) {
      for (const el of this.els) {
        el.volume = volume;
      }
      // only the visible element is ever unmuted
      this.applyVisibility();
    }
  }

  /** a deliberate jump is not drift — let it correct immediately */
  clearSeekCooldown() {
    this.lastHardSeekAt = 0;
  }

  resetMetrics() {
    this.debug.maxAbsDriftMs = 0;
    this.debug.segmentSwaps = 0;
    this.debug.hardSeeks = 0;
    this.debug.nudges = 0;
    this.debug.coldSwitches = 0;
  }

  private applyVisibility() {
    if (!this.els) return;
    this.els.forEach((el, i) => {
      const active = i === this.activeIdx;
      el.style.opacity = active ? "1" : "0";
      el.style.pointerEvents = "none";
      el.style.zIndex = active ? "1" : "0";
      el.muted = active ? this.muted : true;
    });
  }

  private loadInto(which: 0 | 1, segIndex: number) {
    if (!this.els) return;
    if (this.elSeg[which] === segIndex) return;
    const el = this.els[which];
    el.src = segmentUrl(this.sessionId, this.track, segIndex);
    this.elSeg[which] = segIndex;
    // el.load() is implied by setting src; forcing it again cancels the fetch
  }

  private seek(el: HTMLMediaElement, seconds: number) {
    const clamped = Math.max(0, seconds);
    try {
      el.currentTime = clamped;
    } catch {
      // readyState HAVE_NOTHING on some paths; the next resync tick retries
    }
  }

  /**
   * @param resync run the drift correction this tick (gated to ~250ms).
   *               Segment boundaries and play/pause are handled every frame.
   */
  tick(master: number, playing: boolean, rate: number, resync: boolean) {
    const els = this.els;
    const d = this.debug;
    if (!els) return;

    const hit = segmentAt(this.track, master);

    if (!hit) {
      d.state = master >= this.endEpoch ? "ended" : master < this.startEpoch ? "pending" : "gap";
      d.segIndex = -1;
      d.driftMs = 0;
      d.displayedEpochMs = null;
      for (const el of els) if (!el.paused) el.pause();
      // Park the upcoming segment on the active element so resuming from a gap
      // (or from before the track started) is instant.
      const upcoming = nextSegmentIndex(this.track, master);
      if (upcoming >= 0 && this.elSeg[this.activeIdx] !== upcoming) {
        this.loadInto(this.activeIdx, upcoming);
        this.activeSeg = -1;
        this.seek(els[this.activeIdx], 0);
      }
      d.readyState = els[this.activeIdx].readyState;
      return;
    }

    d.segIndex = hit.index;

    /* ---- a segment the browser cannot decode ---- *
     * A force-killed recording leaves a final segment with no moov atom. The
     * manifest normally excludes it, but a rebuild pass may not — say so
     * instead of showing a black tile, and leave the other tracks alone: the
     * spread metric ignores any element below readyState 2, so one broken
     * segment cannot drag the rest out of sync. */
    if (els[this.activeIdx].error && this.elSeg[this.activeIdx] === hit.index) {
      d.state = "unreadable";
      d.driftMs = 0;
      d.displayedEpochMs = null;
      for (const el of els) if (!el.paused) el.pause();
      return;
    }

    d.state = "playing";

    /* ---- segment boundary ---- */
    if (this.activeSeg !== hit.index) {
      const other = (this.activeIdx ^ 1) as 0 | 1;
      if (this.elSeg[other] === hit.index && els[other].readyState >= 2) {
        // pre-rolled: swap, no load
        els[this.activeIdx].pause();
        this.activeIdx = other;
        d.segmentSwaps++;
      } else {
        // a seek, or a pre-roll that did not arrive in time
        this.loadInto(this.activeIdx, hit.index);
        // only a natural boundary that missed its pre-roll is a stall; jumping
        // to an unrelated segment because the user scrubbed is not
        if (hit.index === this.activeSeg + 1) d.coldSwitches++;
      }
      this.activeSeg = hit.index;
      this.applyVisibility();
      this.seek(els[this.activeIdx], hit.offsetMs / 1000);
    }

    const el = els[this.activeIdx];
    const desired = hit.offsetMs / 1000;
    d.readyState = el.readyState;

    /* ---- drift correction ---- */
    if (el.readyState >= 1) {
      const drift = (el.currentTime - desired) * 1000;
      d.driftMs = drift;
      d.displayedEpochMs = this.track.segments[hit.index].start_epoch_ms + el.currentTime * 1000;

      if (!playing) {
        if (Math.abs(drift) > PAUSED_SNAP_MS) this.seek(el, desired);
        el.playbackRate = 1;
      } else if (resync) {
        if (el.readyState >= 2) {
          d.maxAbsDriftMs = Math.max(d.maxAbsDriftMs, Math.abs(drift));
        }
        const a = Math.abs(drift);
        const now = performance.now();
        if (a > HARD_SEEK_MS) {
          // Cooldown only guards against seeking twice inside one decode
          // round-trip. It is deliberately shorter than the resync interval's
          // recovery budget: staying aligned outranks staying smooth.
          if (now - this.lastHardSeekAt >= HARD_SEEK_COOLDOWN_MS) {
            this.seek(el, desired);
            this.lastHardSeekAt = now;
            d.hardSeeks++;
          }
          el.playbackRate = rate;
        } else if (a > SOFT_NUDGE_MS) {
          // element ahead → slow it down; behind → speed it up
          el.playbackRate = rate * (drift > 0 ? 1 - NUDGE_RATIO : 1 + NUDGE_RATIO);
          d.nudges++;
        } else {
          el.playbackRate = rate;
        }
      }
    } else {
      d.displayedEpochMs = null;
    }

    /* ---- transport ---- */
    if (playing) {
      /* An ended element must be left alone: per spec, play() at end-of-media
       * seeks back to the start, which flashes the segment's first frame. This
       * is reachable normally — the nudge band tolerates running up to
       * HARD_SEEK_MS ahead, so an element can hit `ended` in a segment's last
       * ~150ms. Hold it here and let the boundary swap take over. */
      if (el.paused && !el.ended) void el.play().catch(() => {});
    } else if (!el.paused) {
      el.pause();
    }
    const standby = els[this.activeIdx ^ 1];
    if (!standby.paused) standby.pause();

    /* ---- pre-roll the next segment ---- */
    if (playing && hit.index + 1 < this.track.segments.length) {
      const seg = this.track.segments[hit.index];
      const remaining = seg.start_epoch_ms + seg.duration_ms - master;
      if (remaining <= PRELOAD_LEAD_MS) {
        const other = (this.activeIdx ^ 1) as 0 | 1;
        if (this.elSeg[other] !== hit.index + 1) {
          this.loadInto(other, hit.index + 1);
          this.seek(els[other], 0);
        }
      }
    }
  }
}

/* ------------------------------------------------------------------ */

export class SyncEngine {
  readonly manifest: SessionManifest;
  readonly start: number;
  readonly end: number;

  masterEpochMs: number;
  playing = false;
  rate = 1;

  private controllers = new Map<string, TrackController>();
  private raf = 0;
  private lastPerf = 0;
  private lastResync = 0;
  private lastState = 0;
  private frameSubs = new Set<(s: EngineSnapshot) => void>();
  private stateSubs = new Set<(s: EngineSnapshot) => void>();
  private maxSpreadMs = 0;
  private spreadMs = 0;
  /**
   * Which tracks are allowed to make noise. More than one may be unmuted at
   * once — mic and system audio played together is the point of "All", so you
   * hear what you said AND what you were listening to. Empty set = silent.
   */
  private audioIds = new Set<string>();
  private volume = 1;

  constructor(manifest: SessionManifest) {
    this.manifest = manifest;
    const b = sessionBounds(manifest);
    this.start = b.start;
    this.end = b.end;
    this.masterEpochMs = b.start;
    for (const t of manifest.tracks) {
      this.controllers.set(t.id, new TrackController(manifest.id, t));
    }
  }

  /* ---- lifecycle ---- */

  startLoop() {
    if (this.raf) return;
    this.lastPerf = performance.now();
    this.lastResync = 0;
    const loop = (now: number) => {
      this.raf = requestAnimationFrame(loop);
      this.step(now);
    };
    this.raf = requestAnimationFrame(loop);
  }

  stopLoop() {
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
    for (const c of this.controllers.values()) c.detach();
  }

  /* ---- track wiring ---- */

  attachTrack(id: string, a: HTMLMediaElement, b: HTMLMediaElement) {
    const c = this.controllers.get(id);
    if (!c) return;
    c.attach(a, b);
    this.applyAudioRouting();
    // seed immediately so a freshly mounted surface shows the right frame
    c.tick(this.masterEpochMs, false, this.rate, true);
  }

  detachTrack(id: string) {
    this.controllers.get(id)?.detach();
  }

  /* ---- transport ---- */

  setPlaying(v: boolean) {
    if (v && this.masterEpochMs >= this.end - 20) this.masterEpochMs = this.start;
    this.playing = v;
    this.lastPerf = performance.now();
    this.tickAll(true);
    this.emitState();
  }

  togglePlaying() {
    this.setPlaying(!this.playing);
  }

  setRate(r: number) {
    this.rate = r;
    this.tickAll(true);
    this.emitState();
  }

  seekTo(epochMs: number) {
    this.masterEpochMs = Math.min(this.end, Math.max(this.start, epochMs));
    this.lastPerf = performance.now();
    for (const c of this.controllers.values()) c.clearSeekCooldown();
    this.tickAll(true);
    this.emitFrame();
    this.emitState();
  }

  nudgeSeconds(delta: number) {
    this.seekTo(this.masterEpochMs + delta * 1000);
  }

  /** Unmute exactly these tracks; everything else goes silent. */
  setAudioTracks(ids: Iterable<string>) {
    this.audioIds = new Set(ids);
    this.applyAudioRouting();
    this.emitState();
  }

  /** Convenience for the single-source case; `null` mutes everything. */
  setAudioTrack(id: string | null) {
    this.setAudioTracks(id === null ? [] : [id]);
  }

  getAudioTracks(): string[] {
    return [...this.audioIds];
  }

  isAudible(id: string) {
    return this.audioIds.has(id);
  }

  setVolume(v: number) {
    this.volume = v;
    this.applyAudioRouting();
  }

  getVolume() {
    return this.volume;
  }

  private applyAudioRouting() {
    /*
     * Mixing N sources at full volume can clip, so scale back as more are
     * unmuted — but sub-linearly (1/sqrt(n)), because equal-loudness for
     * uncorrelated sources goes as the square root, not the count. Two tracks
     * at 1/2 each sounds like someone turned it down; at 1/sqrt(2) it doesn't.
     */
    const n = this.audioIds.size;
    const vol = n > 1 ? this.volume / Math.sqrt(n) : this.volume;
    for (const [id, c] of this.controllers) {
      c.setAudio(!this.audioIds.has(id), vol);
    }
  }

  resetMetrics() {
    this.maxSpreadMs = 0;
    for (const c of this.controllers.values()) c.resetMetrics();
    this.emitState();
  }

  /* ---- the loop ---- */

  private step(now: number) {
    const dt = now - this.lastPerf;
    this.lastPerf = now;

    if (this.playing) {
      this.masterEpochMs += dt * this.rate;
      if (this.masterEpochMs >= this.end) {
        this.masterEpochMs = this.end;
        this.playing = false;
        this.tickAll(true);
        this.emitState();
        this.emitFrame();
        return;
      }
    }

    const resync = now - this.lastResync >= RESYNC_INTERVAL_MS;
    if (resync) this.lastResync = now;
    this.tickAll(resync);

    this.emitFrame();
    if (now - this.lastState >= STATE_INTERVAL_MS) {
      this.lastState = now;
      this.emitState();
    }
  }

  private tickAll(resync: boolean) {
    for (const c of this.controllers.values()) {
      c.tick(this.masterEpochMs, this.playing, this.rate, resync);
    }
    this.measureSpread();
  }

  /**
   * The honest sync metric: where each attached VIDEO element actually believes
   * it is, in session epoch-ms. If the burned-in timecodes agree on screen, this
   * number is small. Measured before the next correction is applied.
   */
  private measureSpread() {
    let min = Infinity;
    let max = -Infinity;
    let n = 0;
    for (const c of this.controllers.values()) {
      if (!c.attached || !isVideoTrack(c.track)) continue;
      const d = c.debug;
      if (d.state !== "playing" || d.displayedEpochMs === null || d.readyState < 2) continue;
      n++;
      if (d.displayedEpochMs < min) min = d.displayedEpochMs;
      if (d.displayedEpochMs > max) max = d.displayedEpochMs;
    }
    if (n < 2) {
      this.spreadMs = 0;
      return;
    }
    this.spreadMs = max - min;
    if (this.playing) this.maxSpreadMs = Math.max(this.maxSpreadMs, this.spreadMs);
  }

  /* ---- subscriptions ---- */

  snapshot(): EngineSnapshot {
    const tracks: Record<string, TrackDebug> = {};
    for (const [id, c] of this.controllers) tracks[id] = { ...c.debug };
    return {
      masterEpochMs: this.masterEpochMs,
      playing: this.playing,
      rate: this.rate,
      spreadMs: this.spreadMs,
      maxSpreadMs: this.maxSpreadMs,
      tracks,
    };
  }

  /** every animation frame — for imperative consumers (playhead, clock readout) */
  subscribeFrame(cb: (s: EngineSnapshot) => void): () => void {
    this.frameSubs.add(cb);
    return () => {
      this.frameSubs.delete(cb);
    };
  }

  /** ~5Hz — for React state */
  subscribeState(cb: (s: EngineSnapshot) => void): () => void {
    this.stateSubs.add(cb);
    return () => {
      this.stateSubs.delete(cb);
    };
  }

  private emitFrame() {
    if (!this.frameSubs.size) return;
    const s = this.snapshot();
    for (const cb of this.frameSubs) cb(s);
  }

  private emitState() {
    if (!this.stateSubs.size) return;
    const s = this.snapshot();
    for (const cb of this.stateSubs) cb(s);
  }
}
