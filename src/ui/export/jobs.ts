/**
 * Export jobs this window started, kept outside React so a job survives
 * leaving the composer: the Exports page shows it running, and coming back to
 * the composer picks its progress up again. One `export-progress` listener for
 * the whole window, attached on first use.
 */
import { useSyncExternalStore } from "react";
import { onExportProgress } from "./api";
import type { ExportProgress } from "./types";

export interface Job {
  id: string;
  sessionId: string;
  from: number;
  to: number;
  title: string;
  progress: ExportProgress;
  startedAt: number;
}

let jobs: readonly Job[] = [];
const subs = new Set<() => void>();
let listening = false;
/**
 * Progress for jobs not registered yet: a job can fail (or finish) before
 * `start_export`'s reply arrives, and its last word must not be lost.
 */
const early = new Map<string, ExportProgress>();
const EARLY_MAX = 50;

function set(next: readonly Job[]) {
  jobs = next;
  for (const f of subs) f();
}

/** Attach the window's one progress listener. Call before starting a job. */
export function ensureListening() {
  if (listening) return;
  listening = true;
  onExportProgress((p) => {
    if (!jobs.some((j) => j.id === p.jobId)) {
      early.set(p.jobId, p);
      if (early.size > EARLY_MAX) {
        const oldest = early.keys().next().value;
        if (oldest !== undefined) early.delete(oldest);
      }
      return;
    }
    set(jobs.map((j) => (j.id === p.jobId ? { ...j, progress: p } : j)));
  });
}

export function addJob(j: Omit<Job, "progress" | "startedAt">) {
  ensureListening();
  const progress: ExportProgress = early.get(j.id) ?? { jobId: j.id, fraction: 0, etaSeconds: null, state: "running" };
  early.delete(j.id);
  set([{ ...j, progress, startedAt: Date.now() }, ...jobs.filter((x) => x.id !== j.id)]);
}

/** Forget a finished job (its row now comes from the index). */
export function dropJob(id: string) {
  if (jobs.some((j) => j.id === id)) set(jobs.filter((j) => j.id !== id));
}

function subscribe(f: () => void) {
  ensureListening();
  subs.add(f);
  return () => {
    subs.delete(f);
  };
}

export function useJobs(): readonly Job[] {
  return useSyncExternalStore(subscribe, () => jobs);
}
