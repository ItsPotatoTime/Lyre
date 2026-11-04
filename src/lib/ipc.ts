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
  showProtected: boolean,
  handlers: ScanHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    const { mockScan } = await import("./mock");
    return mockScan(path, showHidden, showProtected, handlers);
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
      request: { path, showHidden, showProtected },
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
  showProtected: boolean,
  handlers: SearchHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    const { mockSearch } = await import("./mock");
    return mockSearch(
      path,
      roots,
      query,
      recursive,
      showHidden,
      showProtected,
      handlers,
    );
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
      request: { path, roots, query, recursive, showHidden, showProtected },
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

/**
 * Watch the folder the window is showing, so files changed by another program
 * appear without a manual refresh. Passing `null` stops watching on Home and
 * This PC, which have no single folder behind them. Watching continues during
 * a search so clearing the query does not reveal a stale listing.
 */
export async function watchFolder(path: string | null): Promise<void> {
  if (!isTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("watch_folder", { path });
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
  /**
   * How far the paste has got: the item being worked on, the items finished,
   * and the bytes written out of the bytes there are. The byte counters are what
   * move during a single large file.
   */
  onProgress: (
    done: number,
    total: number,
    name: string,
    doneBytes: number,
    totalBytes: number,
  ) => void;
  onDone: (pasted: number, skipped: string[], warnings: string[]) => void;
  onFailed: (message: string) => void;
}

/**
 * Paste copied or cut entries into a folder.
 *
 * The paste never overwrites: a name that already exists gets a numbered
 * sibling. Progress arrives per item, so a large folder shows movement.
 */
/** What to do when the destination already holds an item of the same name. */
export type Conflict = "rename" | "overwrite" | "skip";

/** Which of these items already exist in the destination. */
export async function pasteConflicts(
  destination: string,
  sources: string[],
): Promise<string[]> {
  if (!isTauri) return [];
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string[]>("paste_conflicts", { destination, sources });
}

export async function pasteInto(
  destination: string,
  sources: string[],
  mode: "copy" | "cut",
  conflict: Conflict,
  handlers: PasteHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    throw new Error("Clipboard operations need the desktop app, not the browser preview.");
  }
  const { Channel, invoke } = await import("@tauri-apps/api/core");
  const channel = new Channel<
    | {
        type: "progress";
        done: number;
        total: number;
        name: string;
        doneBytes: number;
        totalBytes: number;
      }
    | { type: "done"; pasted: number; skipped: string[]; warnings: string[] }
    | { type: "failed"; message: string }
  >();
  let jobId: number | null = null;
  let cancelled = false;

  channel.onmessage = (event) => {
    if (event.type === "progress") {
      // A cancelled paste stops moving, but it still has to report that it
      // stopped: swallowing the final message left the progress bar frozen at
      // whatever percentage it had reached.
      if (!cancelled) {
        handlers.onProgress(
          event.done,
          event.total,
          event.name,
          event.doneBytes,
          event.totalBytes,
        );
      }
    } else if (event.type === "done") {
      handlers.onDone(event.pasted, event.skipped, event.warnings);
    } else {
      handlers.onFailed(event.message);
    }
  };

  try {
    jobId = await invoke<number>("paste_into", {
      destination,
      sources,
      mode,
      conflict,
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

export interface DeleteHandlers {
  onProgress: (
    done: number,
    total: number,
    name: string,
    doneBytes: number,
    totalBytes: number,
  ) => void;
  onDone: (deleted: number, warnings: string[]) => void;
}

/** Move entries to the operating system's bin, reporting progress as it goes. */
export async function deletePaths(
  paths: string[],
  handlers: DeleteHandlers,
): Promise<ScanHandle> {
  if (!isTauri) {
    throw new Error("Deleting needs the desktop app, not the browser preview.");
  }
  const { Channel, invoke } = await import("@tauri-apps/api/core");
  const channel = new Channel<
    | {
        type: "progress";
        done: number;
        total: number;
        name: string;
        doneBytes: number;
        totalBytes: number;
      }
    | { type: "done"; deleted: number; warnings: string[] }
  >();
  let jobId: number | null = null;
  let cancelled = false;

  channel.onmessage = (event) => {
    if (event.type === "progress") {
      if (!cancelled) {
        handlers.onProgress(
          event.done,
          event.total,
          event.name,
          event.doneBytes,
          event.totalBytes,
        );
      }
    } else {
      handlers.onDone(event.deleted, event.warnings);
    }
  };

  try {
    jobId = await invoke<number>("delete_paths", { paths, onEvent: channel });
  } catch (error) {
    handlers.onDone(0, [describe(error)]);
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

export interface PreviewInfo {
  kind: "image" | "video" | "audio" | "pdf" | "text" | "folder" | "other";
  size: number;
  modifiedMs: number | null;
  /** First lines of a text file. */
  excerpt: string | null;
  hasThumbnail: boolean;
}

export async function previewInfo(path: string): Promise<PreviewInfo> {
  if (!isTauri) {
    // The browser preview cannot run the thumbnail tools, so it reports the
    // same shape with no picture, which exercises the fallback path.
    const ext = path.split(".").pop()?.toLowerCase() ?? "";
    const kind = ["png", "jpg", "jpeg", "gif", "webp"].includes(ext)
      ? "image"
      : ["mp4", "mkv", "webm", "mov"].includes(ext)
        ? "video"
        : ext === "pdf"
          ? "pdf"
          : ["txt", "md", "json", "ts", "tsx"].includes(ext)
            ? "text"
            : "other";
    return {
      kind,
      size: 0,
      modifiedMs: null,
      excerpt: kind === "text" ? "Preview text is produced by the desktop app." : null,
      hasThumbnail: kind === "image" || kind === "video" || kind === "pdf",
    };
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<PreviewInfo>("preview_info", { path });
}

/** A thumbnail for the file as an object URL, or null when there is none. */
export async function previewThumbnail(path: string): Promise<string | null> {
  if (!isTauri) {
    const { mockThumbnail } = await import("./mock");
    return mockThumbnail(path);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  const raw = await invoke<ArrayBuffer | number[]>("preview_thumbnail", { path });
  // Binary IPC normally arrives as an ArrayBuffer; a plain array is accepted
  // too so this cannot silently turn into a Blob of comma-separated digits.
  const bytes =
    raw instanceof ArrayBuffer ? new Uint8Array(raw) : new Uint8Array(raw);
  if (bytes.byteLength === 0) return null;
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

export interface GraphicsSettings {
  reducedMemory: boolean;
  active: boolean;
  needsRestart: boolean;
}

export async function graphicsSettings(): Promise<GraphicsSettings> {
  if (!isTauri) return { reducedMemory: false, active: false, needsRestart: false };
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<GraphicsSettings>("graphics_settings");
}

export async function setGraphicsSettings(
  reducedMemory: boolean,
): Promise<GraphicsSettings> {
  if (!isTauri) throw new Error("Needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<GraphicsSettings>("set_graphics_settings", { reducedMemory });
}

export interface EjectOutcome {
  ejected: boolean;
  steps: string[];
  error: string | null;
}

/** Unmount a removable volume and power the device down. */
export async function ejectVolume(path: string): Promise<EjectOutcome> {
  if (!isTauri) {
    return {
      ejected: false,
      steps: [],
      error: "Ejecting needs the desktop app, not the browser preview.",
    };
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<EjectOutcome>("eject_volume", { path });
}

export interface ManagerState {
  current: string | null;
  isDefault: boolean;
  /** Set when this platform cannot be asked to change the folder handler. */
  unsupported: string | null;
  desktopFile: string | null;
  /** Whether the change can be undone from here. */
  restoreSupported: boolean;
}

export async function fileManagerState(): Promise<ManagerState> {
  if (!isTauri) {
    return {
      current: null,
      isDefault: false,
      unsupported: "Making Lyre the folder handler needs the desktop app.",
      desktopFile: null,
      restoreSupported: false,
    };
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ManagerState>("file_manager_state");
}

export async function restoreFileManager(): Promise<ManagerState> {
  if (!isTauri) throw new Error("Needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ManagerState>("restore_file_manager");
}

export async function becomeFileManager(): Promise<ManagerState> {
  if (!isTauri) throw new Error("Needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ManagerState>("become_file_manager");
}

/** Whether Win+E is taken even with no Lyre window open. */
export async function winEAgent(): Promise<boolean> {
  if (!isTauri) return false;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<boolean>("win_e_agent");
}

/** Turn the logon agent that owns Win+E on or off. */
export async function setWinEAgent(enabled: boolean): Promise<boolean> {
  if (!isTauri) throw new Error("Needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<boolean>("set_win_e_agent", { enabled });
}

/** The folder this process was launched with, when a desktop opened one. */
export async function startupFolder(): Promise<string | null> {
  if (!isTauri) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string | null>("startup_folder");
}

/** A newer build that the release page is offering. */
export interface UpdateOffer {
  version: string;
  current: string;
  notes: string | null;
}

/** Ask the project's releases whether there is a newer build. */
export async function checkForUpdate(): Promise<UpdateOffer | null> {
  if (!isTauri) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<UpdateOffer | null>("check_for_update");
}

/**
 * Download the offered build and hand it to the installer.
 *
 * Bytes are reported as they arrive, so a slow connection can be seen to be
 * working rather than guessed at.
 */
export async function installUpdate(handlers: {
  onProgress: (downloaded: number, total: number | null) => void;
  onInstalling: () => void;
}): Promise<void> {
  if (!isTauri) throw new Error("Updates need the desktop app.");
  const { Channel, invoke } = await import("@tauri-apps/api/core");
  const channel = new Channel<
    | { type: "progress"; downloaded: number; total: number | null }
    | { type: "installing" }
  >();
  channel.onmessage = (event) => {
    if (event.type === "progress") {
      handlers.onProgress(event.downloaded, event.total);
    } else {
      handlers.onInstalling();
    }
  };
  return invoke("install_update", { onEvent: channel });
}

/** Restart into the build that was just installed. */
export async function restartApp(): Promise<void> {
  if (!isTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("restart_app");
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

/**
 * One item the system's own menu would offer.
 *
 * The id is the only part that is stored, so it names the registration rather
 * than the text: a handler's labels change with the selection and with the
 * language, and an item that was turned off has to stay off.
 */
export interface ShellMenuEntry {
  id: string;
  label: string;
  /** `file`, `folder` or `background`. */
  origin: string;
  /** The program that registered it. */
  source: string;
  kind: string;
  /** Whether it shows before anyone has chosen anything. */
  defaultOn: boolean;
  children: ShellMenuEntry[];
}

export interface ShellMenuCatalog {
  supported: boolean;
  note: string | null;
  entries: ShellMenuEntry[];
  /** Registrations that exist but could not be read, for the customise list. */
  warnings: string[];
}

/** What the system's menu would offer for this target, or for a folder's own background. */
export async function shellMenu(
  path: string | null,
  background: boolean,
): Promise<ShellMenuCatalog> {
  if (!isTauri) {
    const { mockShellMenu } = await import("./mock");
    return mockShellMenu(background);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ShellMenuCatalog>("shell_menu", { path, background });
}

/** Run one of those items, through the program that registered it. */
export async function runShellMenuItem(id: string, paths: string[]): Promise<void> {
  if (!isTauri) {
    throw new Error("The system's own menu items need the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("shell_menu_run", { id, paths });
}

