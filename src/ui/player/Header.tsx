/** Player header: back, the day, times / length / size, the health pill, Tracks and Export. */
import { useState } from "react";
import type { SessionManifest, SessionTrack, TrackKind } from "../types";
import { href } from "../app/router";
import { IconBack, IconExport, IconLayers, IconWarn } from "../app/icons";
import { fmtDayTitle, fmtSize, fmtSpan, fmtTimeAmPm } from "../session/format";
import { isExpectedGap, isIdleReason, outageHeadline, screenNumber } from "./timeline";
import { Popover } from "./Popover";

interface Props {
  manifest: SessionManifest;
  live: boolean;
  totals: { start: number; end: number; lengthMs: number; bytes: number };
  wide: boolean;
  onSeek: (t: number) => void;
  onExport: () => void;
}

export function Header({ manifest, live, totals, wide, onSeek, onExport }: Props) {
  const [tracksOpen, setTracksOpen] = useState(false);
  const headline = outageHeadline(manifest);
  const pill = headline
    ? headline.screens.length === 1
      ? `Screen ${headline.screens[0]} stopped at ${fmtTimeAmPm(headline.at)}`
      : `Screens stopped at ${fmtTimeAmPm(headline.at)}`
    : null;

  return (
    <div className="flex h-16 shrink-0 items-center gap-3">
      <a
        href={href({ name: "library" })}
        className="-ml-1.5 flex h-8 items-center gap-[7px] rounded-ctl pl-1 pr-2 text-[12.5px] font-semibold text-ink-2 transition-colors duration-150 hover:bg-card/70 hover:text-ink"
      >
        <IconBack size={15} />
        Library
      </a>
      <div className="h-5 w-px bg-line" />
      <h1 className={`whitespace-nowrap font-bold tracking-[-0.02em] ${wide ? "text-[22px]" : "text-[20px]"}`}>
        {fmtDayTitle(manifest.started_epoch_ms)}
      </h1>
      <div className="num min-w-0 truncate whitespace-nowrap text-[11.5px] text-ink-3">
        {fmtTimeAmPm(totals.start)} – {live ? "now" : fmtTimeAmPm(totals.end)} · {fmtSpan(totals.lengthMs)} · {fmtSize(totals.bytes)}
      </div>
      {pill && headline && (
        <button
          className="flex h-[26px] shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[13px] border border-amber-line bg-amber-wash px-2.5 text-[12px] font-semibold text-[#8A4A05] transition-colors duration-150 hover:bg-[#FFEBCB]"
          onClick={() => onSeek(headline.at)}
          title="Jump there"
        >
          <IconWarn size={13} />
          {pill}
        </button>
      )}
      <div className="flex-1" />
      <div className="relative">
        <button
          className={`flex h-8 items-center gap-[7px] whitespace-nowrap rounded-ctl border border-line px-3 text-[12.5px] font-semibold transition-colors duration-150 ${
            tracksOpen ? "bg-sunk" : "bg-card hover:bg-paper"
          }`}
          onClick={() => setTracksOpen((v) => !v)}
          aria-expanded={tracksOpen}
        >
          <IconLayers size={15} />
          Tracks
        </button>
        <Popover open={tracksOpen} onClose={() => setTracksOpen(false)} align="right" className="w-[520px] p-0">
          <TracksTable manifest={manifest} />
        </Popover>
      </div>
      <button
        className="flex h-8 items-center gap-[7px] whitespace-nowrap rounded-ctl border border-ink bg-ink px-3 text-[12.5px] font-semibold text-white transition-opacity duration-150 hover:opacity-90"
        onClick={onExport}
      >
        <IconExport size={15} />
        Export
      </button>
    </div>
  );
}

const KIND_NAME: Record<TrackKind, string> = {
  screen: "Screen",
  webcam: "Camera",
  mic: "Mic",
  "system-audio": "System audio",
};

export function trackName(t: SessionTrack): string {
  return t.kind === "screen" ? `Screen ${screenNumber(t)}` : KIND_NAME[t.kind];
}

function TracksTable({ manifest }: { manifest: SessionManifest }) {
  return (
    <div>
      <div className="px-3.5 pb-2 pt-3 text-[14px] font-bold">Tracks</div>
      <table className="w-full border-collapse text-[12px]">
        <thead>
          <tr className="text-left text-[11px] text-ink-3">
            <th className="px-3.5 pb-1.5 font-medium">Track</th>
            <th className="px-2 pb-1.5 font-medium">Format</th>
            <th className="px-2 pb-1.5 text-right font-medium">Segments</th>
            <th className="px-2 pb-1.5 text-right font-medium">Size</th>
            <th className="px-2 pb-1.5 text-right font-medium">Gaps</th>
            <th className="px-3.5 pb-1.5 font-medium">Status</th>
          </tr>
        </thead>
        <tbody>
          {manifest.tracks.map((t) => {
            const bytes = t.segments.reduce((a, s) => a + s.bytes, 0);
            const failures = t.gaps.filter((g) => !isExpectedGap(g.reason) && g.to_epoch_ms - g.from_epoch_ms >= 1000).length;
            const away = t.gaps.filter((g) => isIdleReason(g.reason)).length;
            const bad = t.status === "failed" || failures > 0;
            const video = t.kind === "screen" || t.kind === "webcam";
            return (
              <tr key={t.id} className="border-t border-line-2 align-top">
                <td className="px-3.5 py-2">
                  <div className="flex items-center gap-2 font-semibold">
                    <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${bad ? "bg-[#E39A3B]" : "bg-ink-4"}`} />
                    {trackName(t)}
                  </div>
                  <div className="mt-0.5 max-w-[150px] truncate pl-3.5 text-[11px] text-ink-3" title={t.label}>
                    {t.label}
                  </div>
                </td>
                <td className="num px-2 py-2 text-[11px] text-ink-2">
                  {video ? `${t.width}×${t.height} · ` : ""}
                  {t.codec}
                </td>
                <td className="num px-2 py-2 text-right text-[11px] text-ink-2">{t.segments.length}</td>
                <td className="num px-2 py-2 text-right text-[11px] text-ink-2">{fmtSize(bytes)}</td>
                <td className="num px-2 py-2 text-right text-[11px] text-ink-2" title={`${failures} failures, ${away} away`}>
                  {failures + away}
                </td>
                <td className="px-3.5 py-2 text-[11.5px]">
                  <span className={bad ? "font-semibold text-[#8A4A05]" : "text-ink-2"}>{statusText(t, failures)}</span>
                  {t.error && <div className="mt-0.5 max-w-[150px] truncate text-[11px] text-ink-3" title={t.error}>{t.error}</div>}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function statusText(t: SessionTrack, failures: number): string {
  if (t.status === "failed") return "Failed";
  if (t.status === "recording" || t.status === "starting") return "Recording";
  if (t.status === "restarting") return "Restarting";
  if (failures) return failures === 1 ? "Stopped once" : `Stopped ${failures} times`;
  return "Fine";
}
