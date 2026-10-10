# Questions for Sal

Status: active (2026-10-10, designer, bosun UX sprint)

These are product questions only Sal can answer. Each one leads with my
recommended answer.

## Purpose

1. **What is the footage for?**
   - **Recommended:** a flight recorder for your working day that feeds the devlog/film pipeline. Most of the time you go back to cut dead time; sometimes you pull a piece out to show.
   - If you mainly use it for memory or "what did I do Tuesday", search (OCR, transcript) moves above export.
   - If it's mainly for content, the export presets move up and the editor needs titles and captions.
2. **"It doesn't look like anything else": inconsistent, or bland?**
   - **Recommended: both.** One system across every window, and a player built around your real desk (Direction A).
   - If you meant only "make it consistent", Direction B is too far.
3. **Kill Clips mode (interval screenshot + clip) entirely?**
   - **Recommended:** yes, including the calendar, gallery and media viewer. There have been 10 captures since sessions shipped and none since August.
   - The 21 legacy days in `G:\code\chirptime\output` get a one-time read-only import into the Library, or are left alone. Which?

## Editing

4. **Delete: recoverable for 7 days, or gone at once?**
   - **Recommended:** a non-destructive cut list in the manifest, with Recently deleted for 7 days and then reclaim (lossless stream-copy trims, no re-encode).
   - The alternative is instant disk space with no undo.
5. **Do you need to cut single tracks?** For example, drop the camera because someone walked in but keep the screens, or mute the mic over a phone call.
   - **Recommended:** yes, via Tracks… in the range toolbar. It is cheap on top of the cut list.
6. **Should ChirpTime suggest cuts?** It would flag empty-chair, no-input stretches as dashed "Away 1h 20m" ranges, and Shift A selects them all.
   - **Recommended:** yes, suggestions only, never automatic.
   - Do you also want an "auto-cut away time older than N days" policy?
7. **How fine does a cut need to be?**
   - **Recommended:** a second at most in the UI, snapped to landmarks. Reclaim keeps up to 2 s of video at each edge (one GOP), and that stays masked.
   - If you need frame-accurate cuts for content, that belongs in export, not delete.
8. **Marks: hotkey only, or keep a typed note while recording?**
   - **Recommended:** a hotkey mark (Ctrl Alt M) and Mark button, with an optional note afterwards in the player.
   - You've never used the current note field (0 marks in 30 sessions). Is that because it needs focus, or because marks aren't useful to you?

## Export

9. **Which export layout matters first?**
   - **Recommended:** Devlog: main screen at 16:9 (following the cursor on the ultrawide) with the camera in a corner, mic with system audio ducked.
   - After that: Short (9:16), Desk (all screens as arranged), Side by side.
10. **Is a cursor-follow crop on the 5120 ultrawide worth recording the cursor track?**
    - **Recommended:** yes. It is cheap to store and makes the ultrawide usable in 16:9.
11. **Where do exports go, and is there anything after export?**
    - **Recommended:** `Videos\ChirpTime\exports`, then reveal in Explorer, with an Exports list in the app.
    - Do you also want a hand-off into your devlog tooling?

## Recording

12. **On Aug 1 your screens died at 2:02 PM and the recorder kept the camera and audio for 5 more hours without telling you. Did you know?**
    - **Recommended:** a toast plus an amber alert in the recorder, auto-restart with backoff, and a Restart screens button. Fixing the stall's root cause (display sleep or lock?) is a separate engineering task.
    - Would you rather it stop the whole session when the screens die?
13. **Exclude the recorder from capture?**
    - **Recommended:** yes, using window display affinity, so the widget never shows up in footage. Needs a spike to confirm ddagrab honours it.
14. **Privacy pause: wanted?**
    - **Recommended:** yes. A hotkey blanks the screens and camera for 5, 15 or 30 minutes, with a visible gap.
    - Should camera footage of other people get anything special, or is that your call per session?
15. **The chirp: keep it?**
    - **Recommended:** keep it as the brand and the consent cue, but make it a designed sound (not `Beep()`), play it on start, stop and mark, and allow turning it off.
16. **Recorder default: full widget, or the pill?**
    - **Recommended:** the full recorder until you press Start, then it auto-collapses to the pill after 10 s. Expand on hover.

## Look

17. **Player direction: A "Darkroom" or B "Logbook"?**
    - **Recommended:** A. Horizontal time matches the media, and it works before presence detection exists. Steal B's written log for A's inspector.
18. **Two surfaces (light Paper for recording and finding, warm-graphite Darkroom for footage), or one theme everywhere?**
    - **Recommended:** two. The footage needs a dark surround, and the recorder lives on your desktop all day where a light card is calmer.
    - A full dark mode for Paper would come later, if you want it.
19. **Canary yellow as the single accent?**
    - **Recommended:** yes. It's a highlighter, reserved for selection, playhead and marks; red stays for recording and delete only.
    - Push back if yellow fights your devlog brand.

## Storage

20. **Retention: is anything automatic?**
    - **Recommended:** nothing automatic yet. The Library offers to clean up sessions under 2 minutes, and Settings shows the per-track size so you can drop, say, screen 4 after 30 days.
    - You're at 174 GB after 10 weeks, about 900 GB a year at this rate.
