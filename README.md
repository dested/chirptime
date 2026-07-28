# ChirpTime

ChirpTime is a background life recorder for Windows. Leave it running and it
quietly records your screens, your camera, your microphone and whatever your
computer is playing — all day, all at once — so that later you can scrub back to
three o'clock and watch what you were actually doing.

It also has a second, lighter mode: every few minutes it chirps, takes a
screenshot and grabs a short clip. That one is for skimming a day rather than
reliving it.

Everything lands in ordinary folders full of ordinary `.mp4` and `.png` files.
Nothing is uploaded anywhere, and nothing is ever deleted for you.

> **This is early.** It has been used by one person, on one machine. Expect rough
> edges, and don't rely on it for anything you can't afford to lose.

## Download

Grab the latest Windows installer from the
[releases page](https://github.com/dested/chirptime/releases/latest):

- **`ChirpTime_1.0.0_x64-setup.exe`** — the usual choice.
- **`ChirpTime_1.0.0_x64.msi`** — same app, MSI installer.
- **`ChirpTime_1.0.0_x64-portable.zip`** — just the executable, no installer.

### About that SmartScreen warning

The installers are **not code-signed**. There's no certificate behind this
project, so Windows will show *"Windows protected your PC"* the first time you
run one — a blue box with a **Don't run** button and no obvious way forward.
Click **More info**, then **Run anyway**.

That warning means "Windows doesn't recognise who published this", not "Windows
found something wrong with this file". It's the expected experience for every
unsigned installer, and it isn't going away until someone buys a signing
certificate. Said plainly here so you can decide for yourself rather than being
surprised by it.

## What you need

**Windows 10 or 11.** ChirpTime is Windows-only on purpose — the capture path is
built on Windows APIs (gdigrab/ddagrab, DirectShow, WASAPI, DXGI) and there is no
Mac or Linux build.

**FFmpeg.** ChirpTime records by driving FFmpeg, so without it nothing captures
at all — but you don't have to go and get it yourself. On first run, if FFmpeg
isn't already on your machine, ChirpTime offers to fetch it: it tells you the
size (about 110 MB) up front and waits for you to press the button. Nothing is
downloaded unless you say so.

What it does when you do say so: downloads a known, pinned
[gyan.dev](https://www.gyan.dev/ffmpeg/builds/) build over HTTPS, checks it
against a SHA-256 hash compiled into the app, and only then unpacks
`ffmpeg.exe` and `ffprobe.exe` into your own app-data folder
(`%APPDATA%\com.chirptime.app\ffmpeg\bin`). If the hash doesn't match, the file
is deleted and nothing runs. No administrator rights are needed, and nothing is
put on your `PATH` or anywhere else on the system.

If you already have FFmpeg, ChirpTime uses it and downloads nothing. It looks,
in order:

1. a folder you picked yourself (the **I Already Have It** button)
2. `ffmpeg` on your `PATH`
3. `C:\ffmpeg\bin`
4. `C:\Program Files\ffmpeg\bin`
5. its own downloaded copy under `%APPDATA%`

If the download is blocked — some corporate networks will — install FFmpeg
however you like and point ChirpTime at the folder holding `ffmpeg.exe` and
`ffprobe.exe`. Both are needed: the recorder uses `ffprobe` to index each clip
as it's written.

**An NVIDIA GPU, ideally.** Not required. With one, encoding takes the NVENC fast
path; without one, recording still works and simply costs more CPU. The widget
tells you which you have.

## Space

A continuous session records every monitor plus the webcam plus two audio
streams simultaneously, which works out to roughly **5–8 GB per hour**. A full
working day is a meaningful fraction of a drive.

**ChirpTime never deletes a recording.** There is no retention policy, no
rolling window, no cleanup on quit. The folder grows until you clear it out
yourself. That's deliberate — a tool that silently throws away yesterday isn't
much of a life recorder — but it does mean the disk is your responsibility.

By default everything goes to `%USERPROFILE%\Videos\ChirpTime`. The first-run
panel shows you the resolved path and the free space on that drive before you
record anything, and you can point it somewhere else right there or later from
the widget.

## Using it

The app is a small always-on-top widget, about the size of a Loom recorder. It
has two tabs:

- **Clips** — the interval capture. Screenshot plus a short screen-and-webcam
  clip every N minutes, announced by a two-tone chirp.
- **Session** — continuous recording. Start it and every screen, the webcam, the
  microphone and your system audio each record as their own track until you
  stop.

A separate viewer window opens on demand: a calendar, then a day, then either a
clip or a full session played back with every track in sync on one timeline.

## Building from source

You'll need [Bun](https://bun.sh), a
[Rust toolchain](https://rustup.rs), and FFmpeg as above.

```sh
bun install
bun run dev      # Tauri dev build, with the Vite dev server
bun run build    # release build → installers under src-tauri/target/release/bundle/
```

Useful while working:

```sh
bunx tsc --noEmit                 # type-check the frontend
cd src-tauri && cargo check
cd src-tauri && cargo test --lib  # NOT plain `cargo test` — see below
```

Don't run bare `cargo test`. `src-tauri/tests/live_session.rs` starts a real
recording against real hardware, which is not what you want from a test run.

There is more orientation in `cliffnotes.md` (a map of the codebase) and
`decisions.md` (why things are the way they are) at the repo root.

## Stack

Tauri v2 shell, React 19 + Tailwind v4 frontend, Rust backend that shells out to
FFmpeg for everything capture-related. Media is served to the UI over a small
local HTTP server rather than the asset protocol, because video scrubbing needs
byte-range requests.

## License

ChirpTime is [MIT licensed](LICENSE).

FFmpeg is a separate program under its own license — ChirpTime runs it as a
subprocess and does not link against it or ship its binaries. The build
ChirpTime offers to download for you is GPL, and that license covers that
download, not ChirpTime itself. That separation is deliberate: it's exactly why
FFmpeg is fetched at your request rather than bundled into the installer.
