import type { Place } from "@/types";

/**
 * What the person has decided about their own folders.
 *
 * Favorites live on the Quick access page; pins live in the rail. They are
 * separate lists on purpose: a folder can be worth reaching from the landing
 * page without taking a permanent row in the sidebar, and the built-in rows
 * (Documents, Pictures, …) can be removed without that meaning anything about
 * the favorites shelf.
 */
export interface Library {
  /** Paths shown in the Favorites section of Quick access. */
  favorites: string[];
  /** Built-in rail rows the person removed. */
  hiddenPins: string[];
  /** Rail rows the person added. */
  extraPins: string[];
}

export const EMPTY_LIBRARY: Library = {
  favorites: [],
  hiddenPins: [],
  extraPins: [],
};

const KEY = "lyre.library.v1";

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

export function loadLibrary(): Library {
  try {
    const text = window.localStorage.getItem(KEY);
    if (!text) return EMPTY_LIBRARY;
    const raw = JSON.parse(text) as Partial<Library>;
    return {
      favorites: strings(raw.favorites),
      hiddenPins: strings(raw.hiddenPins),
      extraPins: strings(raw.extraPins),
    };
  } catch {
    // A corrupt value must never stop the window opening.
    return EMPTY_LIBRARY;
  }
}

export function saveLibrary(library: Library): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(library));
  } catch {
    // Quota or a read-only profile: not worth an error.
  }
}

/** The last path segment, which is what a folder is called. */
export function placeName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** A place-like row for a path the person added, which has no scan behind it. */
export function placeForPath(path: string): Place {
  return {
    name: placeName(path),
    path,
    kind: "folder",
    icon: "folder",
    detail: null,
    total: 0,
    free: 0,
  };
}
