export interface CaptureEntry {
  timestamp: string; // "HH-MM-SS"
  displayTime: string; // "HH:MM:SS"
  screenVideo?: string; // relative path from output/
  webcamVideo?: string;
  combinedVideo?: string;
  screenshot?: string;
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

export interface DateEntry {
  date: string;
  entries: CaptureEntry[];
  fullDayVideo?: string;
}

export interface AppState {
  dates: string[];
  selectedDate: string | null;
  dateEntry: DateEntry | null;
  selectedTimestamp: string | null;
  monitorConfig: MonitorConfig | null;
  zoomedMonitor: number | null; // null = "All"
  loading: boolean;
}
