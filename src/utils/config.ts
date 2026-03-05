export interface ScreenerConfig {
  /** Minutes between captures */
  intervalMinutes: number;
  /** Enable video mode instead of snapshot mode */
  videoMode: boolean;
  /** Duration of video clips in seconds */
  videoDurationSeconds: number;
  /** Run once then exit */
  once: boolean;
  /** Just list devices and exit */
  listDevices: boolean;
  /** Base output directory */
  outputDir: string;
  /** Webcam device name (auto-detected if not provided) */
  webcamDevice: string | null;
  /** Screenshot image format */
  screenshotFormat: "png" | "jpg";
  /** Video codec preset (ultrafast = low CPU) */
  videoPreset: "ultrafast" | "superfast" | "veryfast" | "faster" | "fast" | "medium";
  /** Video framerate for screen recording */
  videoFramerate: number;
  /** Enable webcam PiP overlay on screen recordings */
  pipOverlay: boolean;
  /** Record audio from webcam microphone */
  recordAudio: boolean;
}

export const DEFAULT_CONFIG: ScreenerConfig = {
  intervalMinutes: 10,
  videoMode: true,
  videoDurationSeconds: 5,
  once: false,
  listDevices: false,
  outputDir: "./output",
  webcamDevice: null,
  screenshotFormat: "png",
  videoPreset: "ultrafast",
  videoFramerate: 30,
  pipOverlay: true,
  recordAudio: true,
};
