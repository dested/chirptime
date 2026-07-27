import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { SessionManifest, SessionTrack } from "../types";
import { SyncEngine, isVideoTrack, HARD_SEEK_MS, SOFT_NUDGE_MS } from "./engine";
import type { EngineSnapshot } from "./engine";
import { TrackSurface } from "./TrackSurface";
import { Timeline } from "./Timeline";
import { fmtBytes, fmtClock, fmtDateLong, fmtElapsed, fmtTimeAmPm } from "./format";

type Layout = "grid" | "focus" | "overlay";

const SPEEDS = [0.5, 1, 2, 4];

interface Props {
  manifest: SessionManifest;
  onBack: () => void;
  /** deep-link instant (`?session=…&ts=<epoch ms>`); clamped to the session */
  initialEpochMs?: number | null;
  /** offered only for a manifest that was reconstructed from disk */
  onRebuild?: () => void;
}

export function SessionPlayer({ manifest, onBack, initialEpochMs, onRebuild }: Props) {
  const engine = useMemo(() => new SyncEngine(manifest), [manifest]);
  const [snap, setSnap] = useState<EngineSnapshot>(() => engine.snapshot());
  const [layout, setLayout] = useState<Layout>("grid");
  const [showSync, setShowSync] = useState(false);
  const [fit, setFit] = useState<"contain" | "cover">("contain");
  const [stageSize, setStageSize] = useState({ w: 0, h: 0 });
  const stageRef = useRef<HTMLDivElement>(null);
  const clockRef = useRef<HTMLSpanElement>(null);
  const wallRef = useRef<HTMLSpanElement>(null);

  const videoTracks = useMemo(() => manifest.tracks.filter(isVideoTrack), [manifest]);
  const screenTracks = useMemo(() => manifest.tracks.filter((t) => t.kind === "screen"), [manifest]);
  const webcamTrack = useMemo(() => manifest.tracks.find((t) => t.kind === "webcam") ?? null, [manifest]);
  const audioTracks = useMemo(() => manifest.tracks.filter((t) => !isVideoTrack(t)), [manifest]);

  const [focusedId, setFocusedId] = useState<string>(() => (screenTracks[0] ?? videoTracks[0])?.id ?? "");
  const focusedTrack = manifest.tracks.find((t) => t.id === focusedId) ?? videoTracks[0] ?? null;

  // Starts muted, always. A session can be opened by a deep link, and a
  // life recorder's mic track is the last thing that should start playing out
  // loud without the user asking for it. They pick sources from the transport.
  // A set, not a single id: mic + system audio together is the useful default
  // once you do opt in — your voice and what you were listening to.
  const [audioIds, setAudioIds] = useState<Set<string>>(() => new Set());
  const audioTrackIds = useMemo(() => audioTracks.map((t) => t.id), [audioTracks]);
  const allAudible = audioTrackIds.length > 0 && audioTrackIds.every((id) => audioIds.has(id));

  /* ---- engine lifecycle ---- */
  useEffect(() => {
    engine.startLoop();
    const un = engine.subscribeState(setSnap);
    return () => {
      un();
      engine.stopLoop();
    };
  }, [engine]);

  useEffect(() => {
    engine.setAudioTracks(audioIds);
  }, [engine, audioIds]);

  /* Deep link lands on an instant, paused — `seekTo` clamps into the session, so
     a stale link never leaves the playhead outside the footage. */
  useEffect(() => {
    if (initialEpochMs != null && Number.isFinite(initialEpochMs)) {
      engine.seekTo(initialEpochMs);
    }
  }, [engine, initialEpochMs]);

  /* 60fps clock readout, written imperatively so the tree does not re-render */
  useEffect(() => {
    return engine.subscribeFrame((s) => {
      if (clockRef.current) clockRef.current.textContent = fmtElapsed(s.masterEpochMs - engine.start, true);
      if (wallRef.current) wallRef.current.textContent = fmtClock(s.masterEpochMs);
    });
  }, [engine]);

  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const obs = new ResizeObserver(([e]) => setStageSize({ w: e.contentRect.width, h: e.contentRect.height }));
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  /* ---- keyboard ---- */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
      if (e.key === " ") {
        e.preventDefault();
        engine.togglePlaying();
      } else if (e.key === "ArrowLeft") {
        engine.nudgeSeconds(e.shiftKey ? -60 : -5);
      } else if (e.key === "ArrowRight") {
        engine.nudgeSeconds(e.shiftKey ? 60 : 5);
      } else if (e.key === "Escape") {
        onBack();
      } else if (e.key === "g" || e.key === "G" || e.key === "0") {
        setLayout("grid");
      } else if (e.key === "o" || e.key === "O") {
        setLayout("overlay");
      } else if (e.key === "w" || e.key === "W") {
        if (webcamTrack) {
          setFocusedId(webcamTrack.id);
          setLayout("focus");
        }
      } else if (e.key >= "1" && e.key <= "9") {
        const t = screenTracks[parseInt(e.key) - 1];
        if (t) {
          setFocusedId(t.id);
          setLayout("focus");
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [engine, onBack, screenTracks, webcamTrack]);

  const totalMs = engine.end - engine.start;
  const totalBytes = useMemo(
    () => manifest.tracks.reduce((a, t) => a + t.segments.reduce((b, s) => b + s.bytes, 0), 0),
    [manifest],
  );

  const focusTrack = useCallback((id: string) => {
    setFocusedId(id);
    setLayout("focus");
  }, []);

  /* ---- layout geometry ---- */
  const gridCols = Math.min(videoTracks.length, videoTracks.length <= 2 ? videoTracks.length : Math.ceil(Math.sqrt(videoTracks.length)));

  const overlayBoxes = useMemo(() => {
    const d = manifest.desktop;
    if (!stageSize.w || !stageSize.h || !d.w || !d.h) return null;
    const scale = Math.min(stageSize.w / d.w, stageSize.h / d.h);
    const offX = (stageSize.w - d.w * scale) / 2;
    const offY = (stageSize.h - d.h * scale) / 2;
    return screenTracks
      .filter((t) => t.monitor)
      .map((t) => ({
        track: t,
        style: {
          position: "absolute" as const,
          left: offX + (t.monitor!.x - d.x) * scale,
          top: offY + (t.monitor!.y - d.y) * scale,
          width: t.monitor!.w * scale,
          height: t.monitor!.h * scale,
        },
      }));
  }, [manifest.desktop, screenTracks, stageSize]);

  return (
    <div className="flex-1 min-h-0 flex flex-col bg-[#0A0A0F]">
      {/* ---- toolbar ---- */}
      <div className="flex items-center px-5 py-3 gap-4 bg-white/[0.03] border-b border-white/[0.06] shrink-0">
        <button
          className="flex items-center gap-1 px-3 py-2 rounded-[10px] border border-white/10 bg-white/[0.06] text-white/70 text-[13px] font-medium cursor-pointer hover:bg-white/10 hover:text-white hover:border-white/20 transition-all"
          onClick={onBack}
        >
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M15 18l-6-6 6-6" />
          </svg>
          <span>Back</span>
        </button>

        <div className="flex-1 min-w-0 flex items-baseline gap-3">
          <span className="text-base font-bold text-white truncate">{fmtDateLong(manifest.started_epoch_ms)}</span>
          <span className="text-[13px] text-white/40 tabular-nums font-mono">
            {fmtTimeAmPm(manifest.started_epoch_ms)}
          </span>
          <span className="text-[13px] text-white/25">
            {videoTracks.length} video · {audioTracks.length} audio · {fmtBytes(totalBytes)}
          </span>
        </div>

        {/* layout */}
        <div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
          {(["grid", "focus", "overlay"] as Layout[]).map((l) => (
            <button
              key={l}
              className={`px-3 py-1 rounded-md text-xs font-semibold capitalize cursor-pointer transition-all ${
                layout === l ? "text-sky-500 bg-sky-500/15" : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
              }`}
              onClick={() => setLayout(l)}
            >
              {l}
            </button>
          ))}
        </div>

        {/* monitor picker */}
        <div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
          {screenTracks.map((t, i) => (
            <button
              key={t.id}
              title={t.label}
              className={`px-3 py-1 rounded-md text-xs font-semibold cursor-pointer transition-all ${
                layout === "focus" && focusedId === t.id
                  ? "text-sky-500 bg-sky-500/15"
                  : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
              }`}
              onClick={() => focusTrack(t.id)}
            >
              {i + 1}
            </button>
          ))}
          {webcamTrack && (
            <button
              title={webcamTrack.label}
              className={`px-3 py-1 rounded-md text-xs font-semibold cursor-pointer transition-all ${
                layout === "focus" && focusedId === webcamTrack.id
                  ? "text-purple-400 bg-purple-500/15"
                  : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
              }`}
              onClick={() => focusTrack(webcamTrack.id)}
            >
              Cam
            </button>
          )}
        </div>

        {/* fit */}
        <div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
          {(["contain", "cover"] as const).map((f) => (
            <button
              key={f}
              className={`px-2.5 py-1 rounded-md text-[11px] font-semibold cursor-pointer transition-all ${
                fit === f ? "text-white bg-white/10" : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
              }`}
              onClick={() => setFit(f)}
            >
              {f === "contain" ? "Fit" : "Fill"}
            </button>
          ))}
        </div>

        {/* This session had no usable session.json, so what is on screen was
            reconstructed by scanning the directory. Writing it back is the only
            action that makes that permanent. */}
        {manifest.rebuilt && onRebuild && (
          <button
            className="px-3 py-1.5 rounded-lg text-xs font-semibold border cursor-pointer transition-all text-amber-400 bg-amber-400/10 border-amber-400/30 hover:bg-amber-400/20"
            onClick={onRebuild}
            title="This manifest was rebuilt from the files on disk. Save it back to the session folder."
          >
            Save Index
          </button>
        )}

        <button
          className={`px-3 py-1.5 rounded-lg text-xs font-semibold border cursor-pointer transition-all ${
            showSync
              ? "text-sky-500 bg-sky-500/15 border-sky-500/30"
              : "text-white/40 bg-white/[0.04] border-white/[0.06] hover:text-white/70"
          }`}
          onClick={() => setShowSync((v) => !v)}
        >
          Sync
        </button>
      </div>

      {/* ---- stage ---- */}
      <div ref={stageRef} className="flex-1 min-h-0 relative p-2">
        {layout === "grid" && (
          <div
            className="w-full h-full grid gap-2"
            style={{
              gridTemplateColumns: `repeat(${Math.max(1, gridCols)}, minmax(0, 1fr))`,
              gridAutoRows: "minmax(0, 1fr)",
            }}
          >
            {videoTracks.map((t) => (
              <TrackSurface
                key={t.id}
                engine={engine}
                track={t}
                debug={snap.tracks[t.id]}
                showSync={showSync}
                fit={fit}
                selected={focusedId === t.id}
                onSelect={() => focusTrack(t.id)}
              />
            ))}
          </div>
        )}

        {layout === "focus" && focusedTrack && (
          <div className="w-full h-full relative">
            <TrackSurface
              engine={engine}
              track={focusedTrack}
              debug={snap.tracks[focusedTrack.id]}
              showSync={showSync}
              fit={fit}
              className="w-full h-full"
            />
            {webcamTrack && webcamTrack.id !== focusedTrack.id && (
              <div
                className="absolute bottom-4 right-4 z-[5] rounded-lg overflow-hidden shadow-[0_8px_32px_rgba(0,0,0,0.6)] ring-1 ring-purple-500/40"
                style={{ width: "22%", aspectRatio: `${webcamTrack.width} / ${webcamTrack.height}` }}
              >
                <TrackSurface
                  engine={engine}
                  track={webcamTrack}
                  debug={snap.tracks[webcamTrack.id]}
                  showSync={showSync}
                  fit="cover"
                  bare
                  className="w-full h-full"
                  onSelect={() => focusTrack(webcamTrack.id)}
                />
              </div>
            )}
          </div>
        )}

        {layout === "overlay" && (
          <div className="w-full h-full relative">
            {overlayBoxes?.map(({ track, style }) => (
              <TrackSurface
                key={track.id}
                engine={engine}
                track={track}
                debug={snap.tracks[track.id]}
                showSync={showSync}
                fit="contain"
                selected={focusedId === track.id}
                onSelect={() => focusTrack(track.id)}
                style={style}
              />
            ))}
            {webcamTrack && (
              <div
                className="absolute bottom-4 right-4 z-[5] rounded-lg overflow-hidden shadow-[0_8px_32px_rgba(0,0,0,0.6)] ring-1 ring-purple-500/40"
                style={{ width: "18%", aspectRatio: `${webcamTrack.width} / ${webcamTrack.height}` }}
              >
                <TrackSurface
                  engine={engine}
                  track={webcamTrack}
                  debug={snap.tracks[webcamTrack.id]}
                  showSync={showSync}
                  fit="cover"
                  bare
                  className="w-full h-full"
                />
              </div>
            )}
          </div>
        )}

        {/* audio-only tracks stay mounted and synced in every layout */}
        <div className="absolute w-0 h-0 overflow-hidden opacity-0 pointer-events-none">
          {audioTracks.map((t) => (
            <TrackSurface
              key={t.id}
              engine={engine}
              track={t}
              debug={snap.tracks[t.id]}
              showSync={false}
              fit="contain"
              bare
            />
          ))}
        </div>

        {showSync && <SyncPanel snap={snap} tracks={manifest.tracks} onReset={() => engine.resetMetrics()} />}
      </div>

      {/* ---- transport ---- */}
      <div className="flex items-center gap-3 px-5 py-2.5 bg-white/[0.03] border-t border-white/[0.06] shrink-0">
        <button
          className="w-10 h-10 flex items-center justify-center rounded-full bg-sky-500 text-white cursor-pointer hover:bg-sky-400 transition-all shrink-0"
          onClick={() => engine.togglePlaying()}
          title="Play / pause (Space)"
        >
          {snap.playing ? (
            <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
              <rect x="6" y="4" width="4" height="16" rx="1" />
              <rect x="14" y="4" width="4" height="16" rx="1" />
            </svg>
          ) : (
            <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
              <polygon points="7 4 20 12 7 20 7 4" />
            </svg>
          )}
        </button>

        <button
          className="w-9 h-9 flex items-center justify-center rounded-[10px] border border-white/10 bg-white/[0.06] text-white/60 cursor-pointer hover:bg-white/10 hover:text-white transition-all"
          onClick={() => engine.nudgeSeconds(-10)}
          title="Back 10s"
        >
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M11 17l-5-5 5-5M18 17l-5-5 5-5" />
          </svg>
        </button>
        <button
          className="w-9 h-9 flex items-center justify-center rounded-[10px] border border-white/10 bg-white/[0.06] text-white/60 cursor-pointer hover:bg-white/10 hover:text-white transition-all"
          onClick={() => engine.nudgeSeconds(10)}
          title="Forward 10s"
        >
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M13 17l5-5-5-5M6 17l5-5-5-5" />
          </svg>
        </button>

        <div className="flex items-baseline gap-2 pl-1">
          <span ref={clockRef} className="text-base font-bold text-white tabular-nums font-mono">
            {fmtElapsed(0, true)}
          </span>
          <span className="text-[13px] text-white/30 tabular-nums font-mono">/ {fmtElapsed(totalMs)}</span>
          <span ref={wallRef} className="text-[13px] text-white/40 tabular-nums font-mono pl-2">
            {fmtClock(manifest.started_epoch_ms)}
          </span>
        </div>

        <div className="flex-1" />

        {/* inter-track spread — the number that says "the timecodes agree" */}
        <div className="flex items-center gap-2 px-2.5 py-1 rounded-lg bg-white/[0.04] border border-white/[0.06]">
          <span className="text-[10px] font-semibold uppercase tracking-wide text-white/25">Spread</span>
          <span
            className={`text-[11px] font-semibold tabular-nums font-mono ${
              snap.spreadMs > HARD_SEEK_MS ? "text-red-400" : snap.spreadMs > SOFT_NUDGE_MS ? "text-amber-400" : "text-emerald-400"
            }`}
          >
            {snap.spreadMs.toFixed(0)}ms
          </span>
          <span className="text-[11px] text-white/25 tabular-nums font-mono">max {snap.maxSpreadMs.toFixed(0)}</span>
        </div>

        {/* audio sources — multi-select, so mic + system audio can play together */}
        <div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
          <button
            className={`px-2.5 py-1 rounded-md text-[11px] font-semibold cursor-pointer transition-all ${
              audioIds.size === 0 ? "text-white bg-white/10" : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
            }`}
            onClick={() => setAudioIds(new Set())}
            title="Mute everything"
          >
            Muted
          </button>
          <button
            className={`px-2.5 py-1 rounded-md text-[11px] font-semibold cursor-pointer transition-all ${
              allAudible ? "text-white bg-white/10" : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
            }`}
            onClick={() => setAudioIds(allAudible ? new Set() : new Set(audioTrackIds))}
            title="Play every audio track at once, mixed together"
          >
            All
          </button>
          {manifest.tracks
            .filter((t) => t.kind !== "screen")
            .map((t) => (
              <button
                key={t.id}
                className={`px-2.5 py-1 rounded-md text-[11px] font-semibold cursor-pointer transition-all ${
                  audioIds.has(t.id)
                    ? "text-white bg-white/10"
                    : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
                }`}
                onClick={() =>
                  setAudioIds((prev) => {
                    const next = new Set(prev);
                    if (next.has(t.id)) next.delete(t.id);
                    else next.add(t.id);
                    return next;
                  })
                }
                title={`${t.label} — click to toggle; combines with the others`}
              >
                {t.id}
              </button>
            ))}
        </div>

        {/* speed */}
        <div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
          {SPEEDS.map((s) => (
            <button
              key={s}
              className={`px-2.5 py-1 rounded-md text-[11px] font-semibold font-mono cursor-pointer transition-all ${
                snap.rate === s ? "text-white bg-white/10" : "text-white/40 hover:text-white/70 hover:bg-white/[0.06]"
              }`}
              onClick={() => engine.setRate(s)}
            >
              {s}x
            </button>
          ))}
        </div>
      </div>

      <Timeline
        manifest={manifest}
        engine={engine}
        start={engine.start}
        end={engine.end}
        focusedTrackId={layout === "focus" ? focusedId : null}
        onFocusTrack={focusTrack}
      />
    </div>
  );
}

function SyncPanel({
  snap,
  tracks,
  onReset,
}: {
  snap: EngineSnapshot;
  tracks: SessionTrack[];
  onReset: () => void;
}) {
  return (
    <div className="absolute top-4 right-4 z-[20] w-[340px] rounded-lg bg-black/85 backdrop-blur-md border border-white/10 p-3">
      <div className="flex items-center justify-between mb-2">
        <span className="text-[10px] font-semibold uppercase tracking-wide text-white/40">Sync</span>
        <button
          className="text-[10px] font-semibold text-white/40 hover:text-white cursor-pointer transition-all"
          onClick={onReset}
        >
          Reset
        </button>
      </div>
      <table className="w-full text-[10px] tabular-nums font-mono">
        <thead>
          <tr className="text-white/25 text-left">
            <th className="font-medium pb-1">track</th>
            <th className="font-medium pb-1">seg</th>
            <th className="font-medium pb-1 text-right">drift</th>
            <th className="font-medium pb-1 text-right">max</th>
            <th className="font-medium pb-1 text-right">seek</th>
            <th className="font-medium pb-1 text-right">swap</th>
            <th className="font-medium pb-1 text-right">cold</th>
          </tr>
        </thead>
        <tbody>
          {tracks.map((t) => {
            const d = snap.tracks[t.id];
            if (!d) return null;
            const a = Math.abs(d.driftMs);
            const tone = a > HARD_SEEK_MS ? "text-red-400" : a > SOFT_NUDGE_MS ? "text-amber-400" : "text-emerald-400";
            return (
              <tr key={t.id} className="text-white/60">
                <td className="py-px truncate max-w-[70px]">{t.id}</td>
                <td className={d.state === "playing" ? "" : "text-white/25"}>
                  {d.state === "playing" ? d.segIndex + 1 : d.state}
                </td>
                <td className={`text-right ${tone}`}>{d.driftMs.toFixed(0)}</td>
                <td className="text-right text-white/40">{d.maxAbsDriftMs.toFixed(0)}</td>
                <td className="text-right text-white/40">{d.hardSeeks}</td>
                <td className="text-right text-white/40">{d.segmentSwaps}</td>
                <td className={`text-right ${d.coldSwitches ? "text-amber-400" : "text-white/40"}`}>{d.coldSwitches}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className="mt-2 pt-2 border-t border-white/10 flex items-center justify-between text-[10px] tabular-nums font-mono">
        <span className="text-white/25">inter-track spread</span>
        <span className="text-white/70">
          {snap.spreadMs.toFixed(0)}ms <span className="text-white/30">max {snap.maxSpreadMs.toFixed(0)}ms</span>
        </span>
      </div>
    </div>
  );
}
