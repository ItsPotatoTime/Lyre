import type { ColumnWidths, SortState, ViewMode } from "@/types";
import type { Mode } from "./modes";
import { DEFAULT_COLUMNS } from "./modes";

/**
 * The parts of the view a person expects the window to remember: how the list
 * is sorted, how it is drawn, and how the chrome is arranged.
 *
 * Kept in localStorage rather than in Rust because it is interface state, and
 * because a corrupt value must never stop the app opening: every read is
 * defensive and every field falls back on its own.
 */
export interface Prefs {
  sort: SortState;
  view: ViewMode;
  groupByDate: boolean;
  mode: Mode;
  showHidden: boolean;
  /**
   * Whether to list the entries the system marks as its own. Off by default,
   * so a folder shows what the desktop's own file manager shows; on, the
   * window lists everything on the volume.
   */
  showProtected: boolean;
  navOpen: boolean;
  navWidth: number;
  columns: ColumnWidths;
  /** Whether the default-file-manager offer may appear at startup. */
  promptDefaultManager: boolean;
  /** Whether the preview pane is shown. On by default. */
  previewOpen: boolean;
}

const KEY = "lyre.prefs.v1";

export const DEFAULT_PREFS: Prefs = {
  sort: { key: "name", dir: 1 },
  view: "details",
  groupByDate: false,
  mode: "explorer",
  showHidden: false,
  showProtected: false,
  navOpen: true,
  navWidth: 160,
  columns: DEFAULT_COLUMNS,
  promptDefaultManager: true,
  previewOpen: true,
};

const SORT_KEYS = new Set(["name", "modified", "type", "size"]);

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    // A hardened webview can deny storage outright; the app still has to run.
    return null;
  }
}

export function loadPrefs(): Prefs {
  const store = storage();
  if (!store) return DEFAULT_PREFS;

  let raw: unknown;
  try {
    const text = store.getItem(KEY);
    if (!text) return DEFAULT_PREFS;
    raw = JSON.parse(text);
  } catch {
    return DEFAULT_PREFS;
  }

  const saved = (raw ?? {}) as Partial<Prefs>;
  const sort = saved.sort;
  const sortValid =
    sort &&
    SORT_KEYS.has(sort.key) &&
    (sort.dir === 1 || sort.dir === -1);

  const columns = saved.columns;
  const columnsValid =
    Array.isArray(columns) &&
    columns.length === 4 &&
    columns.every((width) => typeof width === "number" && width >= 40);

  return {
    sort: sortValid ? { key: sort.key, dir: sort.dir } : DEFAULT_PREFS.sort,
    view: saved.view === "grid" || saved.view === "details" ? saved.view : DEFAULT_PREFS.view,
    groupByDate: typeof saved.groupByDate === "boolean" ? saved.groupByDate : DEFAULT_PREFS.groupByDate,
    mode: saved.mode === "compact" ? "compact" : "explorer",
    showHidden: typeof saved.showHidden === "boolean" ? saved.showHidden : DEFAULT_PREFS.showHidden,
    showProtected:
      typeof saved.showProtected === "boolean"
        ? saved.showProtected
        : DEFAULT_PREFS.showProtected,
    navOpen: typeof saved.navOpen === "boolean" ? saved.navOpen : DEFAULT_PREFS.navOpen,
    navWidth:
      typeof saved.navWidth === "number" && saved.navWidth >= 140 && saved.navWidth <= 360
        ? saved.navWidth
        : DEFAULT_PREFS.navWidth,
    columns: columnsValid ? (columns as ColumnWidths) : DEFAULT_COLUMNS,
    promptDefaultManager:
      typeof saved.promptDefaultManager === "boolean"
        ? saved.promptDefaultManager
        : true,
    previewOpen:
      typeof saved.previewOpen === "boolean" ? saved.previewOpen : true,
  };
}

export function savePrefs(prefs: Prefs): void {
  const store = storage();
  if (!store) return;
  try {
    store.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Quota or a read-only profile: remembering the view is not worth an error.
  }
}
