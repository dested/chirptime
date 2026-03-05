import React, { useEffect, useMemo, useRef, useState } from "react";
import { mediaUrl } from "../hooks/mediaUrl";
import type { MonitorConfig, CaptureEntry } from "../types";

interface Props {
  entry: CaptureEntry | null;
  monitorConfig: MonitorConfig | null;
  zoomedMonitor: number | null;
  mainVideoRef: React.RefObject<HTMLVideoElement | null>;
  onEnded?: () => void;
  playbackRate?: number;
  outputDir: string;
}

export function ScreenView({ entry, monitorConfig, zoomedMonitor, mainVideoRef, onEnded, playbackRate = 1 }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerSize, setContainerSize] = useState({ w: 0, h: 0 });

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const obs = new ResizeObserver(([e]) => {
      setContainerSize({ w: e.contentRect.width, h: e.contentRect.height });
    });
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    const video = mainVideoRef.current;
    if (video) video.playbackRate = playbackRate;
  }, [playbackRate, mainVideoRef, entry]);

  useEffect(() => {
    const video = mainVideoRef.current;
    if (!video) return;
    const onLoaded = () => { video.playbackRate = playbackRate; };
    video.addEventListener("loadedmetadata", onLoaded);
    return () => video.removeEventListener("loadedmetadata", onLoaded);
  }, [playbackRate, mainVideoRef, entry]);

  const videoSrc = useMemo(() => {
    if (!entry) return null;
    const rel = entry.combined_video ?? entry.screen_video;
    if (!rel) return null;
    return mediaUrl(rel);
  }, [entry]);

  const screenshotSrc = useMemo(() => {
    if (!entry?.screenshot) return null;
    return mediaUrl(entry.screenshot);
  }, [entry]);

  const isZoomed = monitorConfig && zoomedMonitor !== null;

  // Compute the inner wrapper style that positions and scales the media
  const innerStyle = useMemo((): React.CSSProperties => {
    if (!monitorConfig || zoomedMonitor === null || !containerSize.w) {
      return { width: "100%", position: "relative" as const };
    }
    const mon = monitorConfig.monitors.find((m) => m.id === zoomedMonitor);
    if (!mon) return { width: "100%", position: "relative" as const };

    // Video native: 3840x1296. Monitor is at (x, y) with size (w, h).
    // We want the monitor's portion to fill the container width.
    // Scale factor: how much bigger the full video needs to be so that mon.w maps to container width.
    const scale = containerSize.w / mon.w; // e.g. 1000 / 960 ~= 1.04

    // Full video rendered size at this scale
    const renderedW = monitorConfig.totalWidth * scale; // e.g. 3840 * 1.04 = 3993
    const renderedH = monitorConfig.totalHeight * scale;

    // Offset to move monitor region to top-left
    const offsetX = mon.x * scale;
    const offsetY = mon.y * scale;

    return {
      position: "relative" as const,
      width: renderedW,
      height: renderedH,
      left: -offsetX,
      top: -offsetY,
    };
  }, [monitorConfig, zoomedMonitor, containerSize]);

  if (!entry) {
    return (
      <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", color: "#52525b", fontSize: 14 }}>
        Select a capture to view
      </div>
    );
  }

  const mediaEl = videoSrc ? (
    <video
      ref={mainVideoRef}
      key={videoSrc}
      src={videoSrc}
      controls
      autoPlay
      style={{ width: "100%", height: "100%", display: "block" }}
      onEnded={onEnded}
    />
  ) : screenshotSrc ? (
    <img
      src={screenshotSrc}
      style={{ width: "100%", height: "100%", display: "block", objectFit: "contain" }}
      alt={`Screenshot ${entry.display_time}`}
    />
  ) : (
    <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: 256, color: "#52525b" }}>
      No media available
    </div>
  );

  return (
    <div style={{ flex: 1, minHeight: 0, padding: "0 12px 8px" }}>
      <div
        ref={containerRef}
        className="video-container"
        style={{ width: "100%", height: "100%", overflow: "hidden" }}
      >
        {isZoomed ? (
          <div style={innerStyle}>
            {mediaEl}
          </div>
        ) : (
          mediaEl
        )}
      </div>
    </div>
  );
}
