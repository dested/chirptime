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

function set(next: readonly Job[]) {
  jobs = next;
  for (const f of subs) f();
}

function ensureListening() {
  if (listening) return;
  listening = true;
  onExportProgress((p) => {
    if (!jobs.some((j) => j.id === p.jobId)) return;
    set(jobs.map((j) => (j.id === p.jobId ? { ...j, progress: p } : j)));
  });
}

export function addJob(j: Omit<Job, "progress" | "startedAt">) {
  ensureListening();
  const progress: ExportProgress = { jobId: j.id, fraction: 0, etaSeconds: null, state: "running" };
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
