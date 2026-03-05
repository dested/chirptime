import React, { useState, useRef, useCallback, useEffect } from "react";
import { mediaUrl } from "../hooks/mediaUrl";

interface Props {
  src: string | null;
  mainVideoRef: React.RefObject<HTMLVideoElement | null>;
}

export function WebcamWindow({ src, mainVideoRef }: Props) {
  const [minimized, setMinimized] = useState(false);
  const [pos, setPos] = useState({ x: -1, y: -1 });
  const webcamRef = useRef<HTMLVideoElement>(null);
  const dragging = useRef(false);
  const dragOffset = useRef({ x: 0, y: 0 });

  const resolvedSrc = src ? mediaUrl(src) : null;

  useEffect(() => {
    const main = mainVideoRef.current;
    const webcam = webcamRef.current;
    if (!main || !webcam) return;

    const onPlay = () => { webcam.play().catch(() => {}); };
    const onPause = () => { webcam.pause(); };
    const onSeeked = () => { webcam.currentTime = main.currentTime; };
    const onRateChange = () => { webcam.playbackRate = main.playbackRate; };

    main.addEventListener("play", onPlay);
    main.addEventListener("pause", onPause);
    main.addEventListener("seeked", onSeeked);
    main.addEventListener("ratechange", onRateChange);

    return () => {
      main.removeEventListener("play", onPlay);
      main.removeEventListener("pause", onPause);
      main.removeEventListener("seeked", onSeeked);
      main.removeEventListener("ratechange", onRateChange);
    };
  }, [mainVideoRef, resolvedSrc]);

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    dragging.current = true;
    const el = (e.target as HTMLElement).closest(".webcam-float") as HTMLElement;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    dragOffset.current = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    e.preventDefault();
  }, []);

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      setPos({ x: e.clientX - dragOffset.current.x, y: e.clientY - dragOffset.current.y });
    };
    const onMouseUp = () => { dragging.current = false; };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, []);

  if (!resolvedSrc) return null;

  const style: React.CSSProperties = pos.x === -1
    ? { position: "fixed", top: 16, right: 16 }
    : { position: "fixed", left: pos.x, top: pos.y };

  return (
    <div
      className="webcam-float"
      style={{ ...style, width: minimized ? 140 : 280, zIndex: 50, borderRadius: 8, overflow: "hidden", background: "#000" }}
    >
      <div
        className="webcam-header"
        style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "4px 10px", cursor: "move", userSelect: "none" }}
        onMouseDown={onMouseDown}
      >
        <span style={{ color: "#a1a1aa", fontSize: 12, fontWeight: 500 }}>Webcam</span>
        <button
          onClick={() => setMinimized(!minimized)}
          style={{ color: "#71717a", fontSize: 12, padding: "0 4px", background: "none", border: "none", cursor: "pointer" }}
        >
          {minimized ? "+" : "\u2013"}
        </button>
      </div>
      {!minimized && (
        <video
          ref={webcamRef}
          key={resolvedSrc}
          src={resolvedSrc}
          muted
          style={{ width: "100%", display: "block" }}
        />
      )}
    </div>
  );
}
