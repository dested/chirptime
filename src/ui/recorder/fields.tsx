/** The recording option controls, shared by the widget's options, Settings and first run. */
import type { DshowDevices, SessionOptions } from "../types";
import { FPS_CHOICES, IDLE_CHOICES, looksBluetooth } from "./setup";
import { Select } from "./ui";

/** The value of a device select: off, automatic, or a named device. */
const OFF = "\u0000off";
const AUTO = "";

interface FieldProps {
  options: SessionOptions;
  setOptions: (next: SessionOptions) => void;
  className?: string;
}

export function CameraSelect({ options, setOptions, devices, className }: FieldProps & { devices: DshowDevices | null }) {
  return (
    <Select
      className={className}
      aria-label="Camera"
      value={!options.record_webcam ? OFF : options.webcam_device ?? AUTO}
      onChange={(e) => {
        const v = e.target.value;
        setOptions(v === OFF ? { ...options, record_webcam: false } : { ...options, record_webcam: true, webcam_device: v || null });
      }}
    >
      <option value={AUTO}>First camera</option>
      {(devices?.video ?? []).map((d) => (
        <option key={d} value={d}>
          {d}
        </option>
      ))}
      <option value={OFF}>Off</option>
    </Select>
  );
}

export function MicSelect({ options, setOptions, devices, className }: FieldProps & { devices: DshowDevices | null }) {
  return (
    <Select
      className={className}
      aria-label="Microphone"
      value={!options.record_mic ? OFF : options.mic_device ?? AUTO}
      onChange={(e) => {
        const v = e.target.value;
        setOptions(v === OFF ? { ...options, record_mic: false } : { ...options, record_mic: true, mic_device: v || null });
      }}
    >
      <option value={AUTO}>Automatic · never Bluetooth</option>
      {(devices?.audio ?? []).map((d) => (
        <option key={d} value={d}>
          {looksBluetooth(d) ? `${d} · Bluetooth` : d}
        </option>
      ))}
      <option value={OFF}>Off</option>
    </Select>
  );
}

export function BluetoothNote({ options }: { options: SessionOptions }) {
  if (!options.record_mic || !options.mic_device || !looksBluetooth(options.mic_device)) return null;
  return (
    <div className="mt-1.5 text-[11.5px] leading-[1.4] text-[#9A5A0C]">
      Recording a Bluetooth mic switches the headset to hands-free: what you hear drops to phone quality until the session ends.
    </div>
  );
}

export function FpsSelect({ options, setOptions, className }: FieldProps) {
  return (
    <Select
      className={className}
      aria-label="Frame rate"
      value={options.screen_fps}
      onChange={(e) => setOptions({ ...options, screen_fps: Number(e.target.value) })}
    >
      {FPS_CHOICES.map((f) => (
        <option key={f} value={f}>
          {f} fps
        </option>
      ))}
    </Select>
  );
}

export function IdleSelect({ options, setOptions, className }: FieldProps) {
  return (
    <Select
      className={className}
      aria-label="Skip footage after"
      value={options.drop_idle_after_seconds}
      onChange={(e) => setOptions({ ...options, drop_idle_after_seconds: Number(e.target.value) })}
    >
      {IDLE_CHOICES.map((c) => (
        <option key={c.seconds} value={c.seconds}>
          {c.label}
        </option>
      ))}
    </Select>
  );
}

export const IDLE_EXPLAINED = "With no keyboard or mouse input for this long, the footage is not kept. Watching a video counts as away.";
