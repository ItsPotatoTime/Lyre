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
  roots: string[],
  query: string,
  recursive: boolean,
  showHidden: boolean,
  handlers: SearchHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    const { mockSearch } = await import("./mock");
    return mockSearch(path, roots, query, recursive, showHidden, handlers);
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
      request: { path, roots, query, recursive, showHidden },
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

/** Create a folder and return the path it actually used. */
export async function createFolder(parent: string, name: string): Promise<string> {
  if (!isTauri) {
    throw new Error("Creating folders needs the desktop app, not the browser preview.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("create_folder", { parent, name });
}

/** Rename an item and return its new path. */
export async function renamePath(path: string, name: string): Promise<string> {
  if (!isTauri) {
    throw new Error("Renaming needs the desktop app, not the browser preview.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("rename_path", { path, name });
}

export interface PathProperties {
  name: string;
  path: string;
  parent: string | null;
  isDir: boolean;
  size: number | null;
  items: number | null;
  modifiedMs: number | null;
  readonly: boolean;
}

export async function pathProperties(path: string): Promise<PathProperties> {
  if (!isTauri) {
    throw new Error("Properties need the desktop app, not the browser preview.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<PathProperties>("path_properties", { path });
}

export interface PasteHandlers {
  onProgress: (done: number, total: number, name: string) => void;
  onDone: (pasted: number, skipped: string[]) => void;
  onFailed: (message: string) => void;
}

/**
 * Paste copied or cut entries into a folder.
 *
 * The paste never overwrites: a name that already exists gets a numbered
 * sibling. Progress arrives per item, so a large folder shows movement.
 */
export async function pasteInto(
  destination: string,
  sources: string[],
  mode: "copy" | "cut",
  handlers: PasteHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    throw new Error("Clipboard operations need the desktop app, not the browser preview.");
  }
  const { Channel, invoke } = await import("@tauri-apps/api/core");
  const channel = new Channel<
    | { type: "progress"; done: number; total: number; name: string }
    | { type: "done"; pasted: number; skipped: string[] }
    | { type: "failed"; message: string }
  >();
  let jobId: number | null = null;
  let cancelled = false;

  channel.onmessage = (event) => {
    if (cancelled) return;
    if (event.type === "progress") {
      handlers.onProgress(event.done, event.total, event.name);
    } else if (event.type === "done") {
      handlers.onDone(event.pasted, event.skipped);
    } else {
      handlers.onFailed(event.message);
    }
  };

  try {
    jobId = await invoke<number>("paste_into", {
      destination,
      sources,
      mode,
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

/** Move entries to the operating system's trash. Returns how many went. */
export async function deleteToTrash(paths: string[]): Promise<number> {
  if (!isTauri) {
    throw new Error("Deleting needs the desktop app, not the browser preview.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<number>("delete_to_trash", { paths });
}

export interface ManagerState {
  current: string | null;
  isDefault: boolean;
  /** Set when this platform cannot be asked to change the folder handler. */
  unsupported: string | null;
  desktopFile: string | null;
}

export async function fileManagerState(): Promise<ManagerState> {
  if (!isTauri) {
    return {
      current: null,
      isDefault: false,
      unsupported: "Making Lyre the folder handler needs the desktop app.",
      desktopFile: null,
    };
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ManagerState>("file_manager_state");
}

export async function becomeFileManager(): Promise<ManagerState> {
  if (!isTauri) throw new Error("Needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ManagerState>("become_file_manager");
}

/** The folder this process was launched with, when a desktop opened one. */
export async function startupFolder(): Promise<string | null> {
  if (!isTauri) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string | null>("startup_folder");
}

export interface OpenWith {
  mime: string;
  defaultApp: string | null;
  apps: { id: string; name: string }[];
  /** Set when this platform cannot answer, with the reason. */
  unsupported: string | null;
  /**
   * True where the platform protects its application list and defaults, so the
   * system's own picker has to be used. Windows is this case.
   */
  systemPicker: boolean;
}

/** Ask the operating system to choose an application and set the default. */
export async function chooseAppWithSystem(path: string): Promise<void> {
  if (!isTauri) {
    throw new Error("Choosing an application needs the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("choose_app_with_system", { path });
}

export async function appsForFile(path: string): Promise<OpenWith> {
  if (!isTauri) {
    throw new Error("Choosing an application needs the desktop app, not the browser preview.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<OpenWith>("apps_for_file", { path });
}

export async function openWithApp(path: string, appId: string): Promise<void> {
  if (!isTauri) throw new Error("Needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_with_app", { path, appId });
}

export async function setDefaultApp(path: string, appId: string): Promise<string> {
  if (!isTauri) throw new Error("Needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string>("set_default_app", { path, appId });
}

function describe(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "The folder could not be read.";
}
