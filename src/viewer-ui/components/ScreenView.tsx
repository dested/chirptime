import React, { useEffect, useMemo } from "react";
import type { MonitorConfig, CaptureEntry } from "../types";

interface Props {
  entry: CaptureEntry | null;
  monitorConfig: MonitorConfig | null;
  zoomedMonitor: number | null;
  mainVideoRef: React.RefObject<HTMLVideoElement | null>;
  onEnded?: () => void;
  playbackRate?: number;
}

export function ScreenView({ entry, monitorConfig, zoomedMonitor, mainVideoRef, onEnded, playbackRate = 1 }: Props) {
  // Set playback rate whenever it changes or video swaps
  useEffect(() => {
    const video = mainVideoRef.current;
    if (video) video.playbackRate = playbackRate;
  }, [playbackRate, mainVideoRef, entry]);

  // Also set rate once the video metadata loads (new src resets playbackRate to 1)
  useEffect(() => {
    const video = mainVideoRef.current;
    if (!video) return;
    const onLoaded = () => { video.playbackRate = playbackRate; };
    video.addEventListener("loadedmetadata", onLoaded);
    return () => video.removeEventListener("loadedmetadata", onLoaded);
  }, [playbackRate, mainVideoRef, entry]);

  const videoSrc = useMemo(() => {
    if (!entry) return null;
    if (entry.screenVideo) return `/files/${entry.screenVideo}`;
    if (entry.combinedVideo) return `/files/${entry.combinedVideo}`;
    return null;
  }, [entry]);

  const screenshotSrc = useMemo(() => {
    if (!entry?.screenshot) return null;
    return `/files/${entry.screenshot}`;
  }, [entry]);

  const cropStyle = useMemo(() => {
    if (!monitorConfig || zoomedMonitor === null) {
      return { transform: "none", transformOrigin: "0 0" };
    }
    const mon = monitorConfig.monitors.find((m) => m.id === zoomedMonitor);
    if (!mon) return { transform: "none", transformOrigin: "0 0" };

    const scaleX = monitorConfig.totalWidth / mon.w;
    const scaleY = monitorConfig.totalHeight / mon.h;
    const scale = Math.min(scaleX, scaleY);
    const tx = (mon.x / monitorConfig.totalWidth) * 100;
    const ty = (mon.y / monitorConfig.totalHeight) * 100;

    return {
      transform: `scale(${scale}) translate(${-tx}%, ${-ty}%)`,
      transformOrigin: "0 0",
    };
  }, [monitorConfig, zoomedMonitor]);

  if (!entry) {
    return (
      <div className="flex-1 flex items-center justify-center text-zinc-600 text-sm tracking-wide">
        Select a capture to view
      </div>
    );
  }

  return (
    <div className="flex-1 min-h-0 px-3 pb-2">
      <div className="video-container w-full h-full">
        {videoSrc ? (
          <video
            ref={mainVideoRef}
            key={videoSrc}
            src={videoSrc}
            controls
            autoPlay
            style={cropStyle}
            className="w-full"
            onEnded={onEnded}
          />
        ) : screenshotSrc ? (
          <img
            src={screenshotSrc}
            style={cropStyle}
            className="w-full"
            alt={`Screenshot ${entry.displayTime}`}
          />
        ) : (
          <div className="flex items-center justify-center h-64 text-zinc-600">
            No media available
          </div>
        )}
      </div>
    </div>
  );
}
