/** Shapes shared with the Rust core. Field names match its serde output. */

export type EntryKind =
  | "folder"
  | "file"
  | "image"
  | "video"
  | "audio"
  | "archive"
  | "pdf"
  | "document"
  | "spreadsheet"
  | "presentation"
  | "text"
  | "code"
  | "executable"
  | "font"
  | "database"
  | "binary"
  | "shortcut";

export interface Entry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number;
  modifiedMs: number | null;
  kind: EntryKind;
  ext: string;
  hidden: boolean;
  /**
   * Belongs to the system rather than to the person using it, which is what
   * the desktop's own file manager hides even when it is showing hidden
   * files. Only Windows marks entries this way.
   */
  protected: boolean;
  readable: boolean;
}

export type ScanEvent =
  | { type: "batch"; entries: Entry[]; scanned: number }
  | { type: "done"; total: number; elapsedMs: number }
  | { type: "failed"; message: string };

export type PlaceKind = "folder" | "drive" | "removable";

export interface Place {
  name: string;
  path: string;
  kind: PlaceKind;
  icon: string;
  /** The device behind the volume: `/dev/sda1`, or `C:` on Windows. */
  detail: string | null;
  /** Volume size in bytes; 0 for folders. */
  total: number;
  /** Free space in bytes; 0 for folders. */
  free: number;
}

export interface PathInfo {
  path: string;
  name: string;
  parent: string | null;
  segments: { name: string; path: string }[];
  exists: boolean;
  isDir: boolean;
}

/**
 * What a tab is pointed at. Explorer distinguishes real folders from its Home
 * page, the This PC page, and Gallery, and so does this: three of those have no
 * directory behind them to read.
 */
export type LocationKind = "folder" | "home" | "thisPc" | "gallery";

export interface Location {
  kind: LocationKind;
  path: string;
}

export type SortKey = "name" | "modified" | "type" | "size";

export interface SortState {
  key: SortKey;
  dir: 1 | -1;
}

export type ViewMode = "details" | "grid";

/** Content filter from the command bar's Filter menu. */
export type KindFilter =
  | "all"
  | "folder"
  | "document"
  | "image"
  | "video"
  | "audio"
  | "archive"
  | "code";

export interface Tab {
  id: string;
  location: Location;
  history: Location[];
  index: number;
  sort: SortState;
  view: ViewMode;
  filter: KindFilter;
  groupByDate: boolean;
}

export interface Selection {
  selected: Set<string>;
  focused: string | null;
  anchor: string | null;
}

/** Column widths in px, in visual order: name, modified, type, size. */
export type ColumnWidths = [number, number, number, number];

/** One search result: the row plus its relevance and match positions. */
export interface Hit extends Entry {
  score: number;
  /** Character offsets of the matched characters, for highlighting. */
  indices: number[];
}

export type SearchPhase = "idle" | "local" | "walking" | "done" | "error";

export interface SearchState {
  items: Hit[];
  /** Total matches found, which can exceed the number of items returned. */
  matched: number;
  /** Directory entries examined by the recursive phase. */
  scanned: number;
  elapsedMs: number | null;
  truncated: boolean;
  phase: SearchPhase;
  error: string | null;
}
