# Brief: cutter (opus-high), delete a block of time, backend + engine

Status: active · 2026-10-10 · Owner: cutter · Lead: Opus 5.5
Re-read this file before every item and before asking. Standing answers at the bottom beat any message.

## Goal

Sal records 8-hour sessions and wants to cut a block of time (say 3 h) out of a
session and get the disk back. The redesigned player UI comes later, from another
agent. You build everything under it: the cut model, the file operations, undo,
the Tauri commands, the TS types/wrappers, and the playback engine skipping cuts.

## Settled product rules (don't revisit)

- A delete applies to all tracks by default; an optional `tracks` list limits it
  (e.g. drop only `mic` over a private call).
- **Disk is freed now, with a ~10 s undo.** No 7-day bin.
- A cut leaves a hole: the session keeps its id, and the cut is remembered
  forever, so playback and the timeline can collapse it.
- Cut precision is about a second. Video edges snap outward to keyframes (2 s GOP), so up to
  2 s of footage inside a cut may physically survive. It must stay masked by the cut forever.
- **Never re-encode** for delete. Stream copy only.
- Refuse a delete on the session that is currently recording:
  `Err("Stop recording before deleting from this session")`.
- Deleting a whole session (Library trash button, "5 sessions under two minutes")
  uses the same pattern: instant, 10 s undo, then gone.

## The model

### Sidecar `cuts.jsonl`, one JSON object per line, in the session dir

It works like `markers.jsonl` (`session::append_marker` / `read_markers`). It is the
durable record, so `rebuild_manifest_from_disk` keeps the cuts; the rebuild must
read it.

```rust
// session.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cut {
    pub id: String,                 // "cut-<epoch_ms of creation>"
    pub from_epoch_ms: i64,
    pub to_epoch_ms: i64,
    /// None = every track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<String>>,
    pub created_epoch_ms: i64,
    /// Pending = inside the undo window, files parked in .trash. Final = files gone.
    pub state: CutState,
    #[serde(default)]
    pub freed_bytes: u64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CutState { Pending, Final, Undone }
```

- `SessionManifest` gains `#[serde(default)] pub cuts: Vec<Cut>`, holding only cuts
  whose state is not `Undone`.
- `read_manifest` and `get_session` populate it from `cuts.jsonl`, with the last line per id winning.
- Append a new line on every state change. Never rewrite the file in place.

### `delete_range(id, from_epoch_ms, to_epoch_ms, tracks: Option<Vec<String>>) -> CutReceipt`

1. Validate: `from < to`; the range intersects the session; the session isn't live;
   every named track exists.
2. `plan_cut(&manifest, from, to, tracks) -> CutPlan`. This is a **pure function**:
   - For each affected track, sort its segments into:
     - `remove`: the segment lies wholly inside `[from, to]`.
     - `trim_head`: keep `[seg.start, from)`.
     - `trim_tail`: keep `[to, seg.end)`.
     - `split`: the cut lies strictly inside one segment, so keep both sides. This
       is common for audio, whose segments are 600 s long.
   - Segments are ragged and can overlap (CLIFFNOTES "Segments are never uniform…"),
     so use each segment's own `start_epoch_ms` / `duration_ms`, never
     `index * segment_seconds`.
   - A kept piece shorter than 1000 ms becomes a full `remove` instead.
3. Move every `remove` file (and its `mic-16k` sidecar WAV, when the track is `mic`
   and a WAV covers the range; see below) into
   `<session>/.trash/<cut_id>/<track_dir>/` with `std::fs::rename`. That is the same
   volume, so it's instant. Write `<session>/.trash/<cut_id>/plan.json` holding the
   removed `Segment` records, the planned trims, and any markers that fall inside the
   range.
4. Update the manifest: take the removed segments out, and drop markers inside
   the range from the manifest (keep them in plan.json for undo). Write the manifest
   atomically (`write_manifest`), then append the `Pending` cut to `cuts.jsonl`.
   Ordering rule: **the manifest is written before anything is deleted, and
   nothing is deleted before the manifest no longer references it.**
5. Return
   `CutReceipt { cut: Cut, freed_bytes_estimate: u64, undo_until_epoch_ms: i64 }`.
   - `undo_until` is now + 10 000.
   - The estimate is the bytes of the removed files plus each trimmed
     segment's bytes × the cut fraction.
6. Schedule `finalize_cuts(id)` 12 s from now on the tokio runtime.

### `undo_cut(id, cut_id) -> SessionManifest`

- Only while the cut is `Pending`; otherwise
  `Err("That delete can no longer be undone")`.
- Move the files back, restore the segments (keeping each track's segments
  sorted by start) and the markers, write the manifest, and append `Undone`.
- Remove `.trash/<cut_id>`.

### `finalize_cuts(id) -> SessionManifest`

For every `Pending` cut:
1. Do the trims and splits with ffmpeg stream copy, writing to a temp name in the
   track dir.
   - For a tail or split second half, find the real keyframe at or before the cut
     end with ffprobe and start the copy there: `-ss <kf> -i in -c copy`.
   - The new segment's `start_epoch_ms` = the old start + kf ms. Its
     `duration_ms` and `bytes` come from `session::probe_segment` on the new file.
   - Name kept pieces the way the recorder names segments (look at
     `epoch_from_segment_name` and the recorder's `-strftime` pattern), by their
     real start second, so a rebuild reproduces the same starts. If the name
     collides with an existing file, append `-1`, `-2` and make sure
     `epoch_from_segment_name` still parses it.
   - Apply the same treatment to the mic-16k WAVs.
2. Write the manifest with the new segment records, then delete the originals and
   `.trash/<cut_id>`, then append `Final` with the real `freed_bytes`.
3. A failure on one trim (ffmpeg error, unreadable segment) leaves that original
   untouched and logs it to `<session>/logs/edit.log`. The cut still masks it,
   and the rest of the work continues.

Also: at app start (`lib.rs` setup, on a background thread), scan the sessions
root for any `cuts.jsonl` with a `Pending` cut older than 10 s, plus any
`sessions/.trash/*`, and finalize them. That's the crash-recovery path. Undo is
only guaranteed while the app stays open.

### Whole sessions

- `delete_session(id) -> SessionTrashReceipt { id, bytes, undo_until_epoch_ms }`:
  - Refuse the live session.
  - Rename `sessions/<id>` → `sessions/.trash/<id>`.
  - Schedule a purge 12 s later (`remove_dir_all`).
- `undo_delete_session(id)`: rename it back while it's still there.
- `list_sessions` must skip `.trash`.

### mic-16k sidecars

- `<session>/mic-16k/*.wav` hold the mic as 16 kHz mono "for later transcription"
  and are **not** in the manifest.
- They contain the same speech, so a cut that covers `mic` (or all tracks) must
  cut them too.
- Map a WAV to its time span by its filename's start time plus the duration from
  ffprobe.
- PCM trims are exact with `-c copy`.

## Playback: `src/ui/session/engine.ts`

- A cut is a hole that playback **skips**.
- When the master clock enters a cut on a track set that covers every track, jump the clock to
  `cut.to_epoch_ms`. This happens in the rAF loop, on the same path as segment boundaries.
- A per-track cut instead makes that track render as a gap inside the cut (`segmentAt`
  returns null), while the other tracks keep playing.
- Add a pure helper `export function collapsedTime(manifest, epochMs)` and its inverse, mapping
  between wall-clock epoch and "session time with cuts removed". The new timeline
  will use them.
- Unit-test them only if a test runner already exists. None does, so put the
  pure helpers in `src/ui/session/cuts.ts` with no DOM imports, and check them
  with a `bun` script in your scratchpad. Don't add a test runner.

## Frontend contract

- `src/ui/types.ts`: `Cut`, `CutState`, `CutReceipt`, `SessionTrashReceipt`, and
  `cuts: Cut[]` on `SessionManifest`. These must match serde exactly (CLIFFNOTES
  warns about a past mismatch).
- `src/ui/hooks/useApi.ts`: typed wrappers `deleteRange`, `undoCut`, `finalizeCuts`,
  `deleteSession`, `undoDeleteSession`.
- No UI. Don't touch `SessionPlayer.tsx`, `Timeline.tsx`, `SessionBrowser.tsx`,
  `CompactApp.tsx` or `ViewerApp.tsx`.

## Ownership (yours only)

- New: `G:\code\chirptime\src-tauri\src\edit.rs` (everything about cuts),
  `G:\code\chirptime\src-tauri\tests\edit_cuts.rs`, `G:\code\chirptime\src\ui\session\cuts.ts`
- Edit: `G:\code\chirptime\src-tauri\src\session.rs` (Cut types, cuts field,
  sidecar read, rebuild keeps cuts), `G:\code\chirptime\src-tauri\src\commands.rs`
  (only the new commands plus a `.trash` skip if `list_sessions` lives there),
  `G:\code\chirptime\src-tauri\src\lib.rs` (`mod edit`, handler registration, startup recovery),
  `G:\code\chirptime\src\ui\types.ts`, `G:\code\chirptime\src\ui\hooks\useApi.ts`,
  `G:\code\chirptime\src\ui\session\engine.ts`
- If `list_sessions` lives in `recorder.rs`, which another agent owns, **don't edit it**.
  Send the lead the exact lines instead.

## Hard safety rules

- **Never run anything against `C:\Users\dested\Videos\ChirpTime`.** That's Sal's
  real footage, 174 GB. No delete, rename, trim or finalize there, and no test
  pointed at it. Every test builds a synthetic session in a temp dir, with segments
  generated by ffmpeg `lavfi` (`testsrc2` at `-g 60` for video, `sine` for AAC and
  PCM WAV) and real filenames.
- Never start a recording. Never run plain `cargo test` (it would run
  `tests/live_session.rs`). Run `cargo test --lib` and
  `cargo test --test edit_cuts`.
- ChirpTime is running (pid 81356, :9120, Vite :5173). Leave it alone. If
  `cargo` can't write `target/` because the running dev build holds it, tell
  the lead; don't kill anything.

## Tests that define done

`G:\code\chirptime\src-tauri\tests\edit_cuts.rs` and unit tests in `edit.rs`:
- `plan_cut`:
  - a cut wholly inside one segment (split);
  - a cut spanning many segments;
  - ragged and overlapping segments;
  - a cut touching the session start and the end;
  - per-track `tracks`;
  - a kept piece under 1 s becoming a remove;
  - a range outside the session (error).
- End to end on synthetic footage:
  - delete → files in `.trash`, manifest updated, `cuts.jsonl` Pending;
  - undo → byte-identical restore (hash the files);
  - delete → finalize → trimmed files probe as playable, the manifest's starts and
    durations match ffprobe within 50 ms, no file referenced by the manifest is
    missing, and no file in a track dir is unreferenced;
  - after finalize, `rebuild_manifest_from_disk` yields the same segment starts
    (±1 s) and still carries the cut;
  - a crash between the move and the finalize (simulate it: skip the finalize,
    then run startup recovery) ends Final with no orphans;
  - `delete_session` → undo → still listed; `delete_session` → purge → gone.
- `cargo test --lib`, `cargo test --test edit_cuts`, `cargo test --test settings_persistence`
  and `bunx tsc --noEmit` (from `G:\code\chirptime`) all exit 0.

## Commits and hand-off

- Commit only through
  `bash ~/.claude/sal/bin/commit-paths.sh "<message> (cutter)" <your paths>`.
  Small commits are fine. Never push. No stash, reset, checkout --, or amend.
- Report to the lead in ≤10 lines:
  `ready cutter <sha> <one line>`, then the four check commands with their exit
  codes, then any doubts.
- Questions: `blocked cutter: <what>`, then keep going on whatever doesn't depend
  on the answer.

## Standing answers

**SA-C1 (2026-10-10):** markers inside a range are dropped only for an all-track cut (`tracks: None`). A per-track cut (e.g. mic only) keeps every marker. The rebuild filter follows the same rule.

**SA-C2 (2026-10-10):** wait for the lead's reviewer findings before any other changes; they arrive as one `fix cutter` message.

**SA-C3 (2026-10-10): fix list from the read-only review of 13cf3c6. Do all of it, one commit per numbered item or a few grouped, each with a test where one is feasible.** Line refs are as of 13cf3c6. You may also edit `G:\code\chirptime\src-tauri\src\state.rs` (AppState) for item 1.

1. **The live-refusal gap while a recording stops.** `stop_session` (commands.rs:823) takes the recorder out of state before `rec.stop()` finishes, so `editable_session` (commands.rs:939) passes while the recorder's final `flush()` can still overwrite session.json. Keep a `stopping_id: Option<String>` in AppState until `stop()` returns, and refuse it in `editable_session` (this covers delete_range, undo, finalize, delete_session and rebuild).
2. **Startup recovery must not replay a Final cut's journal** (edit.rs:1093-1098). Final means the manifest is already written: the Final branch never touches the manifest. It only deletes leftover journal originals that the manifest doesn't reference, then removes the trash folder.
3. **Never delete an original on ffmpeg's word alone** (edit.rs:889-905, 737-742). Accept a piece only if a head runs at least `planned_len - 100 ms` and a tail ends within 250 ms of the original's end, both measured by ffprobe on the piece. Anything else is a failed trim: keep the original and log to `logs/edit.log`.
4. **Undo is all-or-nothing.** If any step of `undo_cut` fails, roll back what it did and leave the cut Pending but marked `undo_failed` (journal it), so the timer's finalize does nothing to it until the next explicit finalize. Refuse undo (`Err("That delete is already being finalized")`) when `finalize.json` exists.
5. **No orphan pieces on a crash mid-finalize.** Write pieces as `*.cut.tmp` and rename them to their final `.mp4` names inside `apply_journal`, after the journal exists. Recovery sweeps any `*.cut.tmp`.
6. **`rebuild_session` (commands.rs:911) runs under `EDIT_LOCK`.**
7. **`check_id` (edit.rs:260) is an allowlist:** `^[0-9A-Za-z_-]+$`, with no `.` and no space. Apply it to session ids and cut ids at every command entry.
8. Minor:
   - a. `append_cut` writes `"\n"` first if the file doesn't end with one (torn last line).
   - b. Startup recovery schedules `finalize_due_cuts` for Pending cuts under 10 s old.
   - c. `piece_name` (edit.rs:769) uses `.earliest()`, not `.single()` (DST fall-back hour).
9. **Lead's decision on MIN_PIECE_MS: never lose footage outside the range.** A kept piece under 1 s does not turn the segment into a remove. Leave the original segment untouched instead (the cut masks the in-range part); it frees less disk, which is fine.
10. **Tests to add** (the review found these uncovered): a crash mid-finalize, then replay; overlapping cuts; refusals for a recording and for a stopping session; a failed undo, then the timer, with nothing deleted; id checks at the command level (`"x."`, `"x "`, `"..\y"`); a short or damaged piece keeping its original (item 3).

Done when all four checks exit 0 in the real tree. Report `ready cutter <sha>` with the exit codes.

**SA-C4 (2026-10-10): apply stall's TrackSummary lines (from recorder commit 9d1299a), in its own small commit, after SA-C3 or between items, whichever is cleaner.**
- `G:\code\chirptime\src\ui\types.ts`, inside `export interface TrackSummary`, after `error: string | null;`:
  ```ts
    /** Why the track is not recording, as a sentence ("Display was locked or asleep"). Null while it records. */
    last_error_human: string | null;
    /** Set while a screen waits out an outage. It keeps retrying until capture works or the session stops. */
    waiting_since_epoch_ms: number | null;
    /** When the next attempt is due, while it is restarting or waiting. */
    next_attempt_epoch_ms: number | null;
  ```
- `G:\code\chirptime\src-tauri\src\commands.rs`: in `pub struct TrackSummary`, after `pub error: Option<String>,` add
  `#[serde(flatten)] pub health: crate::recorder::TrackHealth,`; in `SessionStatus::from_recorder`, after `error: t.error.clone(),` add
  `health: rec.track_health(&t.id),`.

**SA-C5 (2026-10-10):** a9c3521 is accepted.
- A session whose recorder stopped itself and has finished stopping (nothing left to flush or write) is editable. Refuse only while it is recording or while `stop()` is still running. Add a unit test on `editable_dir` for that case.
- SA-C4 is still not applied (no `last_error_human` in types.ts or commands.rs): do it now.
