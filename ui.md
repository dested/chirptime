# ChirpTime — UI / Visual Language

> The source of truth for how ChirpTime **looks and feels**. Follow it for
> anything visual. Keep it current as part of the definition of done for any UI
> change — same discipline as the cliffnotes.

## North star

**A calm, precise little instrument.** The widget should feel like a native macOS
menubar app that wandered onto Windows: white, hairline-bordered, rounded, with
one confident colored button. The viewer should feel like Photos/Loom — light and
airy while you browse, then drop to near-black the moment media is on screen so
the footage is the only bright thing.

Failure looks like: a purple gradient dashboard (too toy), or an undifferentiated
grey admin panel (too sterile). Neither. Reference points: Loom's recorder pill,
Linear's surfaces, Apple Photos' day grid.

1. **Two themes, one system.** Light slate everywhere; dark only inside the media
   viewer. Same radii, same accent, same type scale across both.
2. **One accent.** Sky (`#0EA5E9`) marks the live/selected/interactive thing.
   Red is reserved for recording and destruction. Purple only ever means webcam.
3. **Hairlines, not shadows.** Structure comes from 1px slate borders. Shadows are
   for lift on hover and for the one primary button.
4. **Numbers are monospaced.** Every time, count, and duration uses
   `tabular-nums font-mono` so nothing jitters while it ticks.
5. **Chrome recedes.** Toolbars are 1–2px of border and muted text; content is the
   only thing at full contrast.

## Tokens

Tailwind v4 utilities, no config file — these are literal Tailwind class names.
Anything not expressible as a utility lives in `src/ui/app.css`.

### Color — light surfaces (widget, calendar, gallery)

| Token | Class / value | Use |
| --- | --- | --- |
| Canvas | `bg-slate-50` | Viewer calendar/gallery page background |
| Surface | `bg-white` | Widget body, cards, buttons, toolbars |
| Hairline | `border-slate-200` / `border-slate-100` | 200 for interactive edges, 100 for dividers |
| Text — primary | `text-slate-900` | Titles, values, active labels |
| Text — secondary | `text-slate-600` | Secondary button labels |
| Text — muted | `text-slate-400` | Eyebrows, meta, timestamps, empty states |
| Text — faintest | `text-slate-300` | Disabled day numbers, idle icons |
| Accent | `sky-500` (`#0EA5E9`) | Today pill, selected state, hover borders, capture counts, primary CTA |
| Recording | `red-600` + `bg-red-50` | Recording status chip, stop button |
| Destructive | `red-500` + `bg-red-50` | Delete hover only |
| Webcam | `purple-500` / `bg-purple-50` | The "Cam" badge and webcam zoom only |
| Video badge | `text-blue-600` / `bg-blue-50` | "Video" badge in the gallery |

### Color — dark surfaces (media viewer only)

| Token | Value | Use |
| --- | --- | --- |
| Canvas | `bg-[#0A0A0F]` | Media viewer background |
| Surface | `bg-white/[0.03]` | Toolbar, filmstrip rail |
| Hairline | `border-white/[0.06]` | Toolbar/filmstrip edges |
| Control | `bg-white/[0.06]` + `border-white/10` | Buttons; hover → `bg-white/10` |
| Segmented track | `bg-white/[0.04]` + `border-white/[0.06]` | Zoom + speed pill groups |
| Text — primary | `text-white` | Current timestamp |
| Text — muted | `text-white/40` | Counts, inactive segment labels |
| Selected segment | `text-sky-500 bg-sky-500/15` | Active monitor / active mode |
| Thumb placeholder | `#1E1E2A` | Filmstrip cells with no thumbnail |

**Accent discipline:** sky is earned by *state* (this is selected / this is live /
this day has captures), never used as decoration. Red is only used when recording
is in progress or something is about to be deleted. Purple never appears outside a
webcam affordance.

### Typography

System stack only — `-apple-system, 'Segoe UI', system-ui, sans-serif`, set once
on `body` in `app.css` with `-webkit-font-smoothing: antialiased`.

| Role | Classes | Use |
| --- | --- | --- |
| Page title | `text-2xl font-bold tracking-tight` | Calendar month |
| Section title | `text-xl font-bold tracking-tight` | Gallery date heading |
| Brand | `text-lg font-bold tracking-tight` (viewer) / `text-[13px] font-bold` (widget) | Wordmark next to the logo |
| Body / control | `text-[13px] font-medium` | Buttons, list rows |
| Primary button | `text-sm font-semibold tracking-tight` | Start/Stop recording |
| Stat value | `text-[15px] font-bold tabular-nums` | Widget stats |
| Eyebrow | `text-[11px] font-semibold uppercase tracking-wide text-slate-400` | "Recent", "Recent Days", stat labels |
| Meta | `text-[10px]`–`text-xs` | Badges, counts |
| Time | `tabular-nums font-mono` | **Always**, on every clock value |

Weights in use: 500 / 600 / 700 only. No light weights, no italics.

### Spacing, shape, elevation

| Token | Value | Use |
| --- | --- | --- |
| Widget gutter | `px-4` | Every widget section |
| Viewer gutter | `px-8`, calendar capped at `max-w-[960px] mx-auto` | Page shell |
| Radius — window | `rounded-xl` (12px) | Widget root + primary button |
| Radius — card | `rounded-[14px]` | Gallery thumbnail cards |
| Radius — control | `rounded-[10px]` | Buttons, status chip, calendar chips |
| Radius — small | `rounded-lg` / `rounded-md` | Segmented groups; segments and filmstrip cells |
| Radius — pill | `rounded-full` | Today pill, status dot, viewer nav arrows |
| Elevation — rest | `border` + `shadow-sm` | Calendar day cells with captures |
| Elevation — hover | `hover:-translate-y-px` + colored shadow, e.g. `hover:shadow-[0_4px_16px_rgba(14,165,233,0.12)]` | Cards and day cells |
| Elevation — primary | `shadow-[0_1px_3px_rgba(14,165,233,0.3),0_4px_12px_rgba(14,165,233,0.15)]` | Start button (red equivalent for Stop) |
| Transition | `transition-all` | Every interactive element; no explicit durations |

## Layout

- **Widget (320×540, undecorated):** `flex flex-col h-screen bg-white rounded-xl
  border border-slate-200` → custom titlebar (drag region) → status chip →
  primary + secondary buttons → stats strip (top+bottom hairline) → scrollable
  Recent list (`flex-1 min-h-0`) → Browse footer.
- **Viewer (1280×800):** one `h-screen overflow-hidden` root, exactly one of three
  views mounted at a time — `calendar` → `gallery` → `viewer`. Back always steps one
  level up; `Escape` does the same.
- **Calendar:** month header row (prev / month+year / next / Today) → 7-col grid,
  cells `aspect-[1.4]`, always 42 cells → "Recent Days" chips below a hairline.
- **Gallery:** white toolbar (Back + date + count) → `grid
  grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-4`, cards are `aspect-video`
  thumb + meta row.
- **Media viewer:** toolbar → centered media (`flex-1 min-h-0`, floating circular
  prev/next) → 72×42 filmstrip rail.

## Components

Everything lives inline in the two app files — there is no component library, and
the files under `src/ui/components/` are **dead code** (see cliffnotes). Copy the
patterns below rather than importing from there.

| Pattern | File | Notes |
| --- | --- | --- |
| Custom titlebar + drag | `CompactApp.tsx` | `onMouseDown` → `appWindow.startDragging()`, ignores clicks landing on a `button` |
| Status chip + pulsing dot | `CompactApp.tsx` | `.dot-active` keyframes live in `app.css` |
| Primary action button | `CompactApp.tsx` | sky when idle, red when recording |
| Calendar grid | `ViewerApp.tsx` | days without captures render as inert transparent cells |
| Thumbnail card | `ViewerApp.tsx` | `loading="lazy"`, play-glyph overlay chip when a video exists |
| Segmented control | `ViewerApp.tsx` | used for monitor zoom and playback speed |
| Filmstrip | `ViewerApp.tsx` | active cell `border-sky-500`, inactive `opacity-50` |
| Icons | both | Inline SVG, `viewBox="0 0 24 24"`, `stroke="currentColor"`, `strokeWidth={2}`, `strokeLinecap="round"`. No icon package. |

Signature snippets:

```tsx
{/* Secondary button — the default control everywhere on light surfaces */}
<button className="flex items-center justify-center gap-2 py-2.5 rounded-[10px] text-[13px] font-medium cursor-pointer border border-slate-200 bg-white text-slate-600 hover:bg-slate-50 hover:border-slate-300 hover:text-slate-900 transition-all disabled:opacity-50 disabled:cursor-not-allowed">

{/* Segmented control — dark viewer toolbar */}
<div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
  <button className={active
    ? "px-3 py-1 rounded-md text-xs font-semibold text-sky-500 bg-sky-500/15"
    : "px-3 py-1 rounded-md text-xs font-semibold text-white/40 hover:text-white/70 hover:bg-white/[0.06]"} />
</div>

{/* Eyebrow */}
<div className="text-[11px] font-semibold text-slate-400 uppercase tracking-wide">Recent</div>
```

The ChirpTime mark is an inline SVG (sky circle at 15% opacity, a dot, and an arc)
duplicated at 18px in the widget titlebar and 24px in the viewer header — keep the
two copies identical.

## States

- **Loading:** centered `text-slate-400 text-sm` — "Loading...". No spinners.
- **Empty:** centered column, 48px `#CBD5E1` outline icon + one muted sentence
  ("No captures for this day"). No illustrations, no CTA.
- **Busy:** buttons take `disabled` with `disabled:opacity-50
  disabled:cursor-not-allowed`; never swap the label for a spinner.
- **Errors:** currently swallowed (`catch {}` everywhere). If you add an error
  surface, make it an inline muted line — not a toast, not a modal.
- **Destructive:** native `confirm()` naming the capture's `display_time`. Delete
  affordances stay invisible until row hover (`opacity-0 group-hover:opacity-100`).

## Voice / copy

Short, literal, sentence case. Verb-first for actions.

- Good: "Start Recording", "Capture Now", "Browse All Captures", "Recording",
  "Idle", "Next capture in", "No captures for this day".
- Bad: "🎬 Let's capture your day!", "Oops! Something went wrong", "Awesome!".
- Eyebrow labels are the only uppercase text. No exclamation marks, no emoji in UI
  copy, no ellipses except the loading string.
- Badges are one word: `Video`, `Cam`, `Photo`.

## Don'ts

- ❌ Dark purple / gradient "dark mode" — the only dark surface is `#0A0A0F` behind
  media, and it is neutral.
- ❌ Gradients, glassmorphism, or decorative blur. The only `backdrop-blur` allowed
  is on the viewer's floating nav arrows and the play chip.
- ❌ A second accent color. Sky, plus red for recording/danger and purple for
  webcam. That's the whole palette.
- ❌ Proportional digits on anything that ticks — always `tabular-nums font-mono`.
- ❌ Shadows for structure on light surfaces. Use `border-slate-200`.
- ❌ Toasts, modals, or drawers. The widget is 320px wide; everything is inline.
- ❌ Icon libraries or CSS frameworks beyond Tailwind. Icons are hand-written SVG.
- ❌ Adding a file to `src/ui/components/` — that directory is dead; the two app
  files are the app.
- ❌ Scrollbars wider than 4px, or a light scrollbar thumb on a dark surface (use
  the `.dark-scroll` class).
- ❌ Sharp corners. Nothing in this app has a 0px radius.
