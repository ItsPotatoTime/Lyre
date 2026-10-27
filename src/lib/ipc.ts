import type { Entry, Hit, PathInfo, Place, ScanEvent } from "@/types";

/**
 * The interface talks to this module rather than to `invoke` directly, so the
 * same screens can run in a plain browser during design work and in tests.
 * Everything below has one implementation per environment and no branching
 * inside the components.
 */

export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface ScanHandlers {
  onBatch: (entries: Entry[], scanned: number) => void;
  onDone: (total: number, elapsedMs: number) => void;
  onFailed: (message: string) => void;
}

export interface ScanHandle {
  cancel: () => void;
}

export async function scanDir(
  path: string,
  showHidden: boolean,
  handlers: ScanHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    const { mockScan } = await import("./mock");
    return mockScan(path, showHidden, handlers);
  }

  const { Channel, invoke } = await import("@tauri-apps/api/core");
  const channel = new Channel<ScanEvent>();
  let jobId: number | null = null;
  let cancelled = false;

  channel.onmessage = (event) => {
    if (cancelled) return;
    switch (event.type) {
      case "batch":
        handlers.onBatch(event.entries, event.scanned);
        break;
      case "done":
        handlers.onDone(event.total, event.elapsedMs);
        break;
      case "failed":
        handlers.onFailed(event.message);
        break;
    }
  };

  try {
    jobId = await invoke<number>("scan_dir", {
      request: { path, showHidden, },
      onEvent: channel,
    });
  } catch (error) {
    handlers.onFailed(describe(error));
  }

  return {
    cancel: () => {
      cancelled = true;
      if (jobId !== null) {
        void invoke("cancel_job", { id: jobId });
        void invoke("release_job", { id: jobId });
      }
    },
  };
}

export interface SearchHandlers {
  onLocal: (items: Hit[], matched: number, elapsedMs: number) => void;
  onBatch: (items: Hit[], scanned: number) => void;
  onDone: (
    matched: number,
    scanned: number,
    elapsedMs: number,
    truncated: boolean,
  ) => void;
  onFailed: (message: string) => void;
}

/**
 * Search a folder, and optionally everything below it.
 *
 * Results arrive in two waves: the folder on screen first, from memory, then
 * matches from deeper in the tree as the walk finds them.
 */
export async function searchDir(
  path: string,
  query: string,
  recursive: boolean,
  showHidden: boolean,
  handlers: SearchHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    const { mockSearch } = await import("./mock");
    return mockSearch(path, query, recursive, showHidden, handlers);
  }

  const { Channel, invoke } = await import("@tauri-apps/api/core");
  const channel = new Channel<SearchEventWire>();
  let jobId: number | null = null;
  let cancelled = false;

  channel.onmessage = (event) => {
    if (cancelled) return;
    switch (event.type) {
      case "local":
        handlers.onLocal(event.items, event.matched, event.elapsedMs);
        break;
      case "batch":
        handlers.onBatch(event.items, event.scanned);
        break;
      case "done":
        handlers.onDone(
          event.matched,
          event.scanned,
          event.elapsedMs,
          event.truncated,
        );
        break;
      case "failed":
        handlers.onFailed(event.message);
        break;
    }
  };

  try {
    jobId = await invoke<number>("search_dir", {
      request: { path, query, recursive, showHidden },
      onEvent: channel,
    });
  } catch (error) {
    handlers.onFailed(describe(error));
  }

  return {
    cancel: () => {
      cancelled = true;
      if (jobId !== null) {
        void invoke("cancel_job", { id: jobId });
        void invoke("release_job", { id: jobId });
      }
    },
  };
}

type SearchEventWire =
  | { type: "local"; items: Hit[]; matched: number; elapsedMs: number }
  | { type: "batch"; items: Hit[]; scanned: number }
  | {
      type: "done";
      matched: number;
      scanned: number;
      elapsedMs: number;
      truncated: boolean;
    }
  | { type: "failed"; message: string };

export async function getPlaces(): Promise<Place[]> {
  if (!isTauri) {
    const { mockPlaces } = await import("./mock");
    return mockPlaces();
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<Place[]>("places");
}

export async function getPathInfo(path: string): Promise<PathInfo> {
  if (!isTauri) {
    const { mockPathInfo } = await import("./mock");
    return mockPathInfo(path);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<PathInfo>("path_info", { path });
}

export async function getHomeDir(): Promise<string> {
  if (!isTauri) {
    const { mockHome } = await import("./mock");
    return mockHome();
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("home_dir");
}

/** Directory mtime, used to decide whether a cached listing is stale. */
export async function getDirStamp(path: string): Promise<number | null> {
  if (!isTauri) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<number | null>("dir_stamp", { path });
}

/**
 * Hand a file to whatever the operating system opens that type with.
 * Directories are navigated instead, so this is only ever called for files.
 */
export async function openPath(path: string): Promise<void> {
  if (!isTauri) {
    throw new Error("Opening files needs the desktop app, not the browser preview.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_path", { path });
}

function describe(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "The folder could not be read.";
}
