/**
 * The app window's router: the hash, parsed into a typed route. No dependency;
 * every link is `href={href(route)}` or `navigate(route)`.
 *
 *   #/library                              (default)
 *   #/session/<id>?t=<epoch_ms>
 *   #/export/<id>?from=<ms>&to=<ms>
 *   #/marks  #/exports  #/settings
 */
import { useSyncExternalStore } from "react";

export type Route =
  | { name: "library" }
  | { name: "session"; id: string; t: number | null }
  | { name: "export"; id: string; from: number | null; to: number | null }
  | { name: "marks" }
  | { name: "exports" }
  | { name: "settings" };

export type RouteName = Route["name"];

function num(v: string | null): number | null {
  if (v === null || v === "") return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

export function parseHash(hash: string): Route {
  const raw = hash.replace(/^#\/?/, "");
  const [path = "", query = ""] = raw.split("?", 2);
  const q = new URLSearchParams(query);
  const [head, id] = path.split("/").map(decodeURIComponent);
  switch (head) {
    case "session":
      if (id) return { name: "session", id, t: num(q.get("t")) };
      break;
    case "export":
      if (id) return { name: "export", id, from: num(q.get("from")), to: num(q.get("to")) };
      break;
    case "marks":
    case "exports":
    case "settings":
      return { name: head };
  }
  return { name: "library" };
}

export function href(r: Route): string {
  switch (r.name) {
    case "session":
      return `#/session/${encodeURIComponent(r.id)}${r.t !== null ? `?t=${Math.round(r.t)}` : ""}`;
    case "export": {
      const q = new URLSearchParams();
      if (r.from !== null) q.set("from", String(Math.round(r.from)));
      if (r.to !== null) q.set("to", String(Math.round(r.to)));
      const qs = q.toString();
      return `#/export/${encodeURIComponent(r.id)}${qs ? `?${qs}` : ""}`;
    }
    default:
      return `#/${r.name}`;
  }
}

export function navigate(r: Route, opts: { replace?: boolean } = {}): void {
  const h = href(r);
  if (opts.replace) {
    history.replaceState(null, "", h);
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  } else {
    window.location.hash = h;
  }
}

/**
 * `open_viewer` still builds `/?mode=viewer&session=<id>&ts=<ms>` (and the old
 * `date` / `tab` links). Fold those onto the hash once at startup.
 */
export function migrateLegacyLink(): void {
  const params = new URLSearchParams(window.location.search);
  const session = params.get("session");
  if (!session && !params.has("date") && !params.has("tab")) return;
  const route: Route = session ? { name: "session", id: session, t: num(params.get("ts")) } : { name: "library" };
  // Keep `mode` (and the dev harness's `mock`) so the window still routes here.
  const keep = new URLSearchParams();
  for (const k of ["mode", "mock"]) {
    const v = params.get(k);
    if (v !== null) keep.set(k, v);
  }
  history.replaceState(null, "", `${window.location.pathname}?${keep.toString()}${href(route)}`);
}

function subscribe(cb: () => void): () => void {
  window.addEventListener("hashchange", cb);
  return () => window.removeEventListener("hashchange", cb);
}

const getHash = () => window.location.hash;

/** The current route; re-renders on every hash change. */
export function useRoute(): Route {
  const hash = useSyncExternalStore(subscribe, getHash);
  return parseHash(hash);
}
