export interface CaptureEntry {
  timestamp: string;
  display_time: string;
  screen_video: string | null;
  webcam_video: string | null;
  combined_video: string | null;
  screenshot: string | null;
}

export interface MonitorRegion {
  id: number;
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface MonitorConfig {
  totalWidth: number;
  totalHeight: number;
  monitors: MonitorRegion[];
}

export interface DateEntries {
  date: string;
  entries: CaptureEntry[];
  full_day_video: string | null;
}

export interface CaptureConfig {
  interval_minutes: number;
  video_mode: boolean;
  video_duration_seconds: number;
  video_framerate: number;
  video_preset: string;
  record_audio: boolean;
  audio_device: string | null;
  pip_overlay: boolean;
  webcam_device: string | null;
  output_dir: string;
}

export interface CaptureStatus {
  running: boolean;
  last_capture_time: string | null;
  next_capture_time: string | null;
  captures_today: number;
}

export interface AppState {
  dates: string[];
  selectedDate: string | null;
  dateEntry: DateEntries | null;
  selectedTimestamp: string | null;
  monitorConfig: MonitorConfig | null;
  zoomedMonitor: number | null;
  loading: boolean;
  captureStatus: CaptureStatus;
  outputDir: string;
}
