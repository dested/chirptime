/**
 * The three global hotkeys, each with what Windows said when it was
 * registered. Click one, press a new combination; Backspace turns it off,
 * Escape keeps the old one.
 */
import { useCallback, useEffect, useState } from "react";
import {
  DEFAULT_HOTKEYS,
  getHotkeys,
  keyLabel,
  setHotkeys,
  type HotkeyAction,
  type HotkeyStatus,
  type Hotkeys,
} from "../recorder/api";
import { btn } from "./Ffmpeg";

const ACTIONS: { action: HotkeyAction; label: string; hint: string }[] = [
  { action: "record", label: "Start and stop", hint: "Starts with your saved options, from anywhere" },
  { action: "mark", label: "Mark", hint: "Flags this moment in the recording" },
  { action: "pause", label: "Pause and resume", hint: "Stops every track until you resume" },
];

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "AltGraph", "OS"]);

/** `Ctrl+Alt+R` from a keydown, or null while only modifiers are held. */
export function acceleratorOf(e: { key: string; code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  let key: string | null = null;
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
  else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  else if (/^F\d{1,2}$/.test(e.code)) key = e.code;
  else if (["Space", "Home", "End", "PageUp", "PageDown", "Insert", "Pause", "PrintScreen"].includes(e.code)) key = e.code;
  else if (/^Numpad\d$/.test(e.code)) key = e.code;
  if (!key) return null;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter((m): m is string => typeof m === "string");
  return [...mods, key].join("+");
}

export function HotkeysEditor({ hotkeys, onSaved }: { hotkeys: Hotkeys; onSaved: () => void }) {
  const [status, setStatus] = useState<HotkeyStatus[] | null>(null);
  const [keys, setKeys] = useState<Hotkeys>(hotkeys);
  const [capturing, setCapturing] = useState<HotkeyAction | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => setKeys(hotkeys), [hotkeys]);
  useEffect(() => {
    getHotkeys().then(setStatus).catch(() => setStatus(null));
  }, []);

  const save = useCallback(
    async (next: Hotkeys) => {
      setKeys(next);
      setError(null);
      try {
        setStatus(await setHotkeys(next));
        onSaved();
      } catch (e) {
        setError(String(e));
      }
    },
    [onSaved],
  );

  useEffect(() => {
    if (!capturing) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return setCapturing(null);
      if ((e.key === "Backspace" || e.key === "Delete") && !e.ctrlKey && !e.altKey) {
        setCapturing(null);
        void save({ ...keys, [capturing]: "" });
        return;
      }
      const acc = acceleratorOf(e);
      if (!acc) return;
      if (!acc.includes("+")) {
        setError("Use at least one of Ctrl, Alt or Shift, so typing never sets it off");
        return;
      }
      setCapturing(null);
      void save({ ...keys, [capturing]: acc });
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capturing, keys, save]);

  const isDefault = ACTIONS.every(({ action }) => keys[action] === DEFAULT_HOTKEYS[action]);

  return (
    <div>
      {ACTIONS.map(({ action, label, hint }) => {
        const st = status?.find((s) => s.action === action);
        const acc = keys[action];
        return (
          <div key={action} className="flex items-center gap-4 border-b border-line-2 py-2.5 last:border-b-0">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold">{label}</div>
              <div className={`mt-0.5 text-[11.5px] ${st?.error ? "text-[#9A5A0C]" : "text-ink-3"}`}>{st?.error ?? hint}</div>
            </div>
            <button
              onClick={() => setCapturing(capturing === action ? null : action)}
              className={`h-8 min-w-[132px] rounded-ctl border px-3 text-left transition-colors ${
                capturing === action ? "border-ink bg-card" : "border-line bg-card hover:border-ink-4"
              }`}
              aria-label={`${label} hotkey`}
            >
              {capturing === action ? (
                <span className="text-[12px] text-ink-3">Press keys</span>
              ) : acc ? (
                <span className="kbd">{keyLabel(acc)}</span>
              ) : (
                <span className="text-[12px] text-ink-3">Off</span>
              )}
            </button>
          </div>
        );
      })}
      <div className="mt-3 flex items-center gap-3">
        <button className={btn} disabled={isDefault} onClick={() => void save(DEFAULT_HOTKEYS)}>
          Back to the defaults
        </button>
        <span className="text-[11.5px] text-ink-3">{capturing ? "Backspace turns it off, Escape keeps it" : "They work from any app, even with the recorder hidden"}</span>
      </div>
      {error && <div className="mt-2 text-[11.5px] text-rec">{error}</div>}
    </div>
  );
}
