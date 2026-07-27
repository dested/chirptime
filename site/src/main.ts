import "./styles.css";

const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
const clamp01 = (n: number) => Math.min(1, Math.max(0, n));

/* ─────────────────────────────────────────────────────────────── player ──
   The day already exists in the page as markup. This makes it move: the
   playhead runs, the clock keeps up, each track lights up while it has
   footage, the camera goes dark where it was off, and the light in the
   picture warms toward evening. Drag it and the whole day follows. */

interface Run {
  from: number;
  to: number;
}

function readRuns(lane: Element): Run[] {
  return [...lane.querySelectorAll<HTMLElement>(".run")].map((run) => {
    const from = parseFloat(run.style.left) / 100;
    const to = from + parseFloat(run.style.width) / 100;
    return { from, to };
  });
}

function startPlayer(): void {
  const player = document.getElementById("player");
  if (!player) return;

  const scrub = player.querySelector<HTMLElement>(".tl__scrub");
  const clockEl = player.querySelector<HTMLElement>("[data-role=clock]");
  const cam = player.querySelector<HTMLElement>("[data-role=cam]");
  const tl = player.querySelector<HTMLElement>(".tl");
  if (!scrub || !tl) return;

  const dayStart = Number(player.dataset.start ?? 0);
  const daySeconds = Number(player.dataset.seconds ?? 1);

  const lanes = [...player.querySelectorAll<HTMLElement>(".tl__lane")].map((lane, i) => ({
    runs: readRuns(lane),
    name: player.querySelectorAll<HTMLElement>(".tl__name")[i] ?? null,
    live: false,
  }));
  const cameraRuns = lanes[2]?.runs ?? [];

  const label = (t: number): string => {
    const s = Math.floor(dayStart + t * daySeconds) % 86400;
    const h24 = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const h = h24 % 12 === 0 ? 12 : h24 % 12;
    return `${h}:${String(m).padStart(2, "0")} ${h24 < 12 ? "AM" : "PM"}`;
  };

  const covered = (runs: Run[], t: number) => runs.some((r) => t >= r.from && t <= r.to);

  let at = Number(player.dataset.at ?? 0.44);

  function render(): void {
    player!.style.setProperty("--t", at.toFixed(5));

    const text = label(at);
    if (clockEl && clockEl.textContent !== text) clockEl.textContent = text;
    scrub!.setAttribute("aria-valuenow", String(Math.round(at * 100)));
    scrub!.setAttribute("aria-valuetext", text);

    for (const lane of lanes) {
      const live = covered(lane.runs, at);
      if (live === lane.live) continue;
      lane.live = live;
      lane.name?.classList.toggle("is-live", live);
    }
    cam?.classList.toggle("is-off", cameraRuns.length > 0 && !covered(cameraRuns, at));
  }

  /* ── playing ── */

  const SWEEP_MS = 150_000; // a whole day, gently, at 1×
  let speed = 1;
  let raf = 0;
  let last = 0;
  let held = false;
  let visible = false;

  function frame(now: number): void {
    const dt = last ? now - last : 0;
    last = now;
    at = (at + (dt / SWEEP_MS) * speed) % 1;
    render();
    raf = requestAnimationFrame(frame);
  }

  function play(): void {
    if (raf || held || !visible || reduceMotion.matches) return;
    last = 0;
    raf = requestAnimationFrame(frame);
  }

  function pause(): void {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  }

  /* ── scrubbing ── */

  function seekTo(clientX: number): void {
    const box = scrub!.getBoundingClientRect();
    at = clamp01((clientX - box.left) / box.width);
    render();
  }

  scrub.addEventListener("pointerdown", (e) => {
    held = true;
    pause();
    tl!.classList.add("is-scrubbing");
    scrub!.setPointerCapture(e.pointerId);
    seekTo(e.clientX);
    e.preventDefault();
  });
  scrub.addEventListener("pointermove", (e) => {
    if (held) seekTo(e.clientX);
  });
  const release = (e: PointerEvent) => {
    if (!held) return;
    held = false;
    tl.classList.remove("is-scrubbing");
    if (scrub.hasPointerCapture(e.pointerId)) scrub.releasePointerCapture(e.pointerId);
    play();
  };
  scrub.addEventListener("pointerup", release);
  scrub.addEventListener("pointercancel", release);

  scrub.addEventListener("keydown", (e) => {
    const step =
      e.key === "ArrowLeft" ? -0.01 : e.key === "ArrowRight" ? 0.01 : e.key === "PageDown" ? -0.06 : e.key === "PageUp" ? 0.06 : 0;
    if (step) {
      at = clamp01(at + step);
    } else if (e.key === "Home") {
      at = 0;
    } else if (e.key === "End") {
      at = 1;
    } else {
      return;
    }
    e.preventDefault();
    pause();
    render();
  });

  for (const button of player.querySelectorAll<HTMLButtonElement>(".speed")) {
    button.addEventListener("click", () => {
      speed = Number(button.dataset.speed ?? 1);
      for (const other of player.querySelectorAll<HTMLButtonElement>(".speed")) {
        const on = other === button;
        other.classList.toggle("is-on", on);
        if (on) other.setAttribute("aria-pressed", "true");
        else other.removeAttribute("aria-pressed");
      }
      play();
    });
  }

  // Nothing moves while the player is off screen.
  new IntersectionObserver(
    ([entry]) => {
      visible = Boolean(entry?.isIntersecting);
      if (visible) play();
      else pause();
    },
    { threshold: 0.15 },
  ).observe(player);

  reduceMotion.addEventListener("change", () => (reduceMotion.matches ? pause() : play()));
  render();
}

/* ─────────────────────────────────────────────────────── scroll reveal ──
   Opt-in, and only once the script runs — the `js` class is what makes the
   hidden starting state apply, so the page stays fully readable if this file
   never loads. */

function startReveals(): void {
  const targets = [...document.querySelectorAll<HTMLElement>("[data-reveal]")];
  if (!targets.length) return;

  const show = (el: HTMLElement) => {
    el.style.transitionDelay = `${Number(el.dataset.reveal ?? 0) * 85}ms`;
    el.classList.add("in");
  };

  if (reduceMotion.matches || typeof IntersectionObserver !== "function") {
    targets.forEach((el) => el.classList.add("in"));
    return;
  }

  // Anything already on screen is shown straight away rather than waiting for
  // the observer — the top of the page must never depend on it firing.
  const pending: HTMLElement[] = [];
  for (const el of targets) {
    const box = el.getBoundingClientRect();
    if (box.top < window.innerHeight && box.bottom > 0) show(el);
    else pending.push(el);
  }
  if (!pending.length) return;

  const io = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        show(entry.target as HTMLElement);
        io.unobserve(entry.target);
      }
    },
    { rootMargin: "0px 0px -8% 0px", threshold: 0 },
  );

  pending.forEach((el) => io.observe(el));
}

/** The top bar only draws its rule once the page has moved under it. */
function startTopbar(): void {
  const bar = document.querySelector<HTMLElement>(".topbar");
  if (!bar) return;
  const onScroll = () => bar.classList.toggle("is-stuck", window.scrollY > 12);
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();
}

document.documentElement.classList.add("js");
startPlayer();
startReveals();
startTopbar();
