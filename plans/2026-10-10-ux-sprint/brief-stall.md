# Brief: stall (opus-xhigh), why the screens die mid-session

Status: active · 2026-10-10 · Owner: stall · Lead: Opus 5.5
Re-read this file before every item and before asking. Standing answers at the bottom beat any message.

## The problem (unknown cause)

On 2026-08-01 Sal recorded 8.5 h, session `2026-08-01_10-30-12`.

What happened:
- Screens 1–3 (`screen-0..2`) stopped at about 2:02 PM; `screen-3` stopped at about 1:45 PM.
- Camera, mic and system audio ran 5 more hours.
- Nobody was told.

What the manifest shows:
- The `screen-0` gap reason is
  `AcquireNextFrame failed: 887a0026` (DXGI_ERROR_ACCESS_LOST) from `ddagrab`.
- Then "ffmpeg stopped reporting progress".
- The track gave up after `MAX_RESTARTS = 20`
  (`G:\code\chirptime\src-tauri\src\recorder.rs:36`, logic near line 2080).

Other sessions also carry many gaps on `screen-3` alone (h264_qsv, the iGPU
monitor), for example 2026-07-31_19-12-33, 2026-08-13_11-38-36 and
2026-08-14_07-19-46.

## What I want

1. **The root cause, with evidence.** Why does ddagrab lose access? Candidates:
   display sleep or power-off, the lock screen or secure desktop (UAC, Ctrl+Alt+Del),
   a mode or resolution change, a fullscreen-exclusive app, a GPU TDR, or a monitor
   reconnecting. Why does every restart then fail? Is it restarting into the same
   lost state too fast, or is the output index stale after the topology changed?
   And why did 20 restarts burn out in minutes while the outage lasted?

   Sources:
   - the session's `logs\screen-*.log`
   - the manifest's gaps and timestamps
   - the Windows event logs, read-only, around 2026-08-01 13:40–14:10 local
     (System and Application: display, power, Kernel-Power, Winlogon, Desktop
     Window Manager, LiveKernelEvent/TDR, Display)
   - the same for the screen-3 gaps in the other sessions

   `Get-WinEvent` with a time filter is fine.

2. **A fix in the recorder**, so a screen track recovers whenever capture becomes
   possible again, no matter how long the outage lasts:
   - No permanent give-up for an access-lost style failure. Use a backoff that
     caps at about 30 s and retries forever while the session runs, re-resolving
     the output (DXGI enumeration) on each attempt.
   - Keep the existing hard-fail path for errors that can't recover (an encoder
     missing, an output that's gone for good, and so on), and say which is which.
   - The gap reason must read as a sentence a person understands: "Display was
     locked or asleep", "Monitor disconnected", not the raw stderr. Keep the raw text in
     the track log.
   - Expose a recovery state the widget can show. `TrackStatus::Restarting` already
     exists; add what's needed to `SessionStatus`/`TrackSummary`, for example
     `last_error_human`, `waiting_since_epoch_ms`.
3. **Tests**: unit tests for the classification (stderr → recoverable or fatal → human
   reason) using the real stderr strings from the logs, and for the backoff
   schedule.

## Ownership

- Edit: `G:\code\chirptime\src-tauri\src\recorder.rs` only.
- If `SessionStatus`/`TrackSummary` change shape, send the lead the exact
  `src/ui/types.ts` lines; don't edit `types.ts`, which another agent owns.
- Read anything.
- Scratch: your own scratchpad folder.

## Hard safety rules

- **Never start a recording** (no `start_session`, no `tests/live_session.rs`, no
  plain `cargo test`). Run `cargo test --lib`.
- Read-only on `C:\Users\dested\Videos\ChirpTime`. Don't delete, move or rebuild anything there.
- Don't touch the running ChirpTime (pid 81356). Don't lock the screen or sleep displays to
  reproduce. If you need a live repro, describe the exact steps to the lead and stop.
- Read the Windows event logs only; change no system settings.
- Another agent (`cutter`) is editing `session.rs`, `commands.rs`, `lib.rs`,
  `types.ts`, `useApi.ts` and `engine.ts` in the same tree. Don't edit them.
  If both of you hit `target/` at once and cargo blocks, wait; don't delete the lock.

## Done when

- Root cause written up, with the evidence (log lines, event IDs, timestamps), in
  your final report. **No report files**: return the text (a hook blocks
  subagents from writing report files).
- The fix is committed via
  `bash ~/.claude/sal/bin/commit-paths.sh "<message> (stall)" src-tauri/src/recorder.rs`,
  and `cargo test --lib` exits 0.
- Report in ≤15 lines:
  - `ready stall <sha>`
  - the cause in 3 lines
  - the fix in 3 lines
  - the test exit code
  - what only a live test can prove, with the exact steps for Sal
  - doubts.

## Standing answers

(none yet)
