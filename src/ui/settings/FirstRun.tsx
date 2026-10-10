/**
 * First run, in the app window: FFmpeg, then what to record and where, then
 * the hotkeys. Shown by App.tsx while `settings.onboarded` is false; ends with
 * `set_onboarded(true)` and `onDone`.
 */
import { useEffect, useState, type ReactNode } from "react";
import { pickOutputDir, setOnboarded } from "../hooks/useApi";
import { DEFAULT_HOTKEYS, keyLabel } from "../recorder/api";
import { DeskMap } from "../recorder/DeskMap";
import { BluetoothNote, CameraSelect, MicSelect } from "../recorder/fields";
import { freeLine, screenOn, toggleScreen, useRecordingSetup } from "../recorder/setup";
import { CheckBox, CtMark, IconCheck, IconChevron } from "../recorder/ui";
import { fmtSize } from "../session/format";
import { FfmpegPanel } from "./Ffmpeg";

type Step = 1 | 2 | 3;

function Fact({ icon, lead, rest }: { icon: ReactNode; lead: string; rest: string }) {
  return (
    <div className="flex gap-3 text-[13px] text-ink-2">
      <span className="shrink-0 text-ink">{icon}</span>
      <span>
        <b className="font-semibold text-ink">{lead}</b> {rest}
      </span>
    </div>
  );
}

const factIcon = (d: ReactNode) => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
    {d}
  </svg>
);

function Num({ n, state }: { n: number; state: "done" | "current" | "later" }) {
  if (state === "done") {
    return (
      <span className="grid h-[22px] w-[22px] shrink-0 place-items-center rounded-full bg-ink">
        <IconCheck size={11} width={3.5} />
      </span>
    );
  }
  return (
    <span
      className={`grid h-[22px] w-[22px] shrink-0 place-items-center rounded-full font-mono text-[11px] font-semibold ${
        state === "current" ? "bg-canary text-canary-ink" : "border-[1.5px] border-ink-4 text-ink-3"
      }`}
    >
      {n}
    </span>
  );
}

/** A step that is not the current one: one row with its summary. */
function StepRow({ n, state, title, line, onOpen, action }: { n: number; state: "done" | "later"; title: string; line: string; onOpen?: () => void; action?: ReactNode }) {
  return (
    <div className="flex items-center gap-3 rounded-card border border-line bg-card px-[18px] py-3.5">
      <Num n={n} state={state} />
      <button className="min-w-0 text-left disabled:cursor-default" onClick={onOpen} disabled={!onOpen}>
        <div className={`text-[13.5px] font-semibold ${state === "later" ? "text-ink-2" : ""}`}>{title}</div>
        <div className="num mt-0.5 truncate text-[11px] text-ink-3">{line}</div>
      </button>
      {action && <div className="ml-auto shrink-0">{action}</div>}
    </div>
  );
}

function StepCard({ n, title, aside, children }: { n: number; title: string; aside?: string; children: ReactNode }) {
  return (
    <div className="rounded-card border border-line bg-card px-[18px] pb-[18px] pt-4">
      <div className="flex items-center gap-3">
        <Num n={n} state="current" />
        <div className="text-[14.5px] font-bold">{title}</div>
        {aside && <div className="ml-auto text-[12px] text-ink-3">{aside}</div>}
      </div>
      <div className="mt-4 pl-[34px]">{children}</div>
    </div>
  );
}

export function FirstRun({ onDone }: { onDone: () => void }) {
  const setup = useRecordingSetup();
  const { options, setOptions, screens, devices, env, ffmpeg, prefs, reloadEnv, reloadFfmpeg } = setup;
  const [step, setStep] = useState<Step | null>(null);
  const [error, setError] = useState<string | null>(null);
  const hasFfmpeg = ffmpeg?.ffmpeg_path != null;

  // Start where there is something to do: FFmpeg if it is missing.
  useEffect(() => {
    if (step === null && ffmpeg !== null) setStep(ffmpeg.ffmpeg_path ? 2 : 1);
  }, [ffmpeg, step]);

  const finish = () => {
    setOnboarded(true)
      .then(onDone)
      .catch((e) => setError(String(e)));
  };

  const current = step ?? 1;
  const free = freeLine(env, fmtSize);
  const keys = prefs?.hotkeys ?? DEFAULT_HOTKEYS;
  const picked = screens.filter((s) => screenOn(options, screens, s.index)).length;
  const what = [
    picked > 0 ? `${picked} ${picked === 1 ? "screen" : "screens"}` : "no screens",
    options.record_webcam ? "camera" : null,
    options.record_mic ? "mic" : null,
    options.record_system_audio ? "system audio" : null,
  ]
    .filter(Boolean)
    .join(", ");
  const canContinue = current === 1 ? hasFfmpeg : current === 2 ? picked > 0 || options.record_webcam || options.record_mic : true;

  return (
    <div className="flex h-full bg-paper">
      <div className="flex w-[470px] shrink-0 flex-col border-r border-line bg-rail px-12 py-14">
        <CtMark size={40} />
        <h1 className="mt-7 text-[34px] font-bold leading-[1.08] tracking-[-0.03em]">Record the whole day. Keep the parts that matter</h1>
        <p className="mt-[18px] text-[14.5px] leading-[1.55] text-ink-2" style={{ textWrap: "pretty" }}>
          ChirpTime records every screen, your camera and your sound in the background, in one-minute pieces. When you step away it stops keeping
          footage. Later you scrub the day, cut what you don't need and export what you do.
        </p>
        <div className="mt-7 flex flex-col gap-3">
          <Fact icon={factIcon(<><rect x="4" y="10" width="16" height="11" rx="2" /><path d="M8 10V7a4 4 0 018 0v3" /></>)} lead="Nothing leaves this PC." rest="No account, no upload." />
          <Fact icon={factIcon(<><path d="M3 12a9 9 0 1018 0 9 9 0 00-18 0" /><path d="M12 7v5l3 2" /></>)} lead="A chirp before it starts," rest="so you always know." />
          <Fact icon={factIcon(<path d="M4 7h16M4 12h10M4 17h6" />)} lead="About 6 GB an hour" rest="with four screens." />
        </div>
        <div className="flex-1" />
        <div className="num text-[11px] text-ink-3">ChirpTime 1.0 · Windows</div>
      </div>

      <div className="flex min-w-0 flex-1 flex-col gap-3 overflow-y-auto px-14 py-10">
        <div className="flex items-baseline">
          <div className="text-[20px] font-bold tracking-[-0.02em]">Set up in three steps</div>
          <div className="ml-auto text-[12.5px] text-ink-3">About a minute</div>
        </div>

        {current === 1 ? (
          <StepCard n={1} title="Install the video engine" aside="FFmpeg records and encodes every track">
            <FfmpegPanel setup={ffmpeg} onChanged={reloadFfmpeg} />
          </StepCard>
        ) : (
          <StepRow
            n={1}
            state="done"
            title="Video engine installed"
            line={`FFmpeg · ${ffmpeg?.ffmpeg_path ?? ""}`}
            onOpen={() => setStep(1)}
            action={
              <button className="text-[12px] font-semibold text-ink-3 hover:text-ink" onClick={() => setStep(1)}>
                Use my own
              </button>
            }
          />
        )}

        {current === 2 ? (
          <StepCard n={2} title="Choose what to record" aside="Click a screen to leave it out">
            <div className="flex justify-center">
              <DeskMap
                screens={screens}
                width={520}
                maxHeight={230}
                look={(i) => ({ kind: "pick", on: screenOn(options, screens, i) })}
                camera={{ kind: "pick", on: options.record_webcam }}
                onPick={(i) => setOptions(toggleScreen(options, screens, i))}
                onCamera={() => setOptions({ ...options, record_webcam: !options.record_webcam })}
                detail
              />
            </div>
            <div className="mt-[18px] grid grid-cols-2 gap-x-4 gap-y-2.5">
              <div>
                <div className="mb-[5px] text-[12px] text-ink-3">Camera</div>
                <CameraSelect options={options} setOptions={setOptions} devices={devices} />
              </div>
              <div>
                <div className="mb-[5px] text-[12px] text-ink-3">Microphone</div>
                <MicSelect options={options} setOptions={setOptions} devices={devices} />
                <BluetoothNote options={options} />
              </div>
              <button
                className="col-span-2 flex items-center gap-2 text-left text-[12.5px] text-ink-2"
                onClick={() => setOptions({ ...options, record_system_audio: !options.record_system_audio })}
                aria-pressed={options.record_system_audio}
              >
                <CheckBox on={options.record_system_audio} size={16} />
                Also record what the PC plays (system audio)
              </button>
            </div>
            <div className="mt-4 flex items-center gap-3 border-t border-line-2 pt-3.5">
              <div className="min-w-0">
                <div className="text-[12px] text-ink-3">Recordings go to</div>
                <div className="num mt-0.5 truncate text-[11.5px]">
                  {env?.output_dir ?? ""}
                  {free ? ` · ${free}` : ""}
                </div>
              </div>
              <button
                className="ml-auto shrink-0 text-[12px] font-semibold text-ink-3 hover:text-ink"
                onClick={() => void pickOutputDir().then((p) => p && reloadEnv()).catch((e) => setError(String(e)))}
              >
                Change
              </button>
            </div>
          </StepCard>
        ) : (
          <StepRow
            n={2}
            state={current > 2 ? "done" : "later"}
            title={current > 2 ? "Recording set" : "Choose what to record"}
            line={current > 2 ? `${what} · ${env?.output_dir ?? ""}` : "Screens, camera, sound and where it goes"}
            onOpen={current > 2 ? () => setStep(2) : undefined}
          />
        )}

        {current === 3 ? (
          <StepCard n={3} title="Your hotkeys" aside="They work from any app">
            <div className="flex flex-col gap-2.5">
              {[
                { k: keys.record, what: "Starts and stops recording" },
                { k: keys.mark, what: "Marks this moment, so you can find it later" },
                { k: keys.pause, what: "Pauses every track until you resume" },
              ].map(({ k, what: w }) => (
                <div key={w} className="flex items-center gap-3 text-[13px]">
                  <span className="kbd min-w-[84px] justify-center">{k ? keyLabel(k) : "Off"}</span>
                  {w}
                </div>
              ))}
            </div>
            <div className="mt-4 text-[12px] leading-[1.5] text-ink-3">
              The recorder stays on top of your screens. Close it while recording and it keeps going in the tray. Change the hotkeys in Settings.
            </div>
          </StepCard>
        ) : (
          <StepRow n={3} state="later" title="Your hotkeys" line={`${keyLabel(keys.record)} to record · ${keyLabel(keys.mark)} to mark`} />
        )}

        <div className="flex-1" />
        {error && <div className="text-[12px] text-rec">{error}</div>}
        <div className="flex items-center gap-3">
          <span className="text-[12.5px] text-ink-3">You can change all of this later in Settings</span>
          <button className="ml-auto h-11 rounded-[11px] px-[18px] font-semibold text-ink-2 hover:text-ink" onClick={finish}>
            Set up later
          </button>
          <button
            className="flex h-11 items-center gap-2 rounded-[11px] bg-ink px-5 text-[14px] font-bold text-white transition-opacity disabled:opacity-50"
            disabled={!canContinue}
            onClick={() => (current === 3 ? finish() : setStep(current === 1 ? 2 : 3))}
          >
            {current === 3 ? "Start using ChirpTime" : "Continue"}
            <IconChevron />
          </button>
        </div>
      </div>
    </div>
  );
}
