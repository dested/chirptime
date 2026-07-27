# ChirpTime site — copy & design brief

**Status: active** · 2026-07-27

The first attempt shipped a systems README with a near-black palette. The user's
response: *"the website is like dark as fuck haha. stop talking about the
implementation. this is a consumer product fool! it has to be like, record your
day, record your memories, play them back, catalog, organize... fucks all this
ram talk hahaha"*

So: **every content and palette decision is made below. Execute it, don't
re-litigate it.** Design freedom lives in layout, rhythm, motion and craft — not
in what the page says or what colour it is.

## Absolute rules

- **No implementation talk. Zero.** Banned words anywhere on the page: NVENC,
  ddagrab, AV1, H.264, codec, GPU, RAM, zero-copy, drift, millisecond, segment,
  encoder, FFmpeg (except once in Requirements), benchmark, throughput, sync
  engine, task, thread, buffer. If a sentence only lands for an engineer, cut it.
- **No numbers describing internals.** No "13% of one core", no "61.6ms", no
  "191 segments". The only permitted numbers are 5–8 GB/hour in Requirements.
- **Light and warm.** Not dark. Not `color-scheme: dark`.
- **No invented social proof** — no testimonials, no logos, no star counts, no
  "trusted by". Warm is not the same as dishonest.
- **No pricing section.** It isn't sold.

## Palette (use these exact values)

```
--paper:      #FAF7F2   /* page background — warm off-white */
--paper-2:    #F2EBE1   /* alternating section / cards */
--paper-3:    #E8DED1   /* borders, rules, quiet fills */
--ink:        #1A1714   /* primary text — warm near-black, never pure #000 */
--ink-2:      #57504A   /* secondary text */
--ink-3:      #8A8078   /* captions, meta */
--accent:     #C2643B   /* terracotta — primary accent, CTAs, emphasis */
--accent-dim: #E0A183   /* accent at rest / hover washes */
--sky:        #0EA5E9   /* the app's own accent. Use ONLY inside depictions of
                           the product UI, so screenshots/mocks feel authentic.
                           Never for site chrome. */
color-scheme: light;
```

Warm paper, deep warm ink, one confident terracotta accent. The feeling is a
personal archive, family photographs, daylight — not a security console.

## Fonts (already downloaded and self-hosted in `site/public/fonts`)

- **Instrument Serif** — display/headlines. Big, warm, editorial. This carries the page.
- **Archivo** — body.
- **IBM Plex Mono** — only for tiny meta (dates, timestamps in the timeline visual).

## Page structure and exact copy

### Hero
- H1: **Most of your days just disappear.**
- Sub: *ChirpTime keeps them. It records quietly in the background — every
  screen, your camera, your voice — so months from now you can sit down and
  watch an ordinary Tuesday.*
- Primary CTA: **Get it on GitHub** · Secondary: **See how it works** (scrolls)

### 2 — It runs and you forget it
- H2: **Start it in the morning. Forget it's there.**
- Body: *ChirpTime sits in a small window in the corner and records all day. No
  clips to manage, no folders to name, nothing to remember to press. You go do
  your work. It keeps up.*

### 3 — Everything together
- H2: **Every screen. Your camera. Your voice.**
- Body: *All of it captured at once and lined up. When you go back, your face,
  your words and whatever you were looking at are the same moment — not three
  recordings you have to reconcile.*

### 4 — Playback (**the visual centrepiece**)
- H2: **Scrub to any moment of any day.**
- Body: *Move through a whole day on one timeline. Watch every monitor at once,
  or focus on one with your camera in the corner. Slow it down, speed it up,
  jump straight to the part you were looking for.*
- This is where the multi-track timeline lives — as a thing you move through,
  warm and inviting, NOT a telemetry readout. Build the page around this idea.

### 5 — Catalog
- H2: **Your days, kept.**
- Body: *Every recording is filed by date and ready to browse. Mark the moments
  worth keeping as they happen — the call that went well, the bug that finally
  broke, your kid wandering in — and find them again later.*

### 6 — Honest requirements (small, near the CTA — not a wall)
- H2: **Where it's at**
- Body: *ChirpTime is open source and early. There's no installer yet, so you'll
  need to build it yourself. It runs on Windows, uses FFmpeg, and works best
  with an NVIDIA graphics card. Recordings run about 5–8 GB an hour, and nothing
  is ever deleted unless you delete it.*
- CTA: **Get it on GitHub**

### Footer
Minimal. Name, one line, GitHub link.

## Technical constraints (unchanged)

- Static Vite build in `site/`, output `dist/`. `bun run build` must succeed.
- No UI kit, no icon package — hand-written SVG.
- Responsive; never scrolls horizontally on mobile.
- Accessible: semantic HTML, real contrast, visible focus states, alt text.
- Respect `prefers-reduced-motion`. Motion must be purposeful.
- Self-hosted fonts, no external runtime calls.
- Repo URL in ONE constant. Assume `https://github.com/dested/chirptime`.

## Definition of done

Screenshots at desktop and mobile, looked at and iterated on. If it reads like a
developer tool, it has failed regardless of how clean the code is.
