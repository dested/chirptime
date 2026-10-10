# ChirpTime UX audit

Status: done (2026-10-10, designer, bosun UX sprint)

This covers every current screen and state, captured from the running dev build
(Vite :5173) with a stubbed Tauri bridge that returns real data from
`C:\Users\dested\Videos\ChirpTime`. Widget shots are exactly 320×540. Viewer shots
are 1024×640, the widest bx can snap, downscaled 0.8. Nothing was recorded and
nothing on disk was changed.

Two mock artifacts in the before shots are not app bugs:
- Backslashes are missing from paths (`C:UsersdestedVideos…`). The stub dropped them.
- "Next capture in 178:07" comes from the stub's stale last-capture time.

## What the data says before any pixels

- **Sessions are the product. Clips are a fossil.**
  - Sessions: 30 sessions from Jul 28 to Oct 10, 174 GB.
  - Interval clips: 10 captures in the current output folder (8 of them on one day), plus 21 legacy days in `G:\code\chirptime\output` that the app no longer reads.
- **Nobody has ever placed a marker.** All 30 manifests have zero markers. The marker field in the recording widget is decoration.
- **A third of the sessions are junk.**
  - Several are 0–2 minutes long, and three on Aug 18 start within 30 s of each other.
  - These are double-presses and aborted starts, and nothing helps clean them up.
- **The longest day failed silently.** Session `2026-08-01_10-30-12` (8h 32m, 48.6 GB):
  - Screen 4 (iGPU / Quick Sync) died at 1:45 PM.
  - Screens 1–3 stalled at 2:02:20 PM ("ffmpeg stopped reporting progress"). They were restarted 20 times over about 11 minutes, then marked failed.
  - Camera, mic and system audio kept going until 7:02 PM.
  - The widget never said a word, so Sal recorded 5 hours believing he had his screens. This is the most important finding in the audit.
- **Gaps are stored oddly.** On that day every gap after the stall carries the same `from_epoch` (12728 s), so the gaps are cumulative and overlap. The current timeline draws them as stacked red failures.
- **Long empty stretches.** The webcam frames on Aug 1 show an empty chair from about 13:40 to 15:20 and from 17:00 to 18:20, plus a stretch with other people in frame.
  - "Cut 3 hours out of 8" is not hypothetical; it is that day.
  - Idle trimming did not catch it, because the screens had already died and the trim is keyed to keyboard and mouse input.

## Widget (320×540)

### Onboarding

Shots:
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-onboarding.png`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-onboarding-noffmpeg.png`

**For.** Picking the recordings folder, adopting the legacy folder, and getting FFmpeg installed.

**Wrong:**
- A whole first-run experience is crammed into a 320 px always-on-top strip, so it has to scroll. The no-FFmpeg state scrolls past its own heading.
- Eyebrows everywhere: "FIRST RUN", "SAVING TO", "THIS MACHINE".
- A red error box shows up on the very first screen a new user sees. A missing FFmpeg is a setup step, not an error.
- The legacy-folder banner ("21 days and 0 sessions") is a decision only Sal will ever face, yet it is presented to everyone.
- "5 to 8 GB per hour" is the most important fact on the page, and it sits below the fold.
- The settings path is shown in raw form.
- Nothing asks what to record. The monitors and camera are only chosen later, buried in Settings.

**Keep:**
- The honesty: "Nothing leaves it", the checksum-verified download, "no admin needed".
- The storage estimate.
- One-click FFmpeg fetch.

### FFmpeg missing banner (on the main widget)

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-ffmpeg-banner.png`

**Wrong:**
- The banner pushes the whole widget down, so tabs, Start and stats fall below the fold, and you can still press Start Recording.
- Three equal-weight buttons plus a link.

**Keep:** the single primary action, "Download FFmpeg (105 MB)".

### Clips mode, idle and recording

Shots:
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-clips-idle.png`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-clips-recording.png`

**For.** Taking a periodic screenshot plus a short clip every N minutes.

**Wrong:**
- It is a second product that halves the widget: a Clips | Session tab bar sits above everything.
- The data shows it is unused.
- Generic SaaS styling: sky-blue primary button, red full-width Stop, "Video" pills on every recent row.
- There is no thumbnail in "Recent", only timestamps.
- "TODAY 0 / LAST CAPTURE --:--" takes prime space.

**Keep:**
- The chirp before each capture, which belongs to the brand.
- "Capture now" as an idea. In the new design it becomes Mark.

### Session mode, idle

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-session-idle.png`

**Wrong:**
- About 60% of the window is empty.
- The stat row shows "0 SEGMENTS · 0 B · -- DISK FREE" (disk free is unknown until you record, although the recording state knows it).
- There is no hint of what will be recorded.
- "SETTINGS" is a collapsed eyebrow accordion.
- The idle state cannot take you to your last session in one click.

**Keep:** one big Start, and "Browse Sessions" at the bottom.

### Session mode, recording

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-session-recording.png`

**Wrong:**
- The track list shows raw ids ("DISPLAY1 (5120x2160)") and is cut off: the 4th monitor and the camera are below the fold.
- Health is a coloured dot per track with no words, so a failed screen looks almost the same as a healthy one. This is how the Aug 1 failure went unnoticed.
- The elapsed time is small red text in a pill, while "698 SEGMENTS" (a number nobody needs) gets a big stat cell.
- The "What just happened?" + Flag row has never been used (0 markers in 30 sessions). It costs a text field and a focus switch, when it should be a hotkey.

**Keep:**
- Stop is unmistakable.
- Written and disk free.
- "Open This Session" while recording.

### Session settings

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-widget-session-settings.png`

**Wrong:**
- Monitors are named `\\.\DISPLAY1`, `\\.\DISPLAY2`… and the user has to map those in their head.
- Checkboxes in a list give no sense of the physical layout.
- Segment length, an implementation detail, is exposed as a setting.
- Everything is at one level of importance.
- It lives inside a scrolling accordion in a 540 px window.

**Keep:**
- Per-monitor include/exclude.
- Device pickers.
- The idle-trim threshold.

## Viewer (separate 1280×800 window)

### Calendar (Clips)

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-viewer-calendar.png`

**Wrong:**
- The viewer opens here by default, on the mode with no data. August 2026 shows two clip days, while that month actually has 7 session days, which do not appear.
- The month grid is mostly empty cells.
- "Recent days" sits below the fold, with a "1 captures" plural bug.

**Keep:** nothing. The time map in the Library replaces it.

### Day gallery

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-viewer-day-gallery.png`

**Wrong:**
- Card thumbnails are mostly letterbox: wide desktop images squeezed into small cards.
- Times are repeated twice ("11:10 AM" and "11:10:30").
- "Video" and "Cam" chips on every card are noise.
- The first two cards read "12:05 AM" and "12:15 AM" and sort ahead of the 11 AM captures. They are either real after-midnight captures or a labelling bug; I didn't verify which.

**Keep:** the idea of glancing at a day as frames. It returns as the day ribbon in the player.

### Media viewer (Clips)

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-viewer-media-viewer.png`

**Wrong:**
- A 5-second clip floats in a sea of black; the frame is about 40% of the window.
- The webcam inset is a tiny tile in the top-right corner of the canvas.
- Native `<video>` controls clash with the custom top bar.
- The filmstrip is unlabelled.
- The trash icon sits next to the speed controls.

**Keep:** the screen/cam switcher concept (1 2 3 4 Cam).

### Session list

Shot: `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-viewer-session-list.png`

**Wrong:**
- It is a spreadsheet with no picture: an identical camera icon on every row.
- The raw session id is shown under every title.
- Columns: DURATION · TRACKS (always 7) · SEGMENTS (meaningless) · SIZE.
- There are no failure flags, and the 10-second abandoned sessions have the same weight as the 8.5-hour day.
- There is no notion of time of day or of the shape of a week.

**Keep:** reverse chronology, duration and size.

### Session player: grid, overlay, focus

Shots:
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-viewer-session-player.png`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-viewer-session-overlay.png`
- `G:\code\chirptime\plans\2026-10-10-ux-sprint\shots\before-viewer-session-focus.png`

**For.** Synchronized playback of every track. This is the heart of the app.

**Look and layout:**
- It looks like a different product from the light widget: saturated cyan, a purple webcam lane, and a neon "SPREAD 0ms max 0" readout, which is debug telemetry left on screen.
- Grid view gives the webcam the same tile size as a 5120-wide monitor and leaves an empty sixth cell.
- Overlay is the best idea in the app (monitors in their real arrangement), but it is not the default, the monitors float in black, and the camera covers a monitor.
- Focus works, but switching to it means hunting for the 1/2/3/4 buttons.

**Timeline:**
- Track rows use ids (`screen-0`, `sysaudio`).
- The idle gap is drawn as a red dashed failure, so the app tells the user that stepping away is an error.
- Every lane is a striped bar of identical segment blocks, with no image and no waveform. You cannot tell where anything happened without scrubbing.
- It is linear across the full session, so on an 8-hour day one pixel is about 30 seconds and the bottom lanes are clipped.

**Header and controls:**
- The header has four segmented groups (layout / screen / fit / sync) plus a muted/all/webcam/mic/sysaudio group, a speed group and the spread readout. That is 24 equally weighted buttons.

**Editing:**
- No editing at all: no selection, no delete, no export.
- The only destructive action in the whole app is deleting an entire session.

**Keep:**
- The sync engine and the master clock: it holds 0 ms spread across 7 tracks, which is impressive.
- Overlay's idea of the real monitor arrangement.
- 1–4 / Cam focus, and speed control.

## Cross-cutting

- **Two apps, not one.** The widget is light and sky-blue, the player is dark with cyan and purple, and the calendar is light and empty. No shared type scale, no shared colour, no shared icon set.
- **Eyebrows on nearly every block:** RECENT, TRACKS, SETTINGS, SAVING TO, THIS MACHINE, MONITORS, DURATION, SEGMENTS. Sal's rules ban these.
- **Implementation vocabulary leaks to the user:** segments, `\\.\DISPLAY1`, `screen-0`, sysaudio, spread, session ids.
- **Health is a dot, not a sentence.** Failures are never explained, never pushed to the user, and never summarized afterwards.
- **No hotkeys** for start, stop or mark, although the app is about not breaking flow.
- **The widget appears in its own recordings:** it sits always-on-top over the monitor being captured.
