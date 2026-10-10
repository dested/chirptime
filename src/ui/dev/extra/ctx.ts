/** Shared contract between ../mockIpc.ts and the per-owner mock extras. Lead-owned. */
import type { SessionManifest } from "../../types";

export const UNHANDLED: unique symbol = Symbol("unhandled");
export type MockResult = unknown;

export interface MockCtx {
  /** The live in-memory fixture manifest (mutations are visible to every later call). */
  session(id: string): SessionManifest;
  /** Ids of every fixture session, newest first. */
  sessionIds(): string[];
  /** Emit a Tauri event to the page, e.g. `export-progress`. */
  emit(event: string, payload: unknown): void;
}
