/** The widget's recording options: only what changes the next recording. */
import type { ReactNode } from "react";
import { openFolder, pickOutputDir } from "../hooks/useApi";
import { fmtSize } from "../session/format";
import { DeskMap } from "./DeskMap";
import { BluetoothNote, CameraSelect, FpsSelect, IDLE_EXPLAINED, IdleSelect, MicSelect } from "./fields";
import { freeLine, screenOn, toggleScreen, type RecordingSetup } from "./setup";
import { Toggle } from "./ui";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="mt-2 flex items-center gap-2.5">
      <span className="w-[78px] shrink-0 text-[12.5px] text-ink-2">{label}</span>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}

const H = ({ children, first }: { children: ReactNode; first?: boolean }) => (
  <div className={`text-[13px] font-bold tracking-[-0.005em] ${first ? "mt-1.5" : "mt-[18px]"}`}>{children}</div>
);

export function OptionsView({ setup, locked }: { setup: RecordingSetup; locked: boolean }) {
  const { options, screens, setOptions, devices, env, reloadEnv } = setup;
  const free = freeLine(env, fmtSize);
  const field = { options, setOptions };

  return (
    <div className="-mx-4 min-h-0 flex-1 overflow-y-auto px-4 pb-3">
      {locked && (
        <div className="mt-1.5 rounded-ctl bg-sunk px-2.5 py-2 text-[11.5px] leading-[1.4] text-ink-2">
          A session is recording. These apply from the next one.
        </div>
      )}
      <H first>Screens</H>
      <div className="mt-0.5 text-[11.5px] text-ink-3">Click a monitor to leave it out</div>
      <div className="mt-2.5 flex justify-center">
        <DeskMap
          screens={screens}
          width={262}
          maxHeight={120}
          look={(i) => ({ kind: "pick", on: screenOn(options, screens, i) })}
          camera={{ kind: "pick", on: options.record_webcam }}
          onPick={(i) => setOptions(toggleScreen(options, screens, i))}
          onCamera={() => setOptions({ ...options, record_webcam: !options.record_webcam })}
        />
      </div>
      <Row label="Frame rate">
        <FpsSelect {...field} />
      </Row>

      <H>Camera and sound</H>
      <Row label="Camera">
        <CameraSelect {...field} devices={devices} />
      </Row>
      <Row label="Microphone">
        <MicSelect {...field} devices={devices} />
      </Row>
      <BluetoothNote options={options} />
      <div className="mt-2.5 flex items-center">
        <span className="text-[12.5px] text-ink-2">System audio</span>
        <span className="ml-auto">
          <Toggle label="System audio" on={options.record_system_audio} onChange={(on) => setOptions({ ...options, record_system_audio: on })} />
        </span>
      </div>

      <H>When you step away</H>
      <Row label="Skip after">
        <IdleSelect {...field} />
      </Row>
      <div className="mt-1.5 text-[11.5px] leading-[1.4] text-ink-3">{IDLE_EXPLAINED}</div>

      <H>Recordings folder</H>
      <div className="mt-2 rounded-card border border-line bg-card px-2.5 py-[9px]">
        <div className="num break-all text-[11px]">{env?.output_dir ?? ""}</div>
        <div className="num mt-[3px] text-[10.5px] text-ink-3">{free ?? ""}</div>
        <div className="mt-2 flex gap-3 text-[12px] font-semibold">
          <button
            className="text-ink-2 hover:text-ink disabled:text-ink-4"
            disabled={locked}
            title={locked ? "Stop recording to move the folder" : undefined}
            onClick={() => void pickOutputDir().then((p) => p && reloadEnv()).catch(() => {})}
          >
            Change
          </button>
          <button className="text-ink-2 hover:text-ink" onClick={() => void openFolder().catch(() => {})}>
            Open
          </button>
        </div>
      </div>
    </div>
  );
}
