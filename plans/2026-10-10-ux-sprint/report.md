# ChirpTime redesign

Status: active (2026-10-10, designer, bosun UX sprint), revised for SA-3. Sal took every recommendation in `G:\code\chirptime\plans\2026-10-10-needs-sal.md` and `G:\code\chirptime\plans\2026-10-10-ux-sprint\questions.md`, with these overrides:
- The player looks like the rest of the app.
- Delete frees disk immediately, with an undo toast.
- The canvas uses placeholders only.
- Light only, for now.

> Lead note: the edit model's implementation truth is `G:\code\chirptime\plans\2026-10-10-ux-sprint\brief-cutter.md` (cuts in a `cuts.jsonl` sidecar merged into the manifest, finalize after 12 s, startup recovery). Where this report differs, the brief wins.

## Framing

**ChirpTime is a flight recorder for a working day.** It runs from morning to night without supervision, then lets you go back to any moment. Mostly you go back to cut dead time, and sometimes to pull a piece out for a devlog or a bug repro.

1. **Trustworthy and quiet recording.** Aug 1 lost 5 hours of screens silently. Health is pushed as a sentence, and the recorder stays out of the footage.
2. **The day is the unit.** Every browsing surface is organised by clock time, with pictures.
3. **Editing is subtractive and coarse.** It needs a day-scale overview with landmarks, one gesture to select, one key to delete, and an undo. It is not a video editor.
4. **Export is the payoff.** Devlog is the default layout (Q1).

## Information architecture

**Two windows:**
- **Recorder**: 320×540, collapsing to a 264×52 pill, tray-first. States: idle, recording, alert, options, pill.
- **The app**: 1280×800 default. Screens: first run, Library, Player, Export, Marks, Exports, Settings.

**What dies:**
- Clips mode, with its calendar, gallery and media viewer. The chirp stays as the start/stop cue.
- The widget's onboarding, which moves into the app as first run.
- The "What just happened?" + Flag field, replaced by Mark and Ctrl Alt M.

**What changes:**
- The session list becomes the Library time map.
- The widget's settings split: recording options stay in the recorder, the rest goes to Settings.

**Shell:**
- A sidebar with Library, Marks, Exports, Settings and a recorder card (status, Start, storage meter).
- At 1280 the player and Export collapse it to a 60 px icon rail so the stage gets room. At 1920 the full sidebar stays.
- No Recently deleted.

## Visual system

**One surface: Paper.**
- Warm light ground #F5F3EE, white cards, ink #1A1917, warm hairlines.
- The only dark thing is the footage stage: a #0B0B0A mat framing the monitors, like thumbnails in the Library. The export preview works the same way.
- Light only for now. A Windows-following dark mode comes later, in neutral graphite, never purple.

**Colour:**
- Canary #FFD84D is the one accent, used as a highlighter: selection, the playhead knob, marks, the current log line.
- Primary buttons are ink.
- Red means recording or deletion only: the record dot, Stop, the Delete button, the confirm outline, the cut seam.
- Amber means health only.
- Being away is a paper veil labelled "Away 1h 43m", never an error.

**Type:**
- Schibsted Grotesk for UI, Geist Mono for every number. Newsreader is dropped.
- 13 px body, 12 px secondary, 11 px mono metadata. Headings at 14, 20, 22 and 34 px.
- No eyebrows, and no terminal periods in headings.

**Shape and motion:**
- Radii: 12 cards, 8 controls, 6 chips.
- Motion: 160 ms hover and press, 220 ms selection snap, 240 ms collapse of a cut, the toast's 10 s countdown bar, and the record dot breathing at 1.6 s.

**Tailwind v4 tokens:**
```css
@theme {
  --font-sans: "Schibsted Grotesk", "Segoe UI", system-ui, sans-serif;
  --font-mono: "Geist Mono", "Cascadia Mono", ui-monospace, monospace;
  --color-paper: #F5F3EE; --color-rail: #EFECE5; --color-card: #FFFFFF; --color-sunk: #EEEBE4;
  --color-ink: #1A1917; --color-ink-2: #55524B; --color-ink-3: #8C887F; --color-ink-4: #BCB7AC;
  --color-line: #E3DFD5; --color-line-2: #ECE9E2; --color-mat: #0B0B0A;
  --color-canary: #FFD84D; --color-canary-deep: #E8B800; --color-canary-ink: #2B2200;
  --color-rec: #E5372B; --color-rec-line: #F2C4BF;
  --color-amber: #C2680A; --color-amber-wash: #FFF3DF; --color-amber-line: #F3D9AE;
  --radius-card: 12px; --radius-ctl: 8px; --radius-chip: 6px;
  --ease-out-soft: cubic-bezier(.2,.7,.2,1);
}
```

## Screens

The mocks use real Aug 1 frames; the recorder's live thumbnails are from Oct 8. The presence data, log wording and the mark are illustrative. The canvas uses neutral placeholders.

**Recorder** (unchanged from round one):
- Mocks: `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\widget-idle.html`, `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\widget-recording.html`, `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\widget-alert.html`, `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\widget-settings.html`, `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\widget-pill.html`.
- Idle shows a desk map, the ink Start with Ctrl Alt R, and the last session.
- Recording shows a big clock, a live-thumbnail desk map that doubles as the health map, the canary Mark, Pause and Stop.
- Alert explains the Aug 1 failure in words, with Restart screens and a Windows toast.
- The pill is 264×52.
- Options hold only what changes the next recording.

**First run and Library:**
- Mocks: `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\first-run.html` and `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\library.html`.
- First run is three steps in the app window.
- The Library is a time map of days × hours with camera-thumbnail blocks and amber dots on failures, a clean-up offer for short sessions, and an inspector.

**Player** (`G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a.html` and `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-1920.html`), top to bottom:
1. **Header**: back to Library, the day as title, times / length / size in mono. An amber health pill appears only when something broke. On the right, Tracks (a popover with per-track detail and health) and Export (ink).
2. **Stage**, the one dark surface.
   - Defaults to Focus, following the mouse (Q23): the followed screen large, with a thin canary edge.
   - Under it: the camera, a mini desk map, "Following your mouse · press 1–4 to pin, D for the desk", and a Focus | Desk switch.
   - Desk shows every monitor in its real arrangement, with the camera in the empty corner.
3. **Log**, beside the stage (B's idea, folded in).
   - The day as lines: started, now, Mark, "Left the desk for 1h 43m", "Screens 1–3 stopped recording", "Back at the desk, with company", stopped.
   - Click a line to jump to it.
   - Away lines carry "Delete 1h 43m". The current line has a canary wash. An All / Marks filter.
4. **Transport**: play in an ink circle, the wall clock (11:31:04 AM) with elapsed / total, ½× to 16×, Mark (M), mute.
5. **Day ribbon**, one calm strip.
   - Camera thumbnails every 30 minutes, with hour labels above.
   - Away stretches veiled and labelled.
   - Landmarks under it: Mark, and Screens stopped.
   - An ink zoom bracket, and the playhead (ink line, canary knob).
6. **Zoom strip**: screen frames every 10 minutes for the bracketed region, with ticks and mark flags. It switches to camera frames, labelled "Camera only after 2:02 PM", once the screens are dead.
   - No track lanes and no waveforms on screen. Per-track detail is in the Tracks popover.

**Editing** (`G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-select.html`, `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-confirm.html`, `G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\player-a-deleted.html`):
1. **Select.** Drag on the ribbon or zoom strip, click Delete on an away line in the log, or press I and O.
   - The range gets a canary outline; outside it rests under a paper veil.
   - Edge time chips, plus "Snapped to when you left the desk". Edges snap to presence changes, idle edges, marks and failures.
   - The other away stretch is offered as a dashed box, "Add away 1h 20m ⇧A".
2. **Selection bar**, replacing the transport: 1h 43m (canary chip) · 1:38 PM – 3:21 PM · 6.2 GB · All tracks ▾ (Q11) · Export (E) · Delete (X) · ×.
   - The Delete menu holds the one-click actions: before the playhead, after it, and all away stretches over N minutes (Q13).
3. **Confirm**, a quiet row in the same place: "Delete 1h 43m? Frees 6.2 GB now. Undo for 10 seconds." with Cancel (Esc) and Delete (red, ↵). The range turns red-outlined with a light wash. No hatching.
4. **Deleted.**
   - The range collapses from the ribbon and the zoom strip, leaving a thin red seam marked −1h 43m. The session keeps a hole (Q14).
   - The header shows 6h 49m · 42.4 GB.
   - The log line reads "Cut 1h 43m · Left the desk · Undo for 8 s".
   - An ink toast reads "Deleted 1h 43m · freed 6.2 GB · Undo (Ctrl Z)" with a canary countdown bar, about 10 s, then it's permanent.

**Export** (`G:\code\chirptime\plans\2026-10-10-ux-sprint\mocks\export.html`), Paper in the same shell.
- **Left side:** the live 16:9 preview on the dark mat, with the camera corner outlined in canary and draggable. Below it, the range as the same strip, with canary edge chips.
- **Right card:**
  - Layout: Devlog, Short, Side by side, Desk.
  - Screen: Follow mouse (default) or 1–4.
  - Camera: corner, Rounded/Circle, size.
  - Sound: mic and system gain, and "Lower system audio while I talk".
  - Speed: 1×, 2×, 4×, Timelapse, plus "Skip idle stretches" (Q18).
  - Size: Original, 1080p, Share · 25 MB (Q17).
  - Footer: the estimate, a secondary Raw tracks option (Q15), "Export 24 minutes" (ink), Save preset.

## Edit model and costs (SA-3)

**At delete (about 1 s):**
1. Append the cut to `cuts` in `session.json`. The engine skips cuts like gaps.
2. Move the fully covered segment files into a session-local pending folder.
3. Show the toast. Sizes already exclude the pending files.

**Undo, within about 10 s:** move the files back and drop the cut.

**Commit, after 10 s or on quit:**
- Unlink the pending files.
- Trim the two boundary segments in the background with `ffmpeg -c copy`. Video lands on a 2 s-GOP keyframe, at most 2 s survives and stays masked. Audio trims to about 21 ms.
- Write the manifest before touching files. On startup, finish or roll back any leftover pending folder.
- No re-encode, ever.

| Item | Cost |
| --- | --- |
| Cuts, engine skip and ribbon collapse | M, 3–4 days |
| Pending-delete, undo and commit with trims | M, about 3 days |
| Per-track cuts | S |
| Range actions | S |
| Derived day log | S–M (presence lines need away detection) |
| Away detection | M |
| Thumbnails on segment close | S |
| Export composer | L, 1.5–2 weeks (two-pass encode for Share ≤25 MB; Follow mouse needs a cursor track, S) |

## Features beyond the asks, ranked

1. Health that speaks.
2. Fix the stall at its root.
3. Keep the recorder out of capture.
4. Thumbnails on segment close.
5. Away detection with ⇧A.
6. Global hotkeys.
7. Export presets, including Share ≤25 MB.
8. Privacy pause.
9. OCR search.
10. Local transcription (phase 2).
11. Storage view, with retention rules off by default.
12. Auto-discard sessions under 60 s, with undo.
13. A designed chirp.

## Claude Design canvas

- The artboards live in `G:\code\chirptime\plans\2026-10-10-ux-sprint\canvas\project\`, listed in `G:\code\chirptime\plans\2026-10-10-ux-sprint\canvas\project\canvas.json`; `G:\code\chirptime\plans\2026-10-10-ux-sprint\canvas\project\Main.dc.html` is the player.
- They are flattened from the rendered mocks: inline styles, no scripts, no classes, and only the Google Fonts link.
- Every footage still is a neutral placeholder: a dark tile with drawn window chrome and a label such as "Screen 1 · editor".
