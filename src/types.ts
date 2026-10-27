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
  readable: boolean;
}

export type ScanEvent =
  | { type: "batch"; entries: Entry[]; scanned: number }
  | { type: "done"; total: number; elapsedMs: number }
  | { type: "failed"; message: string };

export type Place = {
  name: string;
  path: string;
  kind: "folder" | "drive" | "removable";
  icon: string;
  detail: string | null;
};

export interface PathInfo {
  path: string;
  name: string;
  parent: string | null;
  segments: { name: string; path: string }[];
  exists: boolean;
  isDir: boolean;
}

export type SortKey = "name" | "modified" | "type" | "size";

export interface SortState {
  key: SortKey;
  dir: 1 | -1;
}

export type ViewMode = "details" | "grid";

export interface Tab {
  id: string;
  path: string;
  history: string[];
  index: number;
  sort: SortState;
  view: ViewMode;
}

export interface Selection {
  selected: Set<string>;
  focused: string | null;
  anchor: string | null;
}
