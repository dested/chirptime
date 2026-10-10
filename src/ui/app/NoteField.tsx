/**
 * A mark's note, edited in place: click the text (or "Add a note"), type,
 * Enter or click away to save, Esc to leave it as it was. Saves through
 * `set_marker_label`; an empty note clears it. Used by the Marks page and the
 * Player's log.
 */
import { useEffect, useRef, useState } from "react";
import { setMarkerLabel } from "../hooks/useApi";
import { errorText } from "./appData";

interface Props {
  sessionId: string;
  epochMs: number;
  label: string;
  /** null when editable; otherwise why not, shown as the tooltip */
  lockedReason: string | null;
  onSaved: (label: string) => void;
  className?: string;
  /** the colour of a written note */
  tone?: string;
}

export function NoteField({ sessionId, epochMs, label, lockedReason, onSaved, className, tone = "text-ink-3" }: Props) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(label);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  // Enter saves and blurs; the blur that follows must not save twice.
  const settled = useRef(false);

  useEffect(() => {
    if (!editing) setDraft(label);
  }, [label, editing]);

  useEffect(() => {
    if (editing) input.current?.select();
  }, [editing]);

  const save = async () => {
    if (settled.current) return;
    settled.current = true;
    const next = draft.trim();
    if (next === label.trim()) {
      setEditing(false);
      return;
    }
    setSaving(true);
    try {
      const mk = await setMarkerLabel(sessionId, epochMs, next);
      onSaved(mk.label);
      setError(null);
      setEditing(false);
    } catch (e) {
      setError(errorText(e));
      settled.current = false;
    } finally {
      setSaving(false);
    }
  };

  const cancel = () => {
    settled.current = true;
    setDraft(label);
    setError(null);
    setEditing(false);
  };

  const text = label.trim();

  if (!editing) {
    return (
      <div className={className}>
        <button
          className={`max-w-full truncate rounded-[5px] text-left transition-colors duration-150 ${
            lockedReason ? "cursor-default" : "-mx-1 px-1 hover:bg-sunk"
          } ${text ? tone : "text-ink-4"}`}
          title={lockedReason ?? (text ? "Edit the note" : "Add a note")}
          onClick={(e) => {
            e.stopPropagation();
            if (lockedReason) return;
            settled.current = false;
            setError(null);
            setEditing(true);
          }}
        >
          {text || "Add a note"}
        </button>
        {error && <div className="mt-0.5 text-[11.5px] text-rec">{error}</div>}
      </div>
    );
  }

  return (
    <div className={className} onClick={(e) => e.stopPropagation()}>
      <input
        ref={input}
        value={draft}
        disabled={saving}
        maxLength={200}
        placeholder="Add a note"
        aria-label="Note"
        className="-mx-1 h-[22px] w-[calc(100%+8px)] rounded-[5px] border border-canary-deep bg-card px-1 text-ink outline-none placeholder:text-ink-4 disabled:opacity-60"
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => void save()}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") {
            e.preventDefault();
            void save();
          } else if (e.key === "Escape") {
            e.preventDefault();
            cancel();
          }
        }}
      />
      {error && <div className="mt-0.5 text-[11.5px] text-rec">{error}</div>}
    </div>
  );
}
