# Brief: stall-fix (opus-high), the review fixes for recorder commit 9d1299a

Status: active · 2026-10-10 · Owner: stall-fix · Lead: Opus 5.5
Re-read this file before every item and before asking. Standing answers at the bottom beat any message.

## Context

Commit `9d1299a` (by `stall`, now stopped) made screen tracks wait out display
outages instead of giving up. The cause is diagnosed: displays power off at idle
(Kernel-Power 566). ddagrab then fails with 887a0026 or access denied, a relaunch
into a dark display never gets a first frame, and the old 20-restart cap burned
out during the outage. Read `G:\code\chirptime\plans\2026-10-10-ux-sprint\brief-stall.md`
for the original brief, and `git show 9d1299a` for the code.

A read-only review found no blocker, but asked for the fixes below. All of them are in
`G:\code\chirptime\src-tauri\src\recorder.rs`. Line refs are as of 9d1299a.

## Fixes (each with a test where feasible)

1. **No silent monitor swap** (:2753, :499). `relocate_before_relaunch`
   overwrites the stored identity with whatever matched, and `relocate_screen`
   doesn't check whether another screen track already claims that output. If the two
   3840×2160 monitors swap `\\.\DISPLAYn` names while the layout settles on wake,
   one monitor gets recorded twice and the other never, for the rest of the session.
   - Keep each track's session-start identity immutable and match against it every time.
   - Skip any candidate whose name + rect equals another screen track's identity.
   - When the match is ambiguous, prefer the original output index.
   - Add a name-swap test.
2. **GpuBusy and outages keep the restart budget until the track has produced
   footage this session** (:2244). A QSV encoder that can never open must still reach
   `Failed` with its raw error, not show "Graphics card ran out of memory" forever.
   Once a track has recorded, outages retry forever as now.
3. **Missing and OtherAdapter** (:2486, :2521).
   - After 10 consecutive `Missing` results, try one launch at the original output index
     (then go back to Missing if it fails).
   - Only fail permanently on `OtherAdapter` after it shows up in 3 consecutive enumerations.
4. **Restarts count once per outage** (:2624), not once per attempt. A 10 h display-off
   must not read as "1200 restarts".
5. **`split_pair` keeps the mic's state** (:2905). Move the mic's `OutputState` (open
   `Downtime`, `idle_tail`) into the solo mic supervisor, so its first segment doesn't add
   an overlapping "dropped time" gap and its health isn't stale.
6. **Cheaper while away** (:60). While the system's last input is older than 60 s
   (`Inner.last_input_ms` from the idle poller), space outage attempts at 120 s. The wake
   shortcut (retry 3 s after input) already covers the return.
7. **The static-desktop question** (the review's live-only doubt). Can a lit but
   unchanging desktop (user idle ≥30 s) fail to give ddagrab its first frame within the
   5 s `NoFrames` deadline, so a healthy screen loops on NoFrames?
   - Investigate from the ffmpeg ddagrab source and docs, and from the existing logs in
     `C:\Users\dested\Videos\ChirpTime\sessions\*\logs\screen-*.log` (read-only).
   - If the risk is real, fix it, e.g. ddagrab `dup_frames=1` or `draw_mouse`, a first-launch
     exemption, or classifying NoFrames as an outage only while the display is known to be
     off. Say which, and why.
8. **Tests for the `supervise_track` state machine.** These paths are untested:
   - Unknown promoted to an outage;
   - the outage count resetting on footage;
   - the Missing no-launch path;
   - stop during a wait (it must return within ~6 s);
   - the resumed path in `index_segment`.

   Factor the decisions into pure functions if that's what it takes to test them. Don't
   spawn real ffmpeg against real displays.

## Ownership

- Edit: `G:\code\chirptime\src-tauri\src\recorder.rs` only.
- Another agent, `cutter`, is editing `edit.rs`, `session.rs`, `commands.rs`, `lib.rs`,
  `state.rs`, `types.ts`, `useApi.ts`, `engine.ts` and `tests/edit_cuts.rs` in the same tree.
  Don't touch those. If a shared cargo build blocks, wait.

## Hard safety rules

- Never start a recording. No `start_session`, no `tests/live_session.rs`, no plain
  `cargo test`. Run `cargo test --lib`.
- Read-only under `C:\Users\dested\Videos\ChirpTime`.
- No system changes: don't lock the screen, sleep the displays or change power settings.

## Done when

- `cargo test --lib` exits 0 in the real tree. If cutter's in-progress edits block
  compiling, run it on a scratch copy made with `git archive` and say so.
- Committed via
  `bash ~/.claude/sal/bin/commit-paths.sh "<message> (stall-fix)" src-tauri/src/recorder.rs`
  (one or a few commits).
- Report in ≤12 lines: `ready stall-fix <sha>`, one line per fix, your answer on item 7,
  the test exit code, and any doubts. Return text only; a hook blocks report files.

## Standing answers

(none yet)
