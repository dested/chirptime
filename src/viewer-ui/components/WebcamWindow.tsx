import React, { useState, useRef, useCallback, useEffect } from "react";

interface Props {
  src: string | null;
  mainVideoRef: React.RefObject<HTMLVideoElement | null>;
}

export function WebcamWindow({ src, mainVideoRef }: Props) {
  const [minimized, setMinimized] = useState(false);
  const [pos, setPos] = useState({ x: -1, y: -1 }); // -1 = default position (top right)
  const webcamRef = useRef<HTMLVideoElement>(null);
  const dragging = useRef(false);
  const dragOffset = useRef({ x: 0, y: 0 });

  // Sync webcam to main video
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
  }, [mainVideoRef, src]);

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
      setPos({
        x: e.clientX - dragOffset.current.x,
        y: e.clientY - dragOffset.current.y,
      });
    };
    const onMouseUp = () => { dragging.current = false; };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, []);

  if (!src) return null;

  const style: React.CSSProperties = pos.x === -1
    ? { position: "fixed", top: 16, right: 16 }
    : { position: "fixed", left: pos.x, top: pos.y };

  return (
    <div
      className="webcam-float z-50 rounded-lg overflow-hidden bg-black"
      style={{ ...style, width: minimized ? 140 : 280 }}
    >
      <div
        className="webcam-header flex items-center justify-between px-2.5 py-1 cursor-move select-none"
        onMouseDown={onMouseDown}
      >
        <span className="text-zinc-400 text-xs font-medium">Webcam</span>
        <button
          onClick={() => setMinimized(!minimized)}
          className="text-zinc-500 hover:text-white text-xs px-1 transition-colors"
        >
          {minimized ? "+" : "\u2013"}
        </button>
      </div>
      {!minimized && (
        <video
          ref={webcamRef}
          key={src}
          src={src}
          muted
          className="w-full block"
        />
      )}
    </div>
  );
}
