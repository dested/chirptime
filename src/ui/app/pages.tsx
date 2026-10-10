/**
 * The Library until S2 builds the time map, and the pages that come later.
 */
import { useAppData } from "./appData";
import { href } from "./router";
import { fmtDayTitle, fmtSize, fmtSpan, fmtTimeAmPm } from "../session/format";

export function LibraryPage() {
  const { sessions, sessionsError, status } = useAppData();
  const liveId = status?.recording ? status.session_id : null;
  const totalMs = sessions?.reduce((a, s) => a + s.duration_ms, 0) ?? 0;
  const totalBytes = sessions?.reduce((a, s) => a + s.bytes, 0) ?? 0;

  return (
    <div className="h-full overflow-y-auto px-8 pb-8 pt-6">
      <h1 className="text-[24px] font-bold tracking-[-0.02em]">Library</h1>
      {sessions && (
        <div className="num mt-1 text-[11px] text-ink-3">
          {sessions.length} {sessions.length === 1 ? "session" : "sessions"} · {Math.round(totalMs / 3_600_000)} hours ·{" "}
          {fmtSize(totalBytes)} on disk
        </div>
      )}

      {sessionsError && <p className="mt-6 text-[13px] text-rec">Could not list sessions: {sessionsError}</p>}
      {sessions === null && !sessionsError && <p className="mt-6 text-ink-3">Loading sessions</p>}
      {sessions?.length === 0 && (
        <p className="mt-6 max-w-[420px] text-ink-2">
          Nothing recorded yet. Press Start in the recorder and your day shows up here.
        </p>
      )}

      {sessions && sessions.length > 0 && (
        <ul className="mt-6 max-w-[720px] overflow-hidden rounded-card border border-line bg-card">
          {sessions.map((s, i) => {
            const live = s.id === liveId;
            const end = s.ended_epoch_ms ?? s.started_epoch_ms + s.duration_ms;
            return (
              <li key={s.id} className={i ? "border-t border-line-2" : ""}>
                <a
                  href={href({ name: "session", id: s.id, t: null })}
                  className="flex items-center gap-4 px-4 py-3 transition-colors duration-150 hover:bg-paper"
                >
                  <span className="w-[200px] shrink-0 text-[13px] font-semibold">{fmtDayTitle(s.started_epoch_ms)}</span>
                  <span className="num text-[11.5px] text-ink-3">
                    {fmtTimeAmPm(s.started_epoch_ms)} – {live ? "now" : fmtTimeAmPm(end)}
                  </span>
                  {live && (
                    <span className="flex items-center gap-1.5 text-[11.5px] font-semibold text-rec">
                      <span className="dot-active h-1.5 w-1.5 rounded-full bg-rec" />
                      Recording
                    </span>
                  )}
                  {!s.has_manifest && <span className="text-[11.5px] font-semibold text-amber">No manifest</span>}
                  <span className="num ml-auto text-[11.5px] text-ink-2">{fmtSpan(s.duration_ms)}</span>
                  <span className="num w-[72px] text-right text-[11.5px] text-ink-3">{fmtSize(s.bytes)}</span>
                </a>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

export function LaterPage({ title, line }: { title: string; line: string }) {
  return (
    <div className="px-8 pt-6">
      <h1 className="text-[24px] font-bold tracking-[-0.02em]">{title}</h1>
      <p className="mt-2 text-ink-2">{line}</p>
    </div>
  );
}
