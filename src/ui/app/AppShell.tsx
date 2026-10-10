/**
 * The app window's frame: the sidebar (212 px) with the recorder card, or the
 * 60 px icon rail on the player and export routes when the window is under
 * 1600 px wide, so the stage gets the room.
 */
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { Route, RouteName } from "./router";
import { href } from "./router";
import { useAppData } from "./appData";
import { IconExport, IconFlag, IconLibrary, IconSliders, Mark } from "./icons";
import { fmtHms, fmtSize } from "../session/format";

export const SIDEBAR_W = 212;
export const RAIL_W = 60;
const WIDE_PX = 1600;
/** what a recorded hour costs on disk, for "about N h" */
const BYTES_PER_HOUR = 6.5e9;

function useWindowWidth(): number {
  const [w, setW] = useState(() => window.innerWidth);
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

interface NavItem {
  name: RouteName;
  label: string;
  icon: (p: { size?: number }) => ReactNode;
  count: number | null;
}

/** Library owns the player and export routes in the nav. */
function navOwner(r: Route): RouteName {
  return r.name === "session" || r.name === "export" ? "library" : r.name;
}

export function AppShell({ route, children }: { route: Route; children: ReactNode }) {
  const width = useWindowWidth();
  const { sessions, markCount } = useAppData();
  const collapsed = (route.name === "session" || route.name === "export") && width < WIDE_PX;
  const active = navOwner(route);

  const items: NavItem[] = [
    { name: "library", label: "Library", icon: IconLibrary, count: sessions?.length ?? null },
    { name: "marks", label: "Marks", icon: IconFlag, count: markCount },
    { name: "exports", label: "Exports", icon: IconExport, count: 0 },
    { name: "settings", label: "Settings", icon: IconSliders, count: null },
  ];

  return (
    <div className="flex h-full w-full bg-paper">
      {collapsed ? <Rail items={items} active={active} /> : <Sidebar items={items} active={active} />}
      <main className="relative min-w-0 flex-1">{children}</main>
    </div>
  );
}

function navHref(name: RouteName): string {
  switch (name) {
    case "session":
    case "export":
      return href({ name: "library" });
    default:
      return href({ name });
  }
}

function Sidebar({ items, active }: { items: NavItem[]; active: RouteName }) {
  return (
    <aside
      className="flex shrink-0 flex-col border-r border-line bg-rail px-2.5 py-3.5"
      style={{ width: SIDEBAR_W }}
    >
      <div className="flex items-center gap-2 px-2 pb-[18px] pt-0.5">
        <Mark size={18} />
        <span className="text-[14px] font-bold tracking-[-0.01em]">ChirpTime</span>
      </div>
      <nav className="flex flex-col gap-0.5">
        {items.map((it) => {
          const on = it.name === active;
          return (
            <a
              key={it.name}
              href={navHref(it.name)}
              className={`flex h-8 items-center gap-2.5 rounded-ctl px-2.5 text-[13px] no-underline transition-colors duration-150 ${
                on ? "bg-card font-semibold text-ink shadow-[0_1px_2px_rgba(0,0,0,.05)]" : "font-medium text-ink-2 hover:bg-card/60"
              }`}
            >
              <it.icon size={16} />
              {it.label}
              {it.count !== null && <span className="num ml-auto text-[11px] text-ink-3">{it.count}</span>}
            </a>
          );
        })}
      </nav>
      <div className="flex-1" />
      <RecorderCard />
    </aside>
  );
}

function Rail({ items, active }: { items: NavItem[]; active: RouteName }) {
  const { status, recorderBusy, start, stop } = useAppData();
  const recording = status?.recording ?? false;
  return (
    <aside
      className="flex shrink-0 flex-col items-center gap-1.5 border-r border-line bg-rail pb-3.5 pt-4"
      style={{ width: RAIL_W }}
    >
      <div className="mb-3">
        <Mark size={20} />
      </div>
      {items.map((it) => {
        const on = it.name === active;
        return (
          <a
            key={it.name}
            href={navHref(it.name)}
            title={it.label}
            aria-label={it.label}
            className={`grid h-9 w-9 place-items-center rounded-[9px] transition-colors duration-150 ${
              on ? "bg-card text-ink shadow-[0_1px_2px_rgba(0,0,0,.06)]" : "text-ink-3 hover:bg-card/60 hover:text-ink-2"
            }`}
          >
            <it.icon size={17} />
          </a>
        );
      })}
      <div className="flex-1" />
      <button
        className="grid h-9 w-9 place-items-center rounded-[9px] hover:bg-card/60 disabled:opacity-50"
        title={recording ? "Recording · Stop" : "Not recording · Start"}
        aria-label={recording ? "Stop recording" : "Start recording"}
        disabled={recorderBusy || status === null}
        onClick={() => void (recording ? stop() : start())}
      >
        {recording ? (
          <span className="dot-active h-3 w-3 rounded-full bg-rec" />
        ) : (
          <span className="h-3 w-3 rounded-full border-2 border-ink-3" />
        )}
      </button>
    </aside>
  );
}

function RecorderCard() {
  const { status, env, sessions, recorderBusy, recorderError, start, stop } = useAppData();
  const recording = status?.recording ?? false;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!recording) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [recording]);

  const recBytes = sessions?.reduce((a, s) => a + s.bytes, 0) ?? null;
  const total = env?.disk_total_bytes ?? null;
  const free = env?.disk_free_bytes ?? null;
  const recPct = total && recBytes !== null ? Math.min(100, (recBytes / total) * 100) : 0;
  const otherPct = total && free !== null ? Math.max(0, 100 - (free / total) * 100 - recPct) : 0;
  const elapsed = recording && status?.started_epoch_ms ? now - status.started_epoch_ms : null;

  return (
    <div className="rounded-[10px] border border-line bg-card p-3">
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 shrink-0 rounded-full ${recording ? "dot-active bg-rec" : "bg-ink-4"}`} />
        <span className="text-[12.5px] font-semibold">{recording ? "Recording" : "Not recording"}</span>
        {elapsed !== null && <span className="num text-[11px] text-ink-3">{fmtHms(elapsed)}</span>}
        <button
          className="ml-auto text-[12px] font-bold text-rec transition-opacity duration-150 hover:opacity-75 disabled:opacity-40"
          disabled={recorderBusy || status === null}
          onClick={() => void (recording ? stop() : start())}
        >
          {recording ? "Stop" : "Start"}
        </button>
      </div>
      <div className="mt-3 flex h-1.5 overflow-hidden rounded-[3px] bg-sunk">
        <span className="bg-ink" style={{ width: `${recPct}%` }} />
        <span className="bg-ink-4" style={{ width: `${otherPct}%` }} />
      </div>
      <div className="num mt-[7px] text-[10.5px] leading-normal text-ink-3">
        {recBytes !== null ? `${fmtSize(recBytes)} in recordings` : "Counting recordings"}
        <br />
        {free !== null ? `${fmtSize(free)} free · about ${Math.floor(free / BYTES_PER_HOUR)} h` : "Free space unknown"}
      </div>
      {recorderError && <div className="mt-2 text-[11.5px] leading-snug text-rec">{recorderError}</div>}
    </div>
  );
}
