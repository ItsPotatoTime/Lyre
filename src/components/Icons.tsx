import type { ReactNode } from "react";

/**
 * One drawn icon set, 20x20 grid, 1.5px stroke, round joins, currentColor.
 * Explorer's glyphs are Fluent System Icons; these are drawn to the same
 * grammar (even weights, generous counters) rather than borrowed from an
 * icon font, so they stay crisp at 12px and consistent at 20px.
 */

const GLYPHS = {
  chevronDown: <path d="M5.5 8.5 10 13l4.5-4.5" />,
  chevronRight: <path d="M8.5 5.5 13 10l-4.5 4.5" />,
  chevronUp: <path d="M5.5 11.5 10 7l4.5 4.5" />,
  chevronLeft: <path d="M11.5 5.5 7 10l4.5 4.5" />,
  arrowLeft: <path d="M4 10h12M9 5l-5 5 5 5" />,
  arrowRight: <path d="M16 10H4M11 5l5 5-5 5" />,
  arrowUp: <path d="M10 16V4M5 9l5-5 5 5" />,
  arrowDown: <path d="M10 4v12M5 11l5 5 5-5" />,
  refresh: (
    <>
      <path d="M16.8 10a6.8 6.8 0 1 1-1.99-4.81" />
      <path d="M16.9 3.4v4.4h-4.4" />
    </>
  ),
  home: (
    <path d="M3.4 9 10 3.1 16.6 9v7.1a1.2 1.2 0 0 1-1.2 1.2h-3.1v-4.5H7.7v4.5H4.6a1.2 1.2 0 0 1-1.2-1.2z" />
  ),
  search: (
    <>
      <circle cx="9" cy="9" r="5.6" />
      <path d="M13.2 13.2 17.6 17.6" />
    </>
  ),
  plus: <path d="M10 4.4v11.2M4.4 10h11.2" />,
  folder: (
    <path d="M3 6.3a1.8 1.8 0 0 1 1.8-1.8h3.1l1.7 2.1h6.6A1.8 1.8 0 0 1 18 8.4v6.5a1.8 1.8 0 0 1-1.8 1.8H4.8A1.8 1.8 0 0 1 3 14.9z" />
  ),
  folderPlus: (
    <>
      <path d="M3 6.3a1.8 1.8 0 0 1 1.8-1.8h3.1l1.7 2.1h6.6A1.8 1.8 0 0 1 18 8.4v6.5a1.8 1.8 0 0 1-1.8 1.8H4.8A1.8 1.8 0 0 1 3 14.9z" />
      <path d="M10.5 9.6v4M8.5 11.6h4" />
    </>
  ),
  cut: (
    <>
      <circle cx="5.8" cy="6.6" r="2.1" />
      <circle cx="5.8" cy="13.4" r="2.1" />
      <path d="M7.6 7.8 16.4 16M7.6 12.2 16.4 4" />
    </>
  ),
  copy: (
    <>
      <rect x="3.4" y="3.4" width="9" height="9" rx="1.6" />
      <path d="M13.4 7h1.8a1.6 1.6 0 0 1 1.6 1.6v6.8a1.6 1.6 0 0 1-1.6 1.6H8.4a1.6 1.6 0 0 1-1.6-1.6v-1.8" />
    </>
  ),
  paste: (
    <>
      <path d="M7.6 4.4H5.9a1.6 1.6 0 0 0-1.6 1.6v9.4a1.6 1.6 0 0 0 1.6 1.6h8.2a1.6 1.6 0 0 0 1.6-1.6V6a1.6 1.6 0 0 0-1.6-1.6h-1.7" />
      <rect x="7.6" y="2.6" width="4.8" height="3.2" rx="1" />
    </>
  ),
  rename: (
    <path d="M12.9 3.7a1.8 1.8 0 0 1 2.5 0l.9.9a1.8 1.8 0 0 1 0 2.5l-8.6 8.6-3.7 1 1-3.7z" />
  ),
  share: (
    <>
      <path d="M12 3.6h4.4V8M16.4 3.6 10.2 9.8" />
      <path d="M8.6 4.5H5.2a1.7 1.7 0 0 0-1.7 1.7v8.5a1.7 1.7 0 0 0 1.7 1.7h8.5a1.7 1.7 0 0 0 1.7-1.7v-3.3" />
    </>
  ),
  trash: (
    <>
      <path d="M3.6 6h12.8M7.8 6V4.7a1.1 1.1 0 0 1 1.1-1.1h2.2a1.1 1.1 0 0 1 1.1 1.1V6" />
      <path d="M5.4 6h9.2l-.7 9.4a1.4 1.4 0 0 1-1.4 1.3H7.5a1.4 1.4 0 0 1-1.4-1.3z" />
    </>
  ),
  sort: (
    <>
      <path d="M3.6 6.4h8.2M3.6 10.4h5.6M3.6 14.4h3" />
      <path d="M15.4 5v9" />
      <path d="M12.8 11.6 15.4 14.2l2.6-2.6" />
    </>
  ),
  viewDetails: (
    <>
      <path d="M3.6 6h1.2M3.6 10h1.2M3.6 14h1.2" />
      <path d="M7.6 6h8.8M7.6 10h8.8M7.6 14h8.8" />
    </>
  ),
  viewGrid: (
    <>
      <rect x="3.4" y="3.4" width="5.8" height="5.8" rx="1.2" />
      <rect x="10.8" y="3.4" width="5.8" height="5.8" rx="1.2" />
      <rect x="3.4" y="10.8" width="5.8" height="5.8" rx="1.2" />
      <rect x="10.8" y="10.8" width="5.8" height="5.8" rx="1.2" />
    </>
  ),
  more: (
    <>
      <circle cx="4.6" cy="10" r="1.35" fill="currentColor" stroke="none" />
      <circle cx="10" cy="10" r="1.35" fill="currentColor" stroke="none" />
      <circle cx="15.4" cy="10" r="1.35" fill="currentColor" stroke="none" />
    </>
  ),
  drive: (
    <>
      <rect x="2.4" y="6.2" width="15.2" height="7.6" rx="1.8" />
      <path d="M5.6 10h.02M8.6 10h.02" strokeWidth="2" />
    </>
  ),
  desktop: (
    <>
      <rect x="2.4" y="4" width="15.2" height="10" rx="1.6" />
      <path d="M7 17h6M10 14v3" />
    </>
  ),
  download: (
    <>
      <path d="M10 3.4v9.4M6.2 9 10 12.8 13.8 9" />
      <path d="M4.4 16.6h11.2" />
    </>
  ),
  document: (
    <>
      <path d="M5 3.4h6.4L16 8v8.6H5z" />
      <path d="M11.4 3.4V8H16" />
    </>
  ),
  image: (
    <>
      <rect x="2.6" y="4" width="14.8" height="12" rx="2" />
      <circle cx="7.2" cy="8.4" r="1.2" />
      <path d="M3.4 14.2 7.6 10.6l2.9 2.5 2.4-1.9 3.7 3" />
    </>
  ),
  audio: (
    <>
      <path d="M8.2 15V5.8l7.6-1.9v9" />
      <circle cx="6" cy="15.4" r="2.2" />
      <circle cx="13.6" cy="13.2" r="2.2" />
    </>
  ),
  video: (
    <>
      <rect x="2.6" y="5" width="10.4" height="10" rx="2" />
      <path d="M13.4 10.2 17.4 7.4v5.2z" />
    </>
  ),
  close: <path d="M5 5l10 10M15 5 5 15" />,
  minimize: <path d="M4 10h12" />,
  maximize: <rect x="4.6" y="4.6" width="10.8" height="10.8" rx="1" />,
  restore: (
    <>
      <rect x="3.4" y="6.6" width="9" height="9" rx="1.2" />
      <path d="M7 6.6V4.9a1.5 1.5 0 0 1 1.5-1.5h6.6A1.5 1.5 0 0 1 16.6 4.9v6.6a1.5 1.5 0 0 1-1.5 1.5h-1.7" />
    </>
  ),
  check: <path d="M4.6 10.5 8.2 14 15.4 6.4" />,
  sidebar: (
    <>
      <rect x="2.6" y="4" width="14.8" height="12" rx="2" />
      <path d="M7.8 4v12" />
    </>
  ),
  pin: (
    <>
      <path d="M7.2 3.4h5.6M8.6 3.4v4.8L6.2 12h7.6l-2.4-3.8V3.4" />
      <path d="M10 12v4.6" />
    </>
  ),
  clock: (
    <>
      <circle cx="10" cy="10" r="6.8" />
      <path d="M10 6.2v4.2l2.8 1.7" />
    </>
  ),
  star: (
    <path d="m10 3.4 2 4.2 4.6.6-3.4 3.2.9 4.6-4.1-2.3-4.1 2.3.9-4.6L3.4 8.2 8 7.6z" />
  ),
  desktopTile: (
    <>
      <rect x="3" y="3.4" width="14" height="9.6" rx="1.4" />
      <path d="M7.4 16.4h5.2" />
    </>
  ),
  zoom: (
    <>
      <circle cx="9" cy="9" r="5.2" />
      <path d="M12.8 12.8 17 17M6.6 9h4.8M9 6.6v4.8" />
    </>
  ),
  filter: (
    <path d="M3.6 5.2h12.8l-4.9 5.9v4.4l-3 1.4v-5.8z" />
  ),
  network: (
    <>
      <circle cx="10" cy="10" r="6.6" />
      <path d="M3.4 10h13.2" />
      <path d="M10 3.4c1.9 2 1.9 11.2 0 13.2M10 3.4c-1.9 2-1.9 11.2 0 13.2" />
    </>
  ),
  gallery: (
    <>
      <rect x="2.6" y="5.4" width="11.4" height="9" rx="1.6" />
      <path d="M6 3.6h8.8a2 2 0 0 1 2 2v7.6" />
      <circle cx="6.6" cy="8.8" r="1.1" />
      <path d="M3.4 14.4 7 11l2.4 2 1.8-1.5 2.6 2.4" />
    </>
  ),
} as const;

export type IconName = keyof typeof GLYPHS;

export function Icon({
  name,
  size = 16,
  strokeWidth = 1.5,
  className,
}: {
  name: IconName;
  size?: number;
  strokeWidth?: number;
  className?: string;
}) {
  const glyph: ReactNode = GLYPHS[name];
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      {glyph}
    </svg>
  );
}
