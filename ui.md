# ChirpTime — UI / Visual Language

> The source of truth for how ChirpTime looks and feels: the **Paper** system
> (redesign sprint, 2026-10-10). Follow it for anything visual and keep it
> current when the UI changes. Tokens live in `src/ui/app.css`. The design
> report is `plans/2026-10-10-ux-sprint/report.md`.

## North star

**A flight recorder for a working day, drawn on warm paper.** It is calm, light
and precise. The footage is the only dark thing on screen. It is not a video
editor: no NLE lanes, no After Effects chrome, no dark purple anything. The
recorder widget, the app shell, the Player and the export composer are one
system, with the same ground, type, buttons and radii.

## Tokens (`@theme` in `src/ui/app.css`)

| Token | Value | Use |
| --- | --- | --- |
| `paper` | `#F5F3EE` | App ground |
| `rail` | `#EFECE5` | Sidebar / icon rail |
| `card` | `#FFFFFF` | Cards, inspector, popovers, secondary buttons |
| `sunk` | `#EEEBE4` | Segmented-control troughs, hover on ghost buttons |
| `ink` · `ink-2` · `ink-3` · `ink-4` | `#1A1917` · `#55524B` · `#8C887F` · `#BCB7AC` | Text from primary down to faint/disabled; `ink` is also the primary button and the toast |
| `line` · `line-2` | `#E3DFD5` · `#ECE9E2` | Hairlines: edges, then dividers |
| `mat` | `#0B0B0A` | The footage mat, and only that |
| `canary` · `canary-deep` · `canary-ink` | `#FFD84D` · `#E8B800` · `#2B2200` | The one accent |
| `rec` · `rec-line` | `#E5372B` · `#F2C4BF` | Recording and delete |
| `amber` · `amber-wash` · `amber-line` | `#C2680A` · `#FFF3DF` · `#F3D9AE` | Health |
| `rounded-card` · `rounded-ctl` · `rounded-chip` | 12 · 8 · 6 px | Cards / controls / chips |
| `ease-out-soft` | `cubic-bezier(.2,.7,.2,1)` | Every enter and collapse |

The widget body is `#FBFAF7` with a `line` border and 12 px radius, drawn on a
transparent window.

## Colour rules

- **Canary is the one accent**, used like a highlighter: the selection, the
  playhead knob, marks, the current log line, the camera corner in the export
  preview, and the toast countdown. Never decoration, never a large fill.
- **Primary buttons are ink** (Start, Export, Open), not canary.
- **Red means recording or deleting, nothing else:** the record dot (breathing),
  Stop, the Delete button, the confirm row's outline, the cut seam on the ribbon.
- **Amber means health only:** a screen outage, a failed track, the header's
  "Screens stopped at 2:02 PM" pill, the Library's outage dot. (The paused
  status dot also uses amber.)
- **Being away is not an error.** It is a paper veil labelled "Away 1h 20m", never red or amber.
- Light only for now. A dark mode that follows Windows comes later: neutral graphite, never purple.

## Type

- **Schibsted Grotesk** (UI) and **Geist Mono** (numbers), bundled with `@fontsource-variable`.
- Body is 13 px, secondary 12–12.5 px, metadata 11–11.5 px. Headings are 14, 20, 22 and 34 px.
- **Every number** (times, durations, sizes, counts) uses `.num` (Geist Mono + `tabular-nums`), so nothing jitters as it ticks.

## Copy

- Sentence case everywhere. Plain words over jargon: "The display went dark",
  "Left the desk for 1h 45m", "Stop recording to edit notes".
- **No eyebrows**: no kicker or label above a heading.
- **No periods at the end of headings.** Body sentences keep theirs.
- Durations read `1h 43m`, wall clock `11:31:04 AM`, ranges `1:38 PM – 3:21 PM`, sizes `6.2 GB`.
- Keyboard hints sit in `.kbd` chips next to the action (`Esc`, `↵`, `Ctrl Alt R`).

## Shape, elevation, motion

- **Hairlines, not shadows.** Use the `line` borders. Shadows are kept for the
  ink toast and the lifted primary button, plus a 1–2 px drop on chips that sit on footage.
- Motion:
  - 150–160 ms for hover and press;
  - 200 ms for the toast's entrance (`.toast-in`);
  - 220 ms for the selection snap;
  - 240 ms for a cut collapsing;
  - a linear countdown bar on the undo toast for the remaining undo time;
  - the record dot breathing at 1.6 s (`.dot-active`).

## Component idioms (copy these, don't invent new ones)

- **App shell** (`app/AppShell.tsx`):
  - The 212 px sidebar on `rail`: Library, Marks, Exports and Settings with mono counts.
  - The recorder card at the bottom: status dot, Start/Stop/Resume, and the storage meter.
  - It collapses to a 60 px icon rail on the Player and Export pages below 1600 px.
- **Cards:** `rounded-card border border-line bg-card`. Lists inside cards are
  `<ul>`s divided by `line-2`.
- **Buttons:**
  - Primary: ink fill, white text, `h-8 rounded-ctl`, 12.5 px semibold.
  - Secondary: card fill with a `line` border.
  - Ghost: no border, `ink-2`, with a `card/70` hover (`GHOST` in `player/Transport.tsx`).
  - Destructive: `bg-rec`, only inside a confirm.
- **Chips:**
  - `rounded-chip`, mono, 11–12.5 px.
  - The canary chip is a selected length (`bg-canary text-canary-ink`).
  - Edge-time chips sit on the ribbon. The amber pill is a health headline.
- **Segmented controls:** a `sunk` trough with `p-0.5`, and the selected
  segment as a white card (`export/Controls.tsx`).
- **Confirm row** (`ConfirmRow` in `player/Transport.tsx`, and the Library inspector):
  - One quiet 40 px row in place of the transport, not a modal.
  - `rec-line` border, a trash icon, and **Delete 1h 43m?** followed by
    "Frees 6.2 GB now. Undo for 10 seconds.".
  - Cancel (`Esc`), then Delete in red (`↵`).
  - The range gets a red outline with a light wash. No hatching, no takeover.
- **Toast** (`player/Toast.tsx`):
  - An ink pill at the bottom centre: "Deleted 1h 43m · freed 6.2 GB · Undo",
    with a canary countdown bar.
  - Ctrl Z also undoes. A plain-message variant clears itself; errors show in soft red text.
- **The dark mat:** `bg-mat`, appearing only behind footage. That means the
  Player stage, the export preview, and the thumbnails in the Library inspector,
  the Marks rows and the Exports rows. Everything around it stays Paper.
- **Ribbon:**
  - One calm day strip: camera thumbnails with hour labels, away veils, and
    landmarks (marks, screens stopped) underneath.
  - An ink zoom bracket, and the playhead (an ink line with a canary knob).
  - The zoom strip below it shows the bracketed region.
  - No per-track lanes or waveforms; that detail lives in the Tracks popover.
- **Log card:** a short line per event, with a mono time. Clicking a line seeks.
  The current line gets a canary wash, and mark lines get a canary-deep square.
- **Inline notes** (`app/NoteField.tsx`): click the text or "Add a note", type,
  then Enter to save or Esc to cancel.
- **Health callouts:** an `amber-wash` box with an `amber-line` border, plain-words text and one action ("Restart screens").

## Don'ts

- No eyebrows, no periods at the end of headings, no title case.
- No canary fills on large areas, and no red or amber outside their meanings.
- No dark panels outside the mat, and no separate "editor" theme.
- No NLE track lanes, hatching or waveform stacks on screen.
- No modals for delete. Confirm in place and undo with the toast.
- No numbers in proportional figures.
