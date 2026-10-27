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

/** Wait this long after the last keystroke before starting a search. Short
 *  enough to feel immediate, long enough that typing a word starts one search
 *  instead of one per letter. */
const DEBOUNCE_MS = 35;

/**
 * Run a search as the query changes.
 *
 * The previous search is cancelled the moment the query moves, so a fast typist
 * never queues work that cannot be shown, and a walk over a large tree never
 * outlives the query that started it.
 */
export function useSearch(
  path: string,
  query: string,
  showHidden: boolean,
  enabled: boolean,
): SearchState {
  const [state, setState] = useState<SearchState>(EMPTY);
  const handle = useRef<ScanHandle | null>(null);

  useEffect(() => {
    const needle = query.trim();
    if (!enabled || needle.length === 0) {
      handle.current?.cancel();
      handle.current = null;
      setState(EMPTY);
      return;
    }

    let disposed = false;
    let running: ScanHandle | null = null;
    setState({ ...EMPTY, phase: "local" });

    const timer = window.setTimeout(() => {
      void searchDir(path, needle, true, showHidden, {
        onLocal: (items, matched, elapsedMs) => {
          if (disposed) return;
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
          setState((prev) => ({
            ...prev,
            items: prev.items.concat(batch),
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
  }, [path, query, showHidden, enabled]);

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
