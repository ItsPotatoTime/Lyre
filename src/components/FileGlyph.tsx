import type { EntryKind } from "@/types";

/**
 * File-type glyphs.
 *
 * Explorer gives each family its own full-colour icon, and that colour is what
 * makes a folder scannable. These are drawn the same way: a solid page in the
 * family's hue with the family's mark knocked out in white, so at 16px the
 * silhouette reads before the label does.
 */

const FILL: Record<EntryKind, string> = {
  folder: "var(--glyph-folder)",
  file: "var(--glyph-file)",
  image: "var(--glyph-image)",
  video: "var(--glyph-video)",
  audio: "var(--glyph-audio)",
  archive: "var(--glyph-archive)",
  pdf: "var(--glyph-pdf)",
  document: "var(--glyph-document)",
  spreadsheet: "var(--glyph-spreadsheet)",
  presentation: "var(--glyph-presentation)",
  text: "var(--glyph-text)",
  code: "var(--glyph-code)",
  executable: "var(--glyph-executable)",
  font: "var(--glyph-font)",
  database: "var(--glyph-database)",
  binary: "var(--glyph-binary)",
  shortcut: "var(--glyph-shortcut)",
};

const MARKS: Partial<Record<EntryKind, React.ReactNode>> = {
  image: (
    <>
      <circle cx="8.2" cy="10.3" r="1.15" />
      <path d="M5.9 15.5 8.9 12.2l1.9 1.8 1.5-1.3 2.1 2.3" />
    </>
  ),
  video: <path d="M8.2 9.6 12.4 12l-4.2 2.4z" />,
  audio: (
    <>
      <circle cx="8.2" cy="14.2" r="1.5" />
      <path d="M9.7 14.2V8.5l3.3-.9v4.5" />
      <circle cx="11.6" cy="12.1" r="1.5" />
    </>
  ),
  archive: <path d="M8.8 9.4h2.4M8.8 11.6h2.4M8.8 13.8h2.4" />,
  pdf: <path d="M7.4 10.6h5.2M7.4 12.9h5.2M7.4 15.2h3.1" />,
  document: <path d="M7.4 10.4h5.2M7.4 12.7h5.2M7.4 15h3.6" />,
  spreadsheet: (
    <>
      <path d="M7.1 10.2h5.8v5.1H7.1z" />
      <path d="M7.1 12.8h5.8M10 10.2v5.1" />
    </>
  ),
  presentation: (
    <>
      <path d="M7.2 10.3h5.6v3.7H7.2z" />
      <path d="M10 14v1.7M8.4 15.7h3.2" />
    </>
  ),
  code: <path d="M8.7 10.4 7 12.4l1.7 2M11.3 10.4 13 12.4l-1.7 2" />,
  text: <path d="M7.4 10.4h5.2M7.4 12.7h5.2M7.4 15h3.6" />,
  executable: (
    <>
      <path d="M6.7 9.7h6.6v5.6H6.7z" />
      <path d="m8.5 11.4 1.3 1.2-1.3 1.2M11 13.8h1.4" />
    </>
  ),
  font: <path d="M8 15.3V9.6L10 8.2l2 1.4v5.7M8.7 12.6h2.6" />,
  database: (
    <>
      <ellipse cx="10" cy="10.1" rx="2.8" ry="1.15" />
      <path d="M7.2 10.1v4.4c0 .64 1.25 1.15 2.8 1.15s2.8-.51 2.8-1.15v-4.4" />
    </>
  ),
  binary: (
    <>
      <circle cx="8.4" cy="10.6" r=".95" />
      <circle cx="11.6" cy="10.6" r=".95" />
      <circle cx="8.4" cy="13.6" r=".95" />
      <circle cx="11.6" cy="13.6" r=".95" />
    </>
  ),
  shortcut: <path d="M11.9 10 8.2 13.7M8.2 10.4v3.3h3.3" />,
  file: <path d="M7.4 10.6h5.2M7.4 13h5.2" />,
};

export function FileGlyph({
  kind,
  size = 16,
  isSymlink = false,
}: {
  kind: EntryKind;
  size?: number;
  isSymlink?: boolean;
}) {
  const fill = FILL[kind] ?? FILL.file;

  if (kind === "folder") {
    return (
      <svg
        width={size}
        height={size}
        viewBox="0 0 20 20"
        aria-hidden="true"
        focusable="false"
        style={{ display: "block", flex: "none" }}
      >
        <path
          d="M2.2 6.3a1.9 1.9 0 0 1 1.9-1.9h3.3l1.7 2.1h6.8a1.9 1.9 0 0 1 1.9 1.9v6.5a1.9 1.9 0 0 1-1.9 1.9H4.1a1.9 1.9 0 0 1-1.9-1.9z"
          fill="var(--glyph-folder)"
        />
        <path
          d="M2.2 9.5h15.6v5.4a1.9 1.9 0 0 1-1.9 1.9H4.1a1.9 1.9 0 0 1-1.9-1.9z"
          fill="var(--glyph-folder-edge)"
          opacity=".55"
        />
      </svg>
    );
  }

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      aria-hidden="true"
      focusable="false"
      style={{ display: "block", flex: "none" }}
    >
      <path
        d="M4.6 2.9h6.6l3.9 3.9v10.3H4.6z"
        fill={fill}
        stroke="color-mix(in srgb, #000 14%, transparent)"
        strokeWidth=".5"
        strokeLinejoin="round"
      />
      <path d="M11.2 2.9v3.9h3.9" fill="rgba(255,255,255,.42)" />
      <g
        fill="none"
        stroke="#fff"
        strokeWidth="1.15"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity=".95"
      >
        {MARKS[kind] ?? MARKS.file}
      </g>
      {isSymlink ? (
        <path
          d="M3.1 15.9h2.6"
          stroke="#fff"
          strokeWidth="1.4"
          strokeLinecap="round"
          opacity=".85"
        />
      ) : null}
    </svg>
  );
}

/** Nav rail and place glyphs, matched to the rail's 16px optical size. */
export function PlaceGlyph({
  icon,
  size = 16,
}: {
  icon: string;
  size?: number;
}) {
  const fill =
    icon === "download"
      ? "var(--glyph-archive)"
      : icon === "image"
        ? "var(--glyph-image)"
        : icon === "audio"
          ? "var(--glyph-audio)"
          : icon === "video"
            ? "var(--glyph-video)"
            : icon === "document"
              ? "var(--glyph-document)"
              : icon === "drive"
                ? "var(--glyph-executable)"
                : "var(--glyph-folder)";

  if (icon === "drive") {
    return (
      <svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
        <rect x="2.2" y="5.4" width="15.6" height="9.2" rx="2" fill={fill} />
        <rect x="2.2" y="5.4" width="15.6" height="4" rx="2" fill="#fff" opacity=".18" />
        <circle cx="6" cy="12" r="1" fill="#fff" opacity=".9" />
      </svg>
    );
  }
  if (icon === "home") {
    return (
      <svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
        <path
          d="M3.2 9 10 2.9 16.8 9v7.1a1.4 1.4 0 0 1-1.4 1.4H4.6a1.4 1.4 0 0 1-1.4-1.4z"
          fill={fill}
        />
        <path d="M8.2 17.5v-5h3.6v5" fill="#fff" opacity=".38" />
      </svg>
    );
  }
  if (icon === "desktop") {
    return (
      <svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
        <rect x="2.2" y="4" width="15.6" height="10" rx="1.6" fill={fill} />
        <path d="M6.6 16.8h6.8" stroke={fill} strokeWidth="1.6" strokeLinecap="round" />
      </svg>
    );
  }
  // A document-family place: page silhouette in the family colour.
  return (
    <svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
      <path d="M5 2.6h6.4L15 6.2v11.2H5z" fill={fill} />
      <path d="M11.4 2.6v3.6H15" fill="rgba(255,255,255,.42)" />
      <path
        d="M7.6 9.2h5M7.6 11.8h5M7.6 14.4h3"
        stroke="#fff"
        strokeWidth="1.1"
        strokeLinecap="round"
        opacity=".85"
      />
    </svg>
  );
}
