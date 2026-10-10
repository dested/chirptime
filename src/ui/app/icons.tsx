/** The app's line icons, drawn on a 24-unit grid like the mocks. */
import type { ReactNode } from "react";

interface P {
  size?: number;
  stroke?: number;
  className?: string;
}

function Svg({ size = 16, stroke = 2, className, children, fill }: P & { children: ReactNode; fill?: boolean }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={fill ? "currentColor" : "none"}
      stroke={fill ? "none" : "currentColor"}
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export const IconLibrary = (p: P) => (
  <Svg {...p}>
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <path d="M3 10h18M9 4v16" />
  </Svg>
);
export const IconFlag = (p: P) => (
  <Svg {...p}>
    <path d="M5 21V4h12l-2 4 2 4H5" />
  </Svg>
);
export const IconExport = (p: P) => (
  <Svg {...p}>
    <path d="M12 15V3M7 8l5-5 5 5" />
    <path d="M5 14v5a2 2 0 002 2h10a2 2 0 002-2v-5" />
  </Svg>
);
export const IconSliders = (p: P) => (
  <Svg {...p}>
    <path d="M4 7h10M18 7h2M4 17h4M12 17h8" />
    <circle cx="16" cy="7" r="2" />
    <circle cx="10" cy="17" r="2" />
  </Svg>
);
export const IconBack = (p: P) => (
  <Svg stroke={2.2} {...p}>
    <path d="M15 18l-6-6 6-6" />
  </Svg>
);
export const IconLayers = (p: P) => (
  <Svg {...p}>
    <path d="M12 3l9 5-9 5-9-5z" />
    <path d="M3 13l9 5 9-5" />
  </Svg>
);
export const IconVolume = (p: P) => (
  <Svg {...p}>
    <path d="M4 9v6h4l5 4V5L8 9z" />
    <path d="M16.5 8.5a5 5 0 010 7M19 6a8.5 8.5 0 010 12" />
  </Svg>
);
export const IconMuted = (p: P) => (
  <Svg {...p}>
    <path d="M4 9v6h4l5 4V5L8 9z" />
    <path d="M17 9l4 6M21 9l-4 6" />
  </Svg>
);
export const IconTrash = (p: P) => (
  <Svg stroke={2.2} {...p}>
    <path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14" />
  </Svg>
);
export const IconX = (p: P) => (
  <Svg stroke={2.4} {...p}>
    <path d="M6 6l12 12M18 6L6 18" />
  </Svg>
);
export const IconDown = (p: P) => (
  <Svg stroke={2.4} {...p}>
    <path d="M6 9l6 6 6-6" />
  </Svg>
);
export const IconWarn = (p: P) => (
  <Svg stroke={2.4} {...p}>
    <path d="M12 3l9.5 17h-19z" />
    <path d="M12 10v4M12 17.5v.5" />
  </Svg>
);
export const IconCheck = (p: P) => (
  <Svg stroke={2.4} {...p}>
    <path d="M5 12.5l4.5 4.5L19 7.5" />
  </Svg>
);
export const IconPlay = (p: P) => (
  <Svg fill {...p}>
    <path d="M7 4l13 8-13 8z" />
  </Svg>
);
export const IconPause = (p: P) => (
  <Svg fill {...p}>
    <rect x="5" y="4" width="5" height="16" rx="1" />
    <rect x="14" y="4" width="5" height="16" rx="1" />
  </Svg>
);

/** The ChirpTime mark: a record dot that chirps. The dot is the status. */
export function Mark({ size = 18, dot = "var(--color-ink)", arcs = "var(--color-ink-3)" }: { size?: number; dot?: string; arcs?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="9" cy="12" r="5.2" fill={dot} />
      <path d="M16.2 8.6a5 5 0 010 6.8" stroke={arcs} strokeWidth="2" strokeLinecap="round" />
      <path d="M19.4 5.8a9 9 0 010 12.4" stroke={arcs} strokeWidth="2" strokeLinecap="round" opacity=".55" />
    </svg>
  );
}
