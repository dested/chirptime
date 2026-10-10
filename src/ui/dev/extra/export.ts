/**
 * Mock IPC handlers owned by `export` (round 2). Return `UNHANDLED` for any
 * command this file doesn't answer; the core mock in ../mockIpc.ts tries the
 * extras before its own switch.
 */
import { UNHANDLED, type MockCtx, type MockResult } from "./ctx";

export function handleExport(cmd: string, a: Record<string, unknown>, ctx: MockCtx): MockResult {
  void a;
  void ctx;
  switch (cmd) {
    default:
      return UNHANDLED;
  }
}
