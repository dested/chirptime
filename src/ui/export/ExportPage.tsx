/**
 * `#/export/<id>?from=&to=`: the composer. A live preview of the output frame
 * on the dark mat, the range on a strip below it, and the card of choices on
 * the right; the footer estimates, exports (or copies raw tracks) and shows
 * the job through to "Saved".
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { SessionManifest, SessionTrack } from "../types";
import { fetchSession } from "../hooks/useApi";
import { SyncEngine, sessionBounds } from "../session/engine";
import type { EngineSnapshot, TrackState } from "../session/engine";
import { sessionHoles } from "../session/cuts";
import { fmtClock12, fmtDayTitle, fmtSize, fmtTimeAmPm } from "../session/format";
import { hasFootage, makeAxis, screenNumber, screenTracks, snapTargets } from "../player/timeline";
import { useCursorFollow } from "../player/useFollow";
import { Popover } from "../player/Popover";
import { errorText } from "../app/appData";
import { href } from "../app/router";
import { IconBack, IconCheck, IconDown } from "../app/icons";
import { cancelExport, estimateExport, loadPreset, revealExport, savePreset, startExport } from "./api";
import { addJob, useJobs } from "./jobs";
import type { Job } from "./jobs";
import { compose, largestScreen, layoutAspect } from "./compose";
import { MAT_PAD, Preview } from "./Preview";
import { RangeStrip } from "./RangeStrip";
import { Seg, Slider, Toggle, ToggleRow } from "./Controls";
import type { CameraOpts, Corner, ExportEstimate, ExportPreset, ExportRequest, Layout, SizeMode, SoundOpts, Speed } from "./types";

const PANEL_W = 356;
const GAP = 24;
const TOP = 72;
/** room under the mat for the caption, the range strip and its chips */
const BELOW_MAT = 228;
const DB_MIN = -30;
const DB_MAX = 10;
const ESTIMATE_DEBOUNCE_MS = 300;

export function ExportPage({ id, from, to }: { id: string; from: number | null; to: number | null }) {
  const [manifest, setManifest] = useState<SessionManifest | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    fetchSession(id)
      .then((m) => alive && setManifest(m))
      .catch((e: unknown) => alive && setLoadError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [id]);

  if (loadError) {
    return (
      <div className="px-8 pt-6">
        <h1 className="text-[20px] font-bold tracking-[-0.02em]">This session could not be opened</h1>
        <p className="mt-2 max-w-[560px] text-ink-2">{loadError}</p>
        <a className="mt-4 inline-block font-semibold text-ink underline" href="#/library">
          Back to the Library
        </a>
      </div>
    );
  }
  if (!manifest) return <div className="px-6 pt-6 text-ink-3">Opening the session</div>;
  return <Composer manifest={manifest} initialFrom={from} initialTo={to} />;
}

/* ---- small hooks ------------------------------------------------------------ */

function useSize<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((e) => {
      const r = e[0]?.contentRect;
      if (r) setSize({ w: r.width, h: r.height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, size] as const;
}

/** Track states and the playhead at the engine's 5 Hz state rate. */
function useEngineState(engine: SyncEngine) {
  const pick = (s: EngineSnapshot) => {
    const states: Record<string, TrackState> = {};
    for (const [k, d] of Object.entries(s.tracks)) states[k] = d.state;
    return { states, at: s.masterEpochMs, playing: s.playing };
  };
  const [v, setV] = useState(() => pick(engine.snapshot()));
  useEffect(() => {
    setV(pick(engine.snapshot()));
    return engine.subscribeState((s) => setV(pick(s)));
  }, [engine]);
  return v;
}

/** `24 minutes`, `1 hour 5 minutes`, `40 seconds` for the Export button. */
function lengthWords(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} ${s === 1 ? "second" : "seconds"}`;
  const totalMin = Math.round(s / 60);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  const mins = `${m} ${m === 1 ? "minute" : "minutes"}`;
  if (!h) return mins;
  const hours = `${h} ${h === 1 ? "hour" : "hours"}`;
  return m ? `${hours} ${mins}` : hours;
}

/** `4 min`, `40 s` */
function shortLength(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${Math.max(1, s)} s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} min`;
}

function dbLabel(v: number | null): string {
  if (v === null) return "Off";
  if (v === 0) return "0 dB";
  return `${v > 0 ? "+" : "−"}${Math.abs(v)} dB`;
}

/** Does the track have any footage between `from` and `to`? */
function hasFootageIn(t: SessionTrack, from: number, to: number): boolean {
  return t.segments.some((s) => s.start_epoch_ms < to && s.start_epoch_ms + s.duration_ms > from);
}

/* ---- the composer ----------------------------------------------------------- */

function Composer({ manifest, initialFrom, initialTo }: { manifest: SessionManifest; initialFrom: number | null; initialTo: number | null }) {
  const id = manifest.id;
  const bounds = useMemo(() => sessionBounds(manifest), [manifest]);
  const holes = useMemo(() => sessionHoles(manifest), [manifest]);
  const targets = useMemo(() => snapTargets(manifest), [manifest]);
  const screens = useMemo(() => [...screenTracks(manifest)].sort((a, b) => screenNumber(a) - screenNumber(b)), [manifest]);
  const webcam = manifest.tracks.find((t) => t.kind === "webcam") ?? null;
  const mic = manifest.tracks.find((t) => t.kind === "mic") ?? null;
  const sys = manifest.tracks.find((t) => t.kind === "system-audio") ?? null;
  const biggest = useMemo(() => largestScreen(screens), [screens]);

  /* ---- the range, and the strip's window around it ---- */
  const [range, setRange] = useState(() => {
    const f = Math.max(bounds.start, Math.min(initialFrom ?? bounds.start, bounds.end - 1_000));
    const t = Math.min(bounds.end, Math.max(initialTo ?? bounds.end, f + 1_000));
    return { from: f, to: t };
  });
  const windowFor = useCallback(
    (f: number, t: number) => {
      const pad = Math.max((t - f) * 0.25, 60_000);
      return makeAxis(holes, Math.max(bounds.start, f - pad), Math.min(bounds.end, t + pad));
    },
    [holes, bounds],
  );
  const [axis, setAxis] = useState(() => windowFor(range.from, range.to));

  /* ---- choices, from the saved preset ---- */
  const [preset] = useState<ExportPreset>(() => loadPreset());
  const [layout, setLayout] = useState<Layout>(preset.layout);
  const [screenId, setScreenId] = useState<string | null>(preset.follow ? null : (biggest?.id ?? null));
  const [camera, setCamera] = useState<CameraOpts>(preset.camera);
  const [sound, setSound] = useState<SoundOpts>(() => ({
    micDb: mic ? preset.sound.micDb : null,
    systemDb: sys ? preset.sound.systemDb : null,
    duck: preset.sound.duck,
  }));
  const [speed, setSpeed] = useState<Speed>(preset.speed);
  const [skipIdle, setSkipIdle] = useState(preset.skipIdle);
  const [size, setSize] = useState<SizeMode>(preset.size);
  const [savedFlash, setSavedFlash] = useState(false);

  /* ---- the engine: silent, looping inside the range ---- */
  const engine = useMemo(() => new SyncEngine(manifest), [manifest]);
  const rangeRef = useRef(range);
  rangeRef.current = range;
  useEffect(() => {
    engine.setAudioTracks([]);
    engine.seekTo(rangeRef.current.from);
    engine.startLoop();
    const off = engine.subscribeFrame((s) => {
      const r = rangeRef.current;
      if (s.playing && s.masterEpochMs >= r.to) engine.seekTo(r.from);
    });
    return () => {
      off();
      engine.setPlaying(false);
      engine.stopLoop();
    };
  }, [engine]);
  const { states, at } = useEngineState(engine);
  const togglePlay = useCallback(() => {
    const r = rangeRef.current;
    if (engine.masterEpochMs < r.from || engine.masterEpochMs >= r.to) engine.seekTo(r.from);
    engine.togglePlaying();
  }, [engine]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== "Space" || e.target instanceof HTMLInputElement || e.target instanceof HTMLButtonElement) return;
      e.preventDefault();
      togglePlay();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [togglePlay]);

  /* ---- what the frame shows now ---- */
  const cursor = useCursorFollow(engine, id, screens, biggest?.id ?? null);
  const following = screenId === null;
  const shown = following ? (cursor.hasData ? screens.find((t) => t.id === cursor.followedId) ?? biggest : biggest) : screens.find((t) => t.id === screenId) ?? biggest;
  const deskScreens = useMemo(() => screens.filter((t) => hasFootageIn(t, range.from, range.to)), [screens, range]);
  const camInRange = webcam && camera.enabled && hasFootageIn(webcam, range.from, range.to) ? webcam : null;
  const screenNow = layout === "desk" ? deskScreens.some((t) => hasFootage(manifest, t, at)) : shown !== null && hasFootage(manifest, shown, at);
  const hidden = useMemo(() => {
    const s = new Set<string>();
    for (const t of [...screens, ...(webcam ? [webcam] : [])]) if (!hasFootage(manifest, t, at)) s.add(t.id);
    return s;
  }, [manifest, screens, webcam, at]);

  /* ---- sizes ---- */
  const [colRef, col] = useSize<HTMLDivElement>();
  const [pageRef, page] = useSize<HTMLDivElement>();
  const matW = col.w;
  const aspect = layoutAspect(layout);
  const wideFrameW = Math.max(0, matW - MAT_PAD * 2);
  const maxFrameH = Math.max(120, page.h - TOP - BELOW_MAT - MAT_PAD * 2);
  const frame16H = Math.min((wideFrameW * 9) / 16, maxFrameH);
  const matH = frame16H + MAT_PAD * 2;
  const fh = frame16H;
  const fw = layout === "short" ? fh * aspect : Math.min(wideFrameW, fh * aspect);
  const pieces = useMemo(
    () => compose({ layout, fw, fh, screen: shown, deskScreens, screenNow, camera: camInRange, cam: camera }),
    [layout, fw, fh, shown, deskScreens, screenNow, camInRange, camera],
  );

  /* ---- the request and its estimate ---- */
  const req: ExportRequest = useMemo(
    () => ({ sessionId: id, fromMs: Math.round(range.from), toMs: Math.round(range.to), layout, screen: screenId, camera, sound, speed, skipIdle, size, rawTracks: null, title: null }),
    [id, range, layout, screenId, camera, sound, speed, skipIdle, size],
  );
  const [estimate, setEstimate] = useState<ExportEstimate | null>(null);
  const [estimateError, setEstimateError] = useState<string | null>(null);
  const reqKey = JSON.stringify(req);
  const reqRef = useRef(req);
  reqRef.current = req;
  useEffect(() => {
    let alive = true;
    const t = window.setTimeout(() => {
      estimateExport(reqRef.current)
        .then((e) => {
          if (!alive) return;
          setEstimate(e);
          setEstimateError(null);
        })
        .catch((e: unknown) => alive && setEstimateError(errorText(e)));
    }, ESTIMATE_DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
    // the key stands for the request: a new object with the same choices is no change
  }, [reqKey]);

  /* ---- the job ---- */
  const jobs = useJobs();
  const [jobId, setJobId] = useState<string | null>(() => jobs.find((j) => j.sessionId === id && j.progress.state === "running")?.id ?? null);
  const job = jobs.find((j) => j.id === jobId) ?? null;
  const [startError, setStartError] = useState<string | null>(null);
  const begin = useCallback(
    async (r: ExportRequest, title: string) => {
      setStartError(null);
      try {
        const jid = await startExport(r);
        addJob({ id: jid, sessionId: r.sessionId, from: r.fromMs, to: r.toMs, title });
        setJobId(jid);
      } catch (e) {
        setStartError(errorText(e));
      }
    },
    [],
  );

  /* ---- raw tracks ---- */
  const [rawOpen, setRawOpen] = useState(false);
  const rawChoices = useMemo(() => manifest.tracks.filter((t) => t.segments.length > 0), [manifest]);
  const [rawPicked, setRawPicked] = useState<ReadonlySet<string>>(() => new Set(rawChoices.map((t) => t.id)));
  const [rawEstimate, setRawEstimate] = useState<ExportEstimate | null>(null);
  const rawReq = useMemo<ExportRequest>(() => ({ ...req, rawTracks: rawChoices.filter((t) => rawPicked.has(t.id)).map((t) => t.id) }), [req, rawChoices, rawPicked]);
  useEffect(() => {
    if (!rawOpen) return;
    let alive = true;
    estimateExport(rawReq)
      .then((e) => alive && setRawEstimate(e))
      .catch(() => alive && setRawEstimate(null));
    return () => {
      alive = false;
    };
  }, [rawOpen, rawReq]);

  const sourceMs = estimate && !estimate.error ? estimate.sourceMs : range.to - range.from;
  const cornerCam = layout === "devlog" || layout === "desk";
  const camRow = !webcam ? "No camera in this session" : !cornerCam ? (layout === "short" ? "Short puts the camera under the screen" : "Side by side gives the camera the right half") : null;
  const hint =
    pieces.some((p) => p.corner) && !job ? "Drag the camera to move it · click to play" : "Click to play";
  const followHint =
    layout === "desk"
      ? "Every screen where it sits on your desk"
      : following
        ? cursor.hasData
          ? "Follows your cursor between screens"
          : biggest
            ? `No mouse data here, so it shows Screen ${screenNumber(biggest)}, the largest`
            : "No screens in this session"
        : shown
          ? `Screen ${screenNumber(shown)}`
          : "";

  return (
    <div ref={pageRef} className="relative h-full overflow-hidden px-6">
      {/* header */}
      <div className="flex h-16 items-center gap-3">
        <a
          href={href({ name: "session", id, t: range.from })}
          className="-ml-1.5 flex h-8 items-center gap-[7px] rounded-ctl pl-1 pr-2 text-[12.5px] font-semibold text-ink-2 transition-colors duration-150 hover:bg-card/70 hover:text-ink"
        >
          <IconBack size={15} />
          {fmtDayTitle(manifest.started_epoch_ms)}
        </a>
        <div className="h-5 w-px bg-line" />
        <h1 className="text-[20px] font-bold tracking-[-0.02em]">Export</h1>
        <div className="num whitespace-nowrap text-[11.5px] text-ink-3">
          {fmtTimeAmPm(range.from)} – {fmtTimeAmPm(range.to)} · {shortLength(range.to - range.from)}
        </div>
      </div>

      <div className="flex" style={{ gap: GAP, height: `calc(100% - 64px - 24px)`, marginTop: TOP - 64 }}>
        {/* left: preview, caption, range */}
        <div ref={colRef} className="min-w-0 flex-1">
          {matW > 0 && (
            <>
              <Preview
                engine={engine}
                matW={matW}
                matH={matH}
                fw={fw}
                fh={fh}
                pieces={pieces}
                states={states}
                hidden={hidden}
                onCorner={(c: Corner) => setCamera((v) => ({ ...v, corner: c }))}
                onTogglePlay={togglePlay}
              />
              <div className="mt-2.5 flex items-center gap-2.5 text-[12px] text-ink-3">
                <span className="num whitespace-nowrap text-ink-2">
                  {estimate && !estimate.error ? `${estimate.width} × ${estimate.height}` : layout === "short" ? "1080 × 1920" : "1920 × 1080"} · {fmtClock12(at)}
                </span>
                <span className="ml-auto truncate">
                  {followHint}
                  {followHint ? " · " : ""}
                  {hint}
                </span>
              </div>
              <div className="mt-[26px] text-[13px] font-bold">Range</div>
              <div className="mt-[11px]">
                <RangeStrip
                  manifest={manifest}
                  axis={axis}
                  width={matW}
                  from={range.from}
                  to={range.to}
                  playhead={at}
                  hero={shown ?? biggest}
                  targets={targets}
                  onRange={(f, t, done) => {
                    setRange({ from: f, to: t });
                    if (engine.masterEpochMs < f || engine.masterEpochMs > t) engine.seekTo(f);
                    if (done) setAxis(windowFor(f, t));
                  }}
                  onSeek={(t) => engine.seekTo(t)}
                />
              </div>
            </>
          )}
        </div>

        {/* right: the card */}
        <aside className="flex shrink-0 flex-col overflow-hidden rounded-card border border-line bg-card" style={{ width: PANEL_W }}>
          <div className="min-h-0 flex-1 overflow-y-auto">
            <Section title="Layout" right={layout === "short" ? "9:16" : "16:9"}>
              <div className="mt-2.5 grid grid-cols-4 gap-1.5">
                {LAYOUTS.map((l) => (
                  <button
                    key={l.value}
                    onClick={() => setLayout(l.value)}
                    aria-pressed={layout === l.value}
                    className={`group flex h-[50px] flex-col items-center justify-center gap-1.5 rounded-[9px] border text-[11.5px] font-semibold transition-colors duration-150 ${
                      layout === l.value ? "on border-ink bg-card text-ink shadow-[inset_0_0_0_1px_var(--color-ink)]" : "border-line bg-paper text-ink-2 hover:bg-card"
                    }`}
                  >
                    <Glyph w={l.glyph[0]} h={l.glyph[1]} rects={l.glyph[2]} on={layout === l.value} />
                    {l.label}
                  </button>
                ))}
              </div>
            </Section>

            <Section title="Screen">
              <Seg
                className="mt-2.5"
                disabled={layout === "desk" || screens.length === 0}
                value={layout === "desk" ? null : (screenId ?? "follow")}
                onChange={(v) => setScreenId(v === "follow" ? null : v)}
                options={[{ value: "follow", label: "Follow mouse" }, ...screens.map((t) => ({ value: t.id, label: String(screenNumber(t)), title: `Screen ${screenNumber(t)}` }))]}
              />
            </Section>

            <Section
              title="Camera"
              right={<Toggle on={camera.enabled && webcam !== null} disabled={!webcam} label="Camera" onChange={(v) => setCamera((c) => ({ ...c, enabled: v }))} />}
            >
              <div className={`mt-[9px] flex items-center gap-2.5 ${camera.enabled && !camRow ? "" : "opacity-50"}`} title={camRow ?? undefined}>
                <span className="w-[58px] shrink-0 text-[12px] text-ink-2">Corner</span>
                <div className="grid grid-cols-[20px_20px] gap-[3px]">
                  {CORNERS.map((c) => (
                    <button
                      key={c}
                      aria-label={c.replace("-", " ")}
                      aria-pressed={camera.corner === c}
                      disabled={!camera.enabled || camRow !== null}
                      onClick={() => setCamera((v) => ({ ...v, corner: c }))}
                      className={`h-3 rounded-[3px] border ${camera.corner === c ? "border-canary-deep bg-canary-deep" : "border-line bg-sunk hover:bg-line"}`}
                    />
                  ))}
                </div>
                <Seg
                  className="ml-auto w-[150px]"
                  disabled={!camera.enabled || camRow !== null}
                  value={camera.shape}
                  onChange={(v) => setCamera((c) => ({ ...c, shape: v }))}
                  options={[
                    { value: "rounded", label: "Rounded" },
                    { value: "circle", label: "Circle" },
                  ]}
                />
              </div>
              <div className={`mt-[9px] flex items-center gap-2.5 ${camera.enabled && !camRow ? "" : "opacity-50"}`}>
                <span className="w-[58px] shrink-0 text-[12px] text-ink-2">Size</span>
                <Slider
                  label="Camera size"
                  min={10}
                  max={50}
                  step={1}
                  disabled={!camera.enabled || camRow !== null}
                  value={Math.round(camera.size * 100)}
                  onChange={(v) => setCamera((c) => ({ ...c, size: v / 100 }))}
                />
                <span className="num w-[40px] text-right text-[11px] text-ink-2">{Math.round(camera.size * 100)}%</span>
              </div>
              {camRow && <div className="mt-1.5 text-[11px] text-ink-3">{camRow}</div>}
            </Section>

            <Section title="Sound">
              <GainRow label="Mic" track={mic} value={sound.micDb} onChange={(v) => setSound((s) => ({ ...s, micDb: v }))} disabled={speed === "timelapse"} />
              <GainRow label="System" track={sys} value={sound.systemDb} onChange={(v) => setSound((s) => ({ ...s, systemDb: v }))} disabled={speed === "timelapse"} />
              <ToggleRow
                label="Lower system audio while I talk"
                on={sound.duck}
                disabled={sound.micDb === null || sound.systemDb === null || speed === "timelapse"}
                onChange={(v) => setSound((s) => ({ ...s, duck: v }))}
              />
            </Section>

            <Section title="Speed">
              <Seg
                className="mt-2.5"
                value={speed}
                onChange={setSpeed}
                options={[
                  { value: "1x", label: "1×" },
                  { value: "2x", label: "2×" },
                  { value: "4x", label: "4×" },
                  { value: "timelapse", label: "Timelapse", title: "One frame a second, played at 30" },
                ]}
              />
              <ToggleRow label="Skip idle stretches" on={skipIdle} onChange={setSkipIdle} />
            </Section>

            <Section title="Size" last>
              <Seg
                className="mt-2.5"
                value={size}
                onChange={setSize}
                options={[
                  { value: "original", label: "Original" },
                  { value: "1080p", label: "1080p" },
                  { value: "share", label: "Share · 25 MB", title: "Small enough to attach: under 25 MB" },
                ]}
              />
            </Section>
          </div>

          <Footer
            job={job}
            estimate={estimate}
            estimateError={estimateError}
            startError={startError}
            sourceMs={sourceMs}
            onExport={() => void begin(req, LAYOUTS.find((l) => l.value === layout)?.label ?? "Export")}
            onSavePreset={() => {
              const ok = savePreset({ layout, follow: screenId === null, camera, sound, speed, skipIdle, size });
              if (ok) {
                setSavedFlash(true);
                window.setTimeout(() => setSavedFlash(false), 1600);
              }
            }}
            savedFlash={savedFlash}
            onDismissJob={() => setJobId(null)}
            raw={
              <div className="relative">
                <button
                  className={`flex items-center gap-1 rounded-[6px] px-1 text-[12px] font-semibold text-ink-2 transition-colors duration-150 hover:text-ink ${rawOpen ? "text-ink" : ""}`}
                  onClick={() => setRawOpen((v) => !v)}
                  aria-expanded={rawOpen}
                >
                  Raw tracks
                  <IconDown size={12} />
                </button>
                <Popover open={rawOpen} onClose={() => setRawOpen(false)} align="right" side="above" className="w-[300px] p-3 text-ink">
                  <div className="text-[13px] font-bold">Raw tracks</div>
                  <p className="mt-1 text-[12px] leading-[1.4] text-ink-2">Each track copied as it was recorded, no re-encode, into one folder.</p>
                  <ul className="mt-2">
                    {rawChoices.map((t) => (
                      <li key={t.id}>
                        <label className="flex h-7 cursor-pointer items-center gap-2 text-[12.5px]">
                          <input
                            type="checkbox"
                            className="accent-[var(--color-ink)]"
                            checked={rawPicked.has(t.id)}
                            onChange={(e) =>
                              setRawPicked((prev) => {
                                const next = new Set(prev);
                                if (e.target.checked) next.add(t.id);
                                else next.delete(t.id);
                                return next;
                              })
                            }
                          />
                          {trackName(t)}
                        </label>
                      </li>
                    ))}
                  </ul>
                  <div className="num mt-1.5 text-[11.5px] text-ink-3">
                    {rawEstimate?.error ? <span className="text-rec">{rawEstimate.error}</span> : rawEstimate ? `About ${fmtSize(rawEstimate.bytes)}` : "Working out the size"}
                  </div>
                  <button
                    className="mt-2.5 flex h-8 w-full items-center justify-center rounded-ctl border border-line bg-card text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper disabled:opacity-50"
                    disabled={rawPicked.size === 0 || (job?.progress.state === "running") || Boolean(rawEstimate?.error)}
                    onClick={() => {
                      setRawOpen(false);
                      void begin(rawReq, "Raw tracks");
                    }}
                  >
                    Save {rawPicked.size} {rawPicked.size === 1 ? "track" : "tracks"}
                  </button>
                </Popover>
              </div>
            }
          />
        </aside>
      </div>
    </div>
  );
}

/* ---- the card's parts --------------------------------------------------------- */

type GlyphRect = readonly [number, number, number, number];

const LAYOUTS: readonly { value: Layout; label: string; glyph: readonly [number, number, readonly GlyphRect[]] }[] = [
  { value: "devlog", label: "Devlog", glyph: [30, 17, [[19, 9, 9, 6]]] },
  { value: "short", label: "Short", glyph: [12, 22, [[2, 2, 8, 8]]] },
  {
    value: "side-by-side",
    label: "Side by side",
    glyph: [
      30,
      17,
      [
        [1, 1, 13, 15],
        [16, 1, 13, 15],
      ],
    ],
  },
  {
    value: "desk",
    label: "Desk",
    glyph: [
      30,
      13,
      [
        [1, 6, 8, 6],
        [10, 6, 10, 6],
        [21, 6, 8, 6],
        [2, 1, 6, 4],
      ],
    ],
  },
];

const CORNERS: readonly Corner[] = ["top-left", "top-right", "bottom-left", "bottom-right"];

function Glyph({ w, h, rects, on }: { w: number; h: number; rects: readonly GlyphRect[]; on: boolean }) {
  return (
    <span className="relative block rounded-[2px] bg-sunk" style={{ width: w, height: h }}>
      {rects.map(([l, t, rw, rh], i) => (
        <i key={i} className={`absolute block rounded-[1px] ${on ? "bg-canary-deep" : "bg-ink-4"}`} style={{ left: l, top: t, width: rw, height: rh }} />
      ))}
    </span>
  );
}

function Section({ title, right, last, children }: { title: string; right?: ReactNode; last?: boolean; children: ReactNode }) {
  return (
    <section className={`px-4 pb-3 pt-2.5 ${last ? "" : "border-b border-line-2"}`}>
      <div className="flex items-center text-[13px] font-bold">
        {title}
        {right !== undefined && <span className="ml-auto text-[12px] font-medium text-ink-3">{right}</span>}
      </div>
      {children}
    </section>
  );
}

function GainRow({
  label,
  track,
  value,
  onChange,
  disabled,
}: {
  label: string;
  track: SessionTrack | null;
  value: number | null;
  onChange: (v: number | null) => void;
  disabled: boolean;
}) {
  const off = !track || disabled;
  return (
    <div className={`mt-[9px] flex items-center gap-2.5 ${off ? "opacity-50" : ""}`} title={!track ? `No ${label.toLowerCase()} track in this session` : disabled ? "Timelapse has no sound" : undefined}>
      <span className="w-[58px] shrink-0 text-[12px] text-ink-2">{label}</span>
      <Slider
        label={`${label} gain`}
        min={DB_MIN}
        max={DB_MAX}
        step={1}
        disabled={off}
        value={value ?? DB_MIN}
        onChange={(v) => onChange(v <= DB_MIN ? null : v)}
      />
      <span className="num w-[40px] whitespace-nowrap text-right text-[11px] text-ink-2">{track ? dbLabel(value) : "None"}</span>
    </div>
  );
}

function trackName(t: SessionTrack): string {
  switch (t.kind) {
    case "screen":
      return `Screen ${screenNumber(t)}`;
    case "webcam":
      return "Camera";
    case "mic":
      return "Mic";
    case "system-audio":
      return "System audio";
    default:
      return t.id;
  }
}

function Footer({
  job,
  estimate,
  estimateError,
  startError,
  sourceMs,
  onExport,
  onSavePreset,
  savedFlash,
  onDismissJob,
  raw,
}: {
  job: Job | null;
  estimate: ExportEstimate | null;
  estimateError: string | null;
  startError: string | null;
  sourceMs: number;
  onExport: () => void;
  onSavePreset: () => void;
  savedFlash: boolean;
  onDismissJob: () => void;
  raw: ReactNode;
}) {
  const p = job?.progress;

  if (job && p && p.state === "running") {
    const pct = Math.round(p.fraction * 100);
    return (
      <div className="border-t border-line-2 px-4 pb-3.5 pt-3">
        <div className="flex items-center text-[12px] text-ink-2">
          <span className="font-semibold text-ink">Exporting {job.title}</span>
          <span className="num ml-auto text-ink-3">
            {pct}%{p.etaSeconds !== null && p.etaSeconds !== undefined && p.fraction > 0.02 ? ` · about ${shortLength(p.etaSeconds * 1000)} left` : ""}
          </span>
        </div>
        <div className="mt-2.5 flex items-center gap-2">
          <div className="relative h-1.5 flex-1 overflow-hidden rounded-[3px] bg-sunk">
            <div className="absolute bottom-0 left-0 top-0 rounded-[3px] bg-ink transition-[width] duration-200" style={{ width: `${pct}%` }} />
          </div>
          <button
            className="flex h-10 items-center rounded-ctl border border-line bg-card px-3.5 text-[13px] font-semibold transition-colors duration-150 hover:bg-paper"
            onClick={() => void cancelExport(job.id).catch(() => {})}
          >
            Cancel
          </button>
        </div>
      </div>
    );
  }

  if (job && p && p.state === "done") {
    return (
      <div className="border-t border-line-2 px-4 pb-3.5 pt-3">
        <div className="flex items-center gap-1.5 text-[12px] text-ink-2">
          <IconCheck size={13} />
          <span className="truncate">{p.outputPath ? p.outputPath.split(/[\\/]/).pop() : job.title}</span>
        </div>
        <div className="mt-2.5 flex gap-2">
          <button
            className="flex h-10 flex-1 items-center justify-center rounded-ctl border border-ink bg-ink text-[13.5px] font-semibold text-white transition-opacity duration-150 hover:opacity-90"
            onClick={() => void revealExport(job.id).catch(() => {})}
          >
            Saved · Show in folder
          </button>
          <button className="flex h-10 items-center rounded-ctl border border-line bg-card px-3.5 text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper" onClick={onDismissJob}>
            Done
          </button>
        </div>
      </div>
    );
  }

  const failed = job && p && p.state === "failed" ? (p.error ?? "The export stopped") : null;
  const cancelled = job && p && p.state === "cancelled";
  const blocked = estimate?.error ?? estimateError;
  let line: ReactNode;
  if (failed) line = <span className="text-rec">{failed}</span>;
  else if (startError) line = <span className="text-rec">{startError}</span>;
  else if (blocked) line = <span className="text-rec">{blocked}</span>;
  else if (cancelled) line = <span>Cancelled · nothing was saved</span>;
  else if (!estimate) line = <span>Working out the size</span>;
  else {
    const changed = Math.abs(estimate.outputMs - estimate.sourceMs) > 1_000;
    line = (
      <span className="block truncate" title={`About ${shortLength(estimate.encodeSeconds * 1000)} to make, with ${estimate.encoder}`}>
        Composed MP4{changed ? ` · ${shortLength(estimate.outputMs)} long` : ""} · about {fmtSize(estimate.bytes)} · {shortLength(estimate.encodeSeconds * 1000)}
      </span>
    );
  }
  const note = !failed && !blocked && estimate?.notes.length ? estimate.notes.join(" · ") : null;

  return (
    <div className="border-t border-line-2 px-4 pb-3.5 pt-3">
      <div className="flex items-start gap-2 text-[12px] text-ink-3">
        <div className="min-w-0 flex-1 leading-[1.4]">{line}</div>
        <div className="shrink-0">{raw}</div>
      </div>
      {note && <div className="mt-1 text-[11px] leading-[1.35] text-ink-3">{note}</div>}
      <div className="mt-2.5 flex gap-2">
        <button
          className="flex h-10 flex-1 items-center justify-center whitespace-nowrap rounded-ctl border border-ink bg-ink text-[13.5px] font-semibold text-white transition-opacity duration-150 hover:opacity-90 disabled:opacity-40"
          disabled={Boolean(blocked)}
          onClick={onExport}
        >
          Export {lengthWords(sourceMs)}
        </button>
        <button
          className="flex h-10 items-center whitespace-nowrap rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper"
          onClick={onSavePreset}
        >
          {savedFlash ? "Preset saved" : "Save preset"}
        </button>
      </div>
    </div>
  );
}
