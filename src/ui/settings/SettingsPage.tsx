/**
 * Settings (#/settings): Recording, Storage, Hotkeys, Privacy, FFmpeg, About.
 * Recording options are the same ones the widget edits and apply to the next
 * recording; everything else applies at once.
 */
import { useEffect, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openFolder, pickOutputDir } from "../hooks/useApi";
import { DEFAULT_HOTKEYS, readPrefs, setContentProtection, setDiscardShortSessions } from "../recorder/api";
import { DeskMap } from "../recorder/DeskMap";
import { BluetoothNote, CameraSelect, FpsSelect, IDLE_EXPLAINED, IdleSelect, MicSelect } from "../recorder/fields";
import { freeLine, screenOn, toggleScreen, useRecordingSetup } from "../recorder/setup";
import { Toggle } from "../recorder/ui";
import { fmtSize } from "../session/format";
import { btn, FfmpegPanel } from "./Ffmpeg";
import { HotkeysEditor } from "./Hotkeys";

function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section className="mt-6 rounded-card border border-line bg-card px-5 pb-5 pt-4">
      <div className="flex items-baseline gap-3">
        <h2 className="text-[15px] font-bold tracking-[-0.01em]">{title}</h2>
        {note && <span className="text-[12px] text-ink-3">{note}</span>}
      </div>
      <div className="mt-3">{children}</div>
    </section>
  );
}

function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-start gap-6 border-b border-line-2 py-2.5 last:border-b-0">
      <div className="w-[200px] shrink-0 pt-1.5">
        <div className="text-[13px] font-semibold">{label}</div>
        {hint && <div className="mt-0.5 text-[11.5px] leading-[1.4] text-ink-3">{hint}</div>}
      </div>
      <div className="min-w-0 flex-1 pt-0.5">{children}</div>
    </div>
  );
}

function SwitchRow({ label, hint, on, onChange }: { label: string; hint: string; on: boolean; onChange: (on: boolean) => void }) {
  return (
    <div className="flex items-center gap-6 py-1">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-semibold">{label}</div>
        <div className="mt-0.5 text-[11.5px] leading-[1.4] text-ink-3">{hint}</div>
      </div>
      <Toggle label={label} on={on} onChange={onChange} />
    </div>
  );
}

export function SettingsPage() {
  const setup = useRecordingSetup();
  const { options, setOptions, screens, devices, env, prefs, ffmpeg, reloadEnv, reloadFfmpeg, reloadSettings } = setup;
  const [version, setVersion] = useState<string | null>(null);
  const [protect, setProtect] = useState<boolean | null>(null);
  const [discard, setDiscard] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const field = { options, setOptions };

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(null));
  }, []);
  const settings = setup.settings;
  useEffect(() => {
    if (!settings) return;
    const p = readPrefs(settings);
    setProtect(p.content_protection);
    setDiscard(p.discard_short_sessions);
  }, [settings]);

  const free = freeLine(env, fmtSize);

  return (
    <div className="h-full overflow-y-auto px-8 pb-12 pt-6">
      <h1 className="text-[24px] font-bold tracking-[-0.02em]">Settings</h1>
      <div className="mt-1 text-[12px] text-ink-3">Saved as you change them</div>
      {(error ?? setup.error) && <div className="mt-3 text-[12px] text-rec">{error ?? setup.error}</div>}

      <div className="max-w-[760px]">
        <Section title="Recording" note="Applies to the next recording">
          <Field label="Screens" hint="Click a monitor to leave it out">
            <DeskMap
              screens={screens}
              width={440}
              maxHeight={170}
              look={(i) => ({ kind: "pick", on: screenOn(options, screens, i) })}
              camera={{ kind: "pick", on: options.record_webcam }}
              onPick={(i) => setOptions(toggleScreen(options, screens, i))}
              onCamera={() => setOptions({ ...options, record_webcam: !options.record_webcam })}
              detail
            />
          </Field>
          <Field label="Frame rate" hint="Higher is smoother and takes more disk">
            <FpsSelect {...field} className="max-w-[260px]" />
          </Field>
          <Field label="Camera">
            <CameraSelect {...field} devices={devices} className="max-w-[360px]" />
          </Field>
          <Field label="Microphone" hint="Automatic never picks a Bluetooth headset">
            <MicSelect {...field} devices={devices} className="max-w-[360px]" />
            <BluetoothNote options={options} />
          </Field>
          <Field label="System audio" hint="What the PC plays">
            <Toggle label="System audio" on={options.record_system_audio} onChange={(on) => setOptions({ ...options, record_system_audio: on })} />
          </Field>
          <Field label="When you step away" hint={IDLE_EXPLAINED}>
            <IdleSelect {...field} className="max-w-[260px]" />
          </Field>
        </Section>

        <Section title="Storage">
          <Field label="Recordings folder" hint={free ?? undefined}>
            <div className="num break-all text-[12px]">{env?.output_dir ?? ""}</div>
            <div className="mt-2 flex gap-2">
              <button className={btn} onClick={() => void pickOutputDir().then((p) => p && reloadEnv()).catch((e) => setError(String(e)))}>
                Change
              </button>
              <button className={btn} onClick={() => void openFolder().catch((e) => setError(String(e)))}>
                Open
              </button>
            </div>
            {env?.legacy_output && (
              <div className="mt-2 text-[11.5px] leading-[1.4] text-[#9A5A0C]">
                Older recordings are still in <span className="num break-all">{env.legacy_output.path}</span>. Nothing there has been moved.
              </div>
            )}
          </Field>
          <div className="pt-2">
            <SwitchRow
              label="Discard sessions under a minute"
              hint="A recording stopped within a minute of starting is deleted, with a few seconds to undo"
              on={discard ?? false}
              onChange={(on) => {
                setDiscard(on);
                setDiscardShortSessions(on).catch((e) => setError(String(e)));
              }}
            />
          </div>
        </Section>

        <Section title="Hotkeys">
          <HotkeysEditor hotkeys={prefs?.hotkeys ?? DEFAULT_HOTKEYS} onSaved={reloadSettings} />
        </Section>

        <Section title="Privacy">
          <SwitchRow
            label="Hide ChirpTime from screen capture"
            hint="The recorder and this window show as black in screen shares, screenshots and ChirpTime's own recordings"
            on={protect ?? true}
            onChange={(on) => {
              setProtect(on);
              setContentProtection(on).catch((e) => setError(String(e)));
            }}
          />
          <div className="mt-2 text-[11.5px] text-ink-3">Nothing ChirpTime records leaves this PC</div>
        </Section>

        <Section title="FFmpeg" note="Records and encodes every track">
          <FfmpegPanel setup={ffmpeg} onChanged={reloadFfmpeg} />
        </Section>

        <Section title="About">
          <div className="flex items-baseline gap-2 text-[13px]">
            <span className="font-semibold">ChirpTime</span>
            <span className="num text-ink-3">{version ?? ""}</span>
          </div>
          {env?.settings_path && (
            <div className="mt-1.5 text-[11.5px] text-ink-3">
              Settings file <span className="num break-all">{env.settings_path}</span>
            </div>
          )}
        </Section>
      </div>
    </div>
  );
}
