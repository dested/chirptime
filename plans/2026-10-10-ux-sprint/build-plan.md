# ChirpTime redesign: build plan

Status: active · 2026-10-10 · Lead: Opus 5.5 (bosun)
Design source of truth:
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\report.md` (tokens, every screen)
- the mocks `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\*.html`, which open in a browser
- the canvas https://claude.ai/artifact/9nUzrnZ2U6iHiLoSY3gdwd

Decisions: `G:\code\chirptime\plans\2026-10-10-needs-sal.md` (all recs taken) plus SA-3
in `G:\code\chirptime\plans\2026-10-10-ux-sprint.md`.

## Slices, in order

| # | Slice | Owner | Gate |
| --- | --- | --- | --- |
| S0 | Foundation: tokens, fonts, app shell, router, dev harness, kill Clips + dead code, port 7460 | `shell` (opus-high) | sample |
| S1a | Player + delete flow (frontend) | `shell` (opus-high) | **sample: Sal reviews S0+S1 before anything else starts** |
| S1b | Thumbnail endpoint + cut purge (backend) | `thumbs` (opus-high) | sample |
| S2 | Library (time map, inspector, whole-session delete, short-session cleanup) | after the sample | |
| S3 | Recorder widget: idle/recording/alert/pill/options, tray, global hotkeys, content protection, cursor track | after the sample | |
| S4 | Settings page + first run in the app window | after the sample | |
| S5 | Export composer (ffmpeg filter_complex backend + UI) | after the sample | |
| S6 | Marks + Exports pages | after the sample | |
| Later | Away detection, transcription, OCR, storage view, dark mode, remove the Rust interval-capture commands | | |

---

## S0 — Foundation (`shell`)

### Tokens and type
- In `src/ui/app.css`, add the `@theme` block from report.md "Tailwind v4 tokens", verbatim.
  Body: `bg-paper text-ink font-sans text-[13px] antialiased`.
- **Fonts are bundled locally** (a desktop app must work offline). Use `bun add @fontsource-variable/schibsted-grotesk @fontsource-variable/geist-mono`,
  imported once in `main.tsx`. If either package doesn't exist under that name, search npm for
  the right one (don't guess). Failing that, download the woff2 files from Google Fonts into
  `src/ui/fonts/` with `@font-face` in app.css, and say which you did.
  Then update the font names in `@theme` to match the family names the packages register.

### Windows and routing
- Keep the two Tauri windows and their labels (`main` = recorder, `viewer` = the app), so
  `capabilities/default.json` doesn't change. Retitle the `viewer` window "ChirpTime".
  Its default size becomes 1280×800, min 1024×700, resizable (`commands.rs::open_viewer`
  builds it; `thumbs` owns commands.rs, so send the lead the exact lines).
- Add a hash router, in-house (~60 lines, no dependency): `src/ui/app/router.ts`. Routes:
  - `#/library` (default)
  - `#/session/<id>?t=<epoch_ms>`
  - `#/export/<id>?from=<ms>&to=<ms>` (placeholder page for now)
  - `#/marks`, `#/exports`, `#/settings` (placeholder pages, each with an h1 and one line saying it comes later)
- Keep `open_viewer`'s deep links working: map the old `?session=<id>&ts=<ms>` onto `#/session/<id>?t=<ms>`
  at startup. The old `?date=` and `?tab=` links go to `#/library`.

### Shell
- `src/ui/app/AppShell.tsx`: the sidebar exactly as in `mocks\library.html` (212 px):
  - Library / Marks / Exports / Settings with counts;
  - the recorder card at the bottom (status dot, Start/Stop wired to the real
    `start_session`/`stop_session`, the storage meter from `get_environment` + `list_sessions` bytes).
- It collapses to the 60 px icon rail (`mocks\player-a.html`) on the session and export routes
  when the window is under 1600 px wide.
- No "Recently deleted" entry.

### Dev harness (you need it to verify anything)
In a plain browser, `invoke()` fails and nothing serves media unless the real app is running.
Build:
- `scripts/make-fixture.ts` (run with bun): writes a synthetic session dir to
  `G:\code\chirptime\fixtures\sessions\<id>\`. Add `fixtures/` to `.gitignore`. It holds:
  - 4 screen tracks (testsrc2 with a burned-in drawtext wall clock, 3840×2160 → use 1280×720 to keep it light) and a webcam track, as 60 s segments;
  - mic + sysaudio (sine, 600 s);
  - a manifest that matches the real schema (`src/ui/types.ts`);
  - about 2 h of span, with one idle gap and one "screens stopped" gap.

  Make the segments short in bytes: low res, `-preset ultrafast -crf 40`.
- `scripts/fixture-server.ts` (bun): serves `fixtures/` at `http://localhost:9120/files/...` with
  Range support, and `/thumb/...` with the same contract as S1b (it can shell out to ffmpeg the
  same way). It refuses to start if 9120 is taken, which means the real app is up.
- `src/ui/dev/mockIpc.ts`: active only when `import.meta.env.DEV` and `?mock=1`. It uses
  `@tauri-apps/api/mocks` (`mockIPC`, `mockWindows`) to answer `list_sessions`, `get_session`,
  `get_session_status`, `get_environment`, `get_settings`, `delete_range`, `undo_cut`,
  `finalize_cuts`, `delete_session`, `undo_delete_session` against the fixture manifests.
  delete/undo mutate an in-memory copy, so the UI flow is fully clickable.
- Document both in `G:\code\chirptime\verify.md`; the lead merges the text you send.

### Kill list (Sal said yes: Q5, Q33)
- Delete these dead files: `src/ui/App.tsx`, `src/ui/components/`, `src/index.ts`, `src/viewer.ts`,
  `src/capture/`, `src/utils/`, `src/viewer-ui/`. Check nothing live imports them first.
- `ViewerApp.tsx`: the calendar, the day gallery and the media viewer go. The viewer window now
  renders `AppShell`.
- `CompactApp.tsx`: the Clips pane and its mode switch go. The widget is sessions-only. Its
  restyle is S3; for now, only remove Clips and keep it working.
- `SessionBrowser.tsx` goes. The S1a Library placeholder is a plain list of sessions linking to
  `#/session/<id>`, until S2.
- The Rust interval-capture commands stay for now; nothing calls them.

### Port (Q32)
- Vite on **7460**, strict: `vite.config.ts`, plus `devUrl` in `src-tauri/tauri.conf.json`.

---

## S1a — Player + delete flow (`shell`)

Build `#/session/<id>` to match these mocks at 1280×800 and 1920×1080:
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a.html`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-select.html`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-confirm.html`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-deleted.html`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-1920.html`

Reuse `SyncEngine` / `TrackController` / `TrackSurface` (engine.ts already skips cuts).
Replace `SessionPlayer.tsx` and `Timeline.tsx` with new components under `src/ui/player/`,
and delete the old ones once nothing imports them.

### Behaviour, settled

- **Header**:
  - the day as title (`Saturday, August 1`), then `10:30 AM – 7:02 PM · 8h 32m · 48.6 GB` in mono.
    Length and size exclude cuts (use `cuts.ts`).
  - an amber pill **only** when a screen track has a non-idle gap over 60 s:
    `Screens stopped at 2:02 PM` (one screen: `Screen 4 stopped at 1:45 PM`).
  - Tracks (a popover: one row per track, with kind, resolution, codec, segments, bytes, gaps and status)
    and Export (ink, routes to `#/export/<id>?from&to`).
- **Stage**:
  - **Focus** (default) shows one screen large: screen-0, or the largest by pixel count. The camera
    tile and the mini desk map (drawn from `track.monitor` rects) sit under it.
    - **There is no cursor data yet**, so the caption reads `Screen 1 · press 1–4 to switch, D for the desk`,
      not "Following your mouse". Cursor-follow comes with the S3 cursor track.
    - Keys 1–4 pin a screen.
  - **Desk** (key D) shows every monitor at its real position from `track.monitor` + `manifest.desktop`,
    with the camera in the largest empty corner.
- **Log card**, derived by a pure function in `src/ui/player/log.ts` from the manifest, so it can be checked in a bun script:
  - `Started recording` + a track summary line.
  - Each marker: `Mark` + its label, or "Add a note" when it has none (notes editing is S6, so display only).
  - Idle gaps (reason starts with `idle`): `Away 1h 20m` + `Not recorded, no keyboard or mouse`.
  - Screen gaps over 60 s that aren't idle: `Screen N stopped recording` (or `Screens 1–3 …` when they
    start within 2 min of each other), with a sub-line from a human-reason map:
    - `887a0026`, `access lost`, `stopped reporting` → "The display went dark";
    - the new recorder's readable reasons pass through;
    - anything else → "Recording failed".

    A matching `Screen N back` line goes where segments resume.
  - Each cut: `Cut 1h 43m`; while Pending, `· Undo for Ns`.
  - `Stopped recording`, or `Interrupted` when `ended_epoch_ms` is null and the session isn't live.
  - Clicking a line seeks. The current line gets the canary wash. There's an All / Marks filter.
  - Presence lines ("Left the desk", "with company") need away detection, which is phase 2: **don't fake them**.
- **Transport**:
  - play/pause (Space);
  - the wall clock `11:31:04 AM` written imperatively from `subscribeFrame` (CLIFFNOTES: never React state at 60 Hz);
  - `elapsed / total` in session time with cuts collapsed;
  - ½×, 1×, 2×, 4×, 16×;
  - Mark, disabled with a tooltip unless the session is live;
  - mute and audio sources.
- **Day ribbon**:
  - The session's full span with cuts collapsed: a cut shows as a 2 px red seam with a `−1h 43m` label.
  - Webcam thumbnails tiled across the width, from S1b's `/thumb` at ~30-min spacing, or as many as fit at 64 px wide.
  - Idle gaps get a paper veil with `Away 1h 20m`. Landmarks sit under the ribbon: marks and screens-stopped.
  - An ink zoom bracket (draggable, default 2 h around the playhead) and the playhead with its canary knob.
- **Zoom strip**: the bracketed region, showing focused-screen thumbnails at ~10-min spacing,
  or webcam frames, labelled `Camera only after 2:02 PM`, where the focused screen has no footage.
- **Selection**:
  - Drag on the ribbon or the zoom strip. I / O set in and out at the playhead. A log "Delete" button selects that range.
  - Edges snap within 8 px to gap edges, marks, cut edges and segment boundaries, and say so:
    `Snapped to a gap` / `to a mark` / `to the session start`.
  - The selection bar replaces the transport:
    `1h 43m` (canary chip) · `1:38 PM – 3:21 PM` · size estimate · `All tracks ▾` (choose tracks) · Export (E) · Delete (X) · ×.
    - The size estimate sums the bytes of segments inside the range, prorated at the edges.
  - Delete ▾ also offers: `Delete everything before the playhead`, `Delete everything after the playhead`.
    ("All away stretches over N min" waits for away detection.)
- **Confirm row**:
  - `Delete 1h 43m? Frees 6.2 GB now. Undo for 10 seconds.`, with Cancel (Esc) and Delete (red, Enter).
  - The range gets a red outline and a light wash. No hatching, no modal.
- **After delete**:
  - Call `deleteRange(id, from, to, tracks)`, re-read the manifest (`get_session`) and rebuild the engine with the playhead kept.
  - The range collapses (240 ms).
  - Toast: `Deleted 1h 43m · freed 6.2 GB · Undo` (Ctrl+Z also undoes), with a canary countdown bar that runs until `undo_until_epoch_ms`.
  - Undo calls `undoCut` and re-reads. Errors show in the toast in plain words.
- **Copy rules**: sentence case; no eyebrows; no periods in headings; every number in Geist Mono with `tabular-nums`.

### Thumbnails contract (S1b provides it; the fixture server mimics it)
`GET http://localhost:9120/thumb/<session_id>/<track_id>/<epoch_ms>.jpg?w=<width>` returns a JPEG.
- Round `epoch_ms` down to 10 s in the frontend so the cache hits.
- 404 inside a gap or a cut.

---

## S1b — Thumbnails + cut purge (`thumbs`)

- `fileserver.rs`: add the `/thumb/...` route.
  - Resolve the segment through the on-disk manifest, using `session::read_manifest` and the cuts sidecar
    (the live session: read from disk, which is acceptable).
  - Return 404 inside a cut or with no segment.
  - Extract with `ffmpeg -ss <offset> -i <segment> -frames:v 1 -vf scale=<w>:-2 -q:v 6`.
    `w` is clamped to 64..960.
  - Cache at `<session>/thumbs/<track_id>/<w>/<epoch_ms>.jpg`, written atomically. At most 3 concurrent ffmpeg extractions
    (a tokio semaphore). Same traversal guard and `silent_command()` as everything else.
  - Thumbnails for a segment still being written (no moov yet) are a 404, not an error.
- `edit.rs`: when a cut is finalized, and when undo is impossible, **delete every cached thumbnail inside the cut's range for the affected
  tracks**. Deleted footage must not survive as JPEGs. Also delete them at delete time (they
  regenerate if the cut is undone). `delete_session` removes everything anyway.
- `list_sessions` / `visible_sessions` ignore `thumbs/`, and `rebuild_manifest_from_disk` must not index it.
- `commands.rs` `open_viewer`: the window is 1280×800, min 1024×700, resizable, title "ChirpTime". Apply
  the lines `shell` sends, or make that change itself.
- Tests: route resolution (pure), cache path, a cut purge on a synthetic session, and the 404 cases.
  `cargo test --lib` + `cargo test --test edit_cuts` + a new `--test thumbs`.

---

## Ownership for the sample

| Owner | Files |
| --- | --- |
| shell | `src/ui/**` (except the files `thumbs` owns), `scripts/**`, `vite.config.ts`, `src-tauri/tauri.conf.json` (devUrl only), `package.json` + `bun.lock` (its font deps only), deletions in the kill list, `index.html` |
| thumbs | `src-tauri/src/fileserver.rs`, `src-tauri/src/edit.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/session.rs`, `src-tauri/tests/thumbs.rs` (new), `src-tauri/tests/edit_cuts.rs` |
| stall-fix | `src-tauri/src/recorder.rs` |
| lead | `.gitignore`, `CLIFFNOTES.md`, `ui.md`, `decisions.md`, `updates.md`, `verify.md`, plans/ |

## Safety (both)

- Never act on `C:\Users\dested\Videos\ChirpTime`. Fixtures only.
- Never start a recording. Never run plain `cargo test`.
- Sal's dev app is currently **not running**. Don't start `bun run dev` (`tauri dev`). Verify in a browser with the
  harness (`bun run vite:dev` on 7460 plus the fixture server on 9120). Stop both before reporting.
- Use bx for every browser check, and look at the screenshots at full size.

## Done for the sample

- `bunx tsc --noEmit` and the cargo suites exit 0.
- bx screenshots of the Player at 1280×800 and 1920×1080 (playing, selected, confirm, deleted with toast,
  Desk view) against the fixture, saved to `G:\code\chirptime\plans\2026-10-10-ux-sprint\build-shots\`
  (gitignored). Compare them side by side with the mocks.
- Report in ≤12 lines with the shas, the checks and their exit codes, and the shot paths.

## Standing answers

- **SB-1:** `thumbs` makes the open_viewer window change itself. `shell` sends nothing for it.
- **SB-2:** commits go only through `bash ~/.claude/sal/bin/commit-paths.sh "<msg> (<agent>)" <own paths>`, deleted paths included. Never push, no stash/reset/checkout --/amend. The lead owns .gitignore; `fixtures/` and `build-shots/` are already ignored.
- **SB-3:** a hook blocks subagents from writing report files. Your hand-off is message text.
