import { useEffect, useRef, useState } from "react";
import type { Hit, SearchState } from "@/types";
import { searchDir, type ScanHandle } from "./ipc";

const EMPTY: SearchState = {
  items: [],
  matched: 0,
  scanned: 0,
  elapsedMs: null,
  truncated: false,
  phase: "idle",
  error: null,
};

/**
 * Wait this long after the last keystroke before starting a search.
 *
 * Long enough that a normal typing rhythm produces one search instead of one
 * per letter, short enough that a single pause feels instant. Combined with
 * keeping the previous results on screen, nothing flickers between keys.
 */
const DEBOUNCE_MS = 60;

/**
 * Run a search as the query changes.
 *
 * Two things keep this calm on the eyes. The previous results stay on screen
 * until the new ones are ready, so the list never blanks between keystrokes,
 * and the search that a keystroke replaces is cancelled immediately, so no work
 * is queued that cannot be shown.
 */
export function useSearch(
  path: string,
  roots: string[],
  query: string,
  showHidden: boolean,
  enabled: boolean,
): SearchState {
  const [state, setState] = useState<SearchState>(EMPTY);
  const handle = useRef<ScanHandle | null>(null);
  const lastPath = useRef(path);
  // Roots can nest and the walk can meet the same folder twice; the list is
  // keyed by path, so one path must appear once.
  const seen = useRef(new Set<string>());
  // Arrays are new objects on every render; the joined key is what the effect
  // actually depends on.
  const rootsKey = roots.join("\u0000");
  const rootsRef = useRef(roots);
  rootsRef.current = roots;

  useEffect(() => {
    const needle = query.trim();
    const where = path === "" ? rootsRef.current : [path];

    if (!enabled || needle.length === 0 || where.length === 0) {
      handle.current?.cancel();
      handle.current = null;
      lastPath.current = path;
      setState(EMPTY);
      return;
    }

    // Moving to another folder invalidates what is on screen; a new query in
    // the same folder does not, so the rows stay put while the new search runs.
    if (lastPath.current !== path) {
      lastPath.current = path;
      setState(EMPTY);
    }

    let disposed = false;
    let running: ScanHandle | null = null;
    setState((prev) => ({ ...prev, phase: "local", error: null }));

    const timer = window.setTimeout(() => {
      void searchDir(path, where, needle, true, showHidden, {
        onLocal: (items, matched, elapsedMs) => {
          if (disposed) return;
          seen.current = new Set(items.map((item) => item.path));
          setState({
            items,
            matched,
            scanned: 0,
            elapsedMs,
            truncated: matched > items.length,
            phase: "local",
            error: null,
          });
        },
        onBatch: (batch, scanned) => {
          if (disposed) return;
          const fresh = batch.filter((item) => !seen.current.has(item.path));
          if (fresh.length === 0) return;
          for (const item of fresh) seen.current.add(item.path);
          setState((prev) => ({
            ...prev,
            items: prev.items.concat(fresh),
            scanned,
            phase: "walking",
          }));
        },
        onDone: (matched, scanned, elapsedMs, truncated) => {
          if (disposed) return;
          setState((prev) => ({
            ...prev,
            matched,
            scanned,
            elapsedMs,
            truncated,
            phase: "done",
          }));
        },
        onFailed: (message) => {
          if (disposed) return;
          setState({ ...EMPTY, phase: "error", error: message });
        },
      }).then((created) => {
        if (disposed) created.cancel();
        else {
          running = created;
          handle.current = created;
        }
      });
    }, DEBOUNCE_MS);

    return () => {
      disposed = true;
      window.clearTimeout(timer);
      running?.cancel();
      handle.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, rootsKey, query, showHidden, enabled]);

  return state;
}

/** Match positions keyed by path, for the renderer. */
export function matchIndex(items: Hit[]): Map<string, number[]> {
  const map = new Map<string, number[]>();
  for (const hit of items) {
    if (hit.indices.length > 0) map.set(hit.path, hit.indices);
  }
  return map;
}

/**
 * Stable identity for a roots list, so an effect can depend on its contents
 * rather than on the array object.
 */
export function rootsKeyOf(roots: string[]): string {
  return roots.join("\u0000");
}
