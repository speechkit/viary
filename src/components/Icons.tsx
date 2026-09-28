// The Viary mark and the stroke icons from the design.

import type { SVGProps } from "react";

/** Five rounded bars; the tall middle one is the live voice. */
export function Mark({
  size = 22,
  ink = "#F4F1EA",
  voice = "#FF6B3D",
}: {
  size?: number;
  ink?: string;
  voice?: string;
}) {
  return (
    <svg viewBox="0 0 48 48" width={size} height={size} aria-hidden="true" className="block shrink-0">
      <rect x="2" y="16" width="6" height="16" rx="3" fill={ink} />
      <rect x="11.5" y="11" width="6" height="26" rx="3" fill={ink} />
      <rect x="21" y="5" width="6" height="38" rx="3" fill={voice} />
      <rect x="30.5" y="11" width="6" height="26" rx="3" fill={ink} />
      <rect x="40" y="16" width="6" height="16" rx="3" fill={ink} />
    </svg>
  );
}

/** The mark on its dark tile, as in the menu bar popover and the sidebar. */
export function AppTile({ size = 36, radius = 8 }: { size?: number; radius?: number }) {
  return (
    <span
      role="img"
      aria-label="Viary"
      className="inline-flex shrink-0 items-center justify-center bg-ink"
      style={{ width: size, height: size, borderRadius: radius }}
    >
      <Mark size={Math.round(size * 0.61)} />
    </span>
  );
}

const paths = {
  sparkle: "M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z",
  check: "M5 12l5 5L20 7",
  undo: "M9 14L4 9l5-5M4 9h11a5 5 0 0 1 0 10h-3",
  copy: "M9 9h11v11H9zM5 15H4V4h11v1",
  warning:
    "M12 8v5M12 16.5v.01M10.3 3.9L2 18a2 2 0 0 0 1.7 3h16.6a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z",
  close: "M6 6l12 12M18 6L6 18",
  mic: "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM5 11a7 7 0 0 0 14 0M12 18v3",
  laptop: "M4 6h16v10H4zM2 19h20",
  cloud: "M7 18a4 4 0 0 1-.5-7.97A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9H7z",
  search: "M11 4a7 7 0 1 0 0 14a7 7 0 0 0 0-14zM20 20l-4-4",
  play: "M8 5v14l11-7z",
  pause: "M8 5v14M16 5v14",
  home: "M3 11l9-7 9 7M5 10v10h14V10",
  history: "M21 12a9 9 0 1 1-18 0a9 9 0 0 1 18 0M12 7v5l3 2",
  dictionary: "M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2V5zM4 19a2 2 0 0 1 2-2h13",
  engine: "M7 7h10v10H7zM10 3v4M14 3v4M10 17v4M14 17v4M3 10h4M3 14h4M17 10h4M17 14h4",
  settings:
    "M12 9a3 3 0 1 0 0 6a3 3 0 0 0 0-6zM12 2v3M12 19v3M4.9 4.9L7 7M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1L7 17M17 7l2.1-2.1",
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  plus: "M12 5v14M5 12h14",
  key: "M15 7a4 4 0 1 1-3.9 4.9L4 19v-3h3v-3h3l1.1-1.1A4 4 0 0 1 15 7z",
  refresh: "M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6",
  trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
  shield: "M12 3l8 3v6c0 5-3.5 8-8 9c-4.5-1-8-4-8-9V6z",
  translate: "M4 5h8M8 3v2M6 5c0 4 3 7 6 8M10 5c-1 4-4 7-6 8M13 21l4-9 4 9M14.5 18h5",
} as const;

export type IconName = keyof typeof paths;

export function Icon({
  name,
  size = 16,
  ...rest
}: { name: IconName; size?: number } & Omit<SVGProps<SVGSVGElement>, "name">) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className="shrink-0"
      {...rest}
    >
      <path d={paths[name]} />
    </svg>
  );
}

/** A key cap, as in "Hold fn to dictate". */
export function Kbd({ children, large = false }: { children: React.ReactNode; large?: boolean }) {
  return (
    <span
      className={
        "inline-flex items-center justify-center rounded-[6px] border border-stone border-b-2 bg-white font-mono " +
        (large ? "h-[30px] min-w-[30px] px-[9px] text-[13px] rounded-[8px] border-b-[3px]" : "h-[22px] min-w-6 px-1.5 text-[11px]")
      }
    >
      {children}
    </span>
  );
}
