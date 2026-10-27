import { useEffect, useRef, useState } from "react";
import type { Entry, SortState } from "@/types";
import { scanDir, type ScanHandle } from "./ipc";
import { compareEntries, mergeSorted } from "./sort";

export type DirStatus = "loading" | "ready" | "error";

export interface DirectoryState {
  entries: Entry[];
  version: number;
  status: DirStatus;
  error: string | null;
  scanned: number;
  total: number;
  elapsedMs: number | null;
}

/**
 * Streams a directory into a kept-sorted array.
 *
 * The rows are held in a ref rather than in state on purpose. A large scan
 * produces a batch every few milliseconds, and copying a 50,000 element array
 * through React state on each one would cost more than the scan itself. The
 * ref is the render source of truth and `version` is the change signal.
 *
 * Batches are merged, not re-sorted: each arrival adds a few hundred rows to
 * an already ordered array, which is one linear pass. The only full sort
 * happens when the user changes the sort order.
 */
export function useDirectory(
  path: string,
  showHidden: boolean,
  sort: SortState,
  /** Bumped by the interface to force a re-read of the same path. */
  nonce = 0,
): DirectoryState {
  const entriesRef = useRef<Entry[]>([]);
  const pendingRef = useRef<Entry[]>([]);
  const versionRef = useRef(0);
  const sortRef = useRef(sort);
  sortRef.current = sort;

  const [state, setState] = useState<DirectoryState>({
    entries: entriesRef.current,
    version: 0,
    status: "loading",
    error: null,
    scanned: 0,
    total: 0,
    elapsedMs: null,
  });

  useEffect(() => {
    let disposed = false;
    let handle: ScanHandle | null = null;
    let frame = 0;

    entriesRef.current = [];
    pendingRef.current = [];
    versionRef.current = 0;
    setState({
      entries: entriesRef.current,
      version: 0,
      status: "loading",
      error: null,
      scanned: 0,
      total: 0,
      elapsedMs: null,
    });

    const flush = () => {
      frame = 0;
      if (disposed) return;
      const pending = pendingRef.current;
      if (pending.length === 0) return;
      pendingRef.current = [];
      const order = sortRef.current;
      pending.sort((a, b) => compareEntries(a, b, order));
      entriesRef.current = mergeSorted(entriesRef.current, pending, order);
      versionRef.current += 1;
      setState((prev) => ({
        ...prev,
        entries: entriesRef.current,
        version: versionRef.current,
      }));
    };

    const schedule = () => {
      if (frame === 0) frame = requestAnimationFrame(flush);
    };

    void scanDir(path, showHidden, {
      onBatch: (entries, scanned) => {
        if (disposed) return;
        const pending = pendingRef.current;
        for (let i = 0; i < entries.length; i += 1) pending.push(entries[i]!);
        setState((prev) =>
          prev.scanned === scanned ? prev : { ...prev, scanned },
        );
        schedule();
      },
      onDone: (total, elapsedMs) => {
        if (disposed) return;
        flush();
        setState((prev) => ({
          ...prev,
          entries: entriesRef.current,
          version: versionRef.current,
          status: "ready",
          total,
          elapsedMs,
        }));
      },
      onFailed: (message) => {
        if (disposed) return;
        setState((prev) => ({
          ...prev,
          status: "error",
          error: message,
          entries: entriesRef.current,
          version: versionRef.current,
        }));
      },
    }).then((created) => {
      if (disposed) created.cancel();
      else handle = created;
    });

    return () => {
      disposed = true;
      if (frame !== 0) cancelAnimationFrame(frame);
      handle?.cancel();
    };
    // `sort` is intentionally absent: changing it must not restart the scan.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, showHidden, nonce]);

  // Re-sort what is already on screen when the comparator changes, and bump the
  // version so the change is published.
  const previousSort = useRef(sort);
  useEffect(() => {
    const previous = previousSort.current;
    if (previous.key === sort.key && previous.dir === sort.dir) return;
    previousSort.current = sort;
    entriesRef.current.sort((a, b) => compareEntries(a, b, sort));
    versionRef.current += 1;
    setState((prev) => ({
      ...prev,
      entries: entriesRef.current,
      version: versionRef.current,
    }));
  }, [sort]);

  return state;
}
