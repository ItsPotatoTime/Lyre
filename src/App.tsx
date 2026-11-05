import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ColumnWidths,
  Entry,
  KindFilter,
  Location,
  PathInfo,
  Place,
  Selection,
  SortKey,
  SortState,
  Tab,
  ViewMode,
} from "@/types";
import {
  isTauri,
  createFolder,
  deletePaths,
  pasteConflicts,
  pasteInto,
  type Conflict,
  getHomeDir,
  getPathInfo,
  getPlaces,
  openPath,
  pathProperties,
  renamePath,
  appsForFile,
  openWithApp,
  setDefaultApp,
  chooseAppWithSystem,
  ejectVolume,
  becomeFileManager,
  restoreFileManager,
  fileManagerState,
  winEAgent as readWinEAgent,
  setWinEAgent,
  checkForUpdate,
  installUpdate,
  restartApp,
  runShellMenuItem,
  watchFolder,
  type UpdateOffer,
  graphicsSettings,
  setGraphicsSettings,
  startupFolder,
  type ManagerState,
  type OpenWith,
  type PathProperties,
  type ShellMenuCatalog,
  type ShellMenuEntry,
} from "@/lib/ipc";
import { useDirectory } from "@/lib/useDirectory";
import { matchIndex, useSearch } from "@/lib/useSearch";
import { useReveal } from "@/lib/hooks";
import { useTheme } from "@/lib/theme";
import { loadPrefs, savePrefs, type Prefs } from "@/lib/prefs";
import { setHomePath } from "@/lib/platform";
import {
  loadLibrary,
  placeForPath,
  saveLibrary,
  type Library,
} from "@/lib/library";
import {
  backgroundDropZone,
  targetAt,
  useDrag,
  type DragTarget,
  type DropMode,
} from "@/lib/dragAndDrop";
import { sameVolume } from "@/lib/volumes";
import { startNativeDrag, warmNativeDrag } from "@/lib/nativeDrag";
import {
  clearDropHighlight,
  setDropHighlight,
  watchExternalDrops,
} from "@/lib/externalDrag";
import { windowChrome } from "@/lib/window";
import { type Mode } from "@/lib/modes";
import {
  folder,
  home as homeLocation,
  locationGlyph,
  locationLabel,
  parentOf,
  sameLocation,
} from "@/lib/location";
import { TitleBar, ResizeEdges } from "@/components/TitleBar";
import { AddressBar } from "@/components/AddressBar";
import { CommandBar } from "@/components/CommandBar";
import { NavRail } from "@/components/NavRail";
import { FileList } from "@/components/FileList";
import { HomeView } from "@/components/HomeView";
import { ThisPcView } from "@/components/ThisPcView";
import { StatusBar } from "@/components/StatusBar";
import { ContextMenu, type ContextItem } from "@/components/ContextMenu";
import { CustomizeMenuDialog } from "@/components/CustomizeMenuDialog";
import {
  filterMenu,
  insertMenuBlock,
  menuItemShown,
  shellMenuItems,
  uniqueShellEntries,
  type MenuScope,
} from "@/lib/menuItems";
import { loadShellMenu, shellMenuKey } from "@/lib/shellMenuCache";
import { PropertiesDialog } from "@/components/PropertiesDialog";
import { OpenWithDialog } from "@/components/OpenWithDialog";
import { UpdateDialog } from "@/components/UpdateDialog";
import { DefaultManagerPrompt } from "@/components/DefaultManagerPrompt";
import { PreviewPane } from "@/components/PreviewPane";
import { ConflictDialog } from "@/components/ConflictDialog";
import { Icon } from "@/components/Icons";

let tabSeq = 0;

function makeTab(location: Location, prefs: Prefs): Tab {
  tabSeq += 1;
  return {
    id: `tab-${tabSeq}`,
    location,
    history: [location],
    index: 0,
    sort: prefs.sort,
    view: prefs.view,
    filter: "all",
    groupByDate: prefs.groupByDate,
  };
}

function matchesFilter(entry: Entry, filter: KindFilter): boolean {
  if (filter === "all") return true;
  if (filter === "folder") return entry.isDir;
  if (filter === "document") {
    return (
      entry.kind === "document" ||
      entry.kind === "spreadsheet" ||
      entry.kind === "presentation" ||
      entry.kind === "pdf" ||
      entry.kind === "text"
    );
  }
  if (filter === "code") {
    return (
      entry.kind === "code" ||
      entry.kind === "executable" ||
      entry.kind === "binary" ||
      entry.kind === "database"
    );
  }
  return entry.kind === filter;
}

export default function App() {
  const [home, setHome] = useState<string | null>(null);
  const [places, setPlaces] = useState<Place[]>([]);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([getHomeDir(), getPlaces()]).then(([start, list]) => {
        if (cancelled) return;
        // Known before the chrome paints, so the home directory is named
        // rather than spelled from the first frame.
        setHomePath(start);
        setHome(start);
        setPlaces(list);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Volumes can appear and disappear while the window is open. The main process
  // watches for that and sends the new list, so a stick plugged in after
  // startup shows up without a manual refresh.
  useEffect(() => {
    let stop: (() => void) | null = null;
    let cancelled = false;

    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<Place[]>("places-changed", (event) => setPlaces(event.payload)),
      )
      .then((unlisten) => {
        if (cancelled) unlisten();
        else stop = unlisten;
      })
      .catch(() => {
        // No event bridge (browser preview); the list simply does not update.
      });

    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  if (home === null) {
    return (
      <div className="app">
        <ResizeEdges />
        <div className="boot">
          <Icon name="folder" size={28} strokeWidth={1.2} />
        </div>
      </div>
    );
  }

  return (
    <Browser
      home={home}
      places={places}
      onRefreshPlaces={() => void getPlaces().then(setPlaces)}
    />
  );
}

function Browser({
  home,
  places,
  onRefreshPlaces,
}: {
  home: string;
  places: Place[];
  onRefreshPlaces: () => void;
}) {
  const theme = useTheme();
  useReveal();

  // Read once, so the first paint is already the saved arrangement.
  const [prefs] = useState(() => loadPrefs());
  const [tabs, setTabs] = useState<Tab[]>(() => [
    makeTab(homeLocation(home), prefs),
  ]);
  // A desktop that opens a folder with us passes it as an argument; the first
  // tab starts there instead of on Home.
  useEffect(() => {
    void startupFolder().then((start) => {
      if (start) navigate(folder(start));
    });
    // Once, at startup.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const [activeId, setActiveId] = useState<string>(tabs[0]!.id);
  const [query, setQuery] = useState("");
  const [navOpen, setNavOpen] = useState(prefs.navOpen);
  const [navWidth, setNavWidth] = useState(prefs.navWidth);
  const [showHidden, setShowHidden] = useState(prefs.showHidden);
  const [showProtected, setShowProtected] = useState(prefs.showProtected);
  const [showFrameRate, setShowFrameRate] = useState(false);
  const [mode, setMode] = useState<Mode>(prefs.mode);
  const [columns, setColumns] = useState<ColumnWidths>(prefs.columns);
  const [notice, setNotice] = useState<string | null>(null);
  const [info, setInfo] = useState<PathInfo | null>(null);
  const [nonce, setNonce] = useState(0);
  const [editSignal, setEditSignal] = useState(0);
  const [driveSelected, setDriveSelected] = useState<string | null>(null);
  const [windowFocused, setWindowFocused] = useState(true);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [promptDefault, setPromptDefault] = useState(prefs.promptDefaultManager);
  const [library, setLibrary] = useState<Library>(() => loadLibrary());
  const [ejecting, setEjecting] = useState<string | null>(null);
  const [reducedMemory, setReducedMemory] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(prefs.previewOpen);
  const [menuHidden, setMenuHidden] = useState<string[]>(prefs.menuHidden);
  const [menuShown, setMenuShown] = useState<string[]>(prefs.menuShown);
  /** The customize list, and which menu it is describing. */
  const [customize, setCustomize] = useState<{
    scope: MenuScope;
    key: string;
  } | null>(null);
  /**
   * What the system's own menu offers, by file type. Filled before the menu is
   * opened, because reading it means loading every handler that wants a say and
   * that is not something to do while the pointer is waiting.
   */
  const [shellCatalogs, setShellCatalogs] = useState<
    Record<string, ShellMenuCatalog>
  >({});
  const [pendingPaste, setPendingPaste] = useState<{
    destination: string;
    sources: string[];
    mode: "copy" | "cut";
    names: string[];
  } | null>(null);
  const [manager, setManager] = useState<ManagerState | null>(null);
  const [showManagerPrompt, setShowManagerPrompt] = useState(false);
  const [managerBusy, setManagerBusy] = useState(false);
  const [managerError, setManagerError] = useState<string | null>(null);
  const [winEAgent, setWinEAgentOn] = useState(false);
  /** A build the release page is offering, and what is happening to it. */
  const [update, setUpdate] = useState<{
    offer: UpdateOffer;
    phase: "offered" | "downloading" | "installing" | "ready" | "failed";
    downloaded: number;
    total: number | null;
    error: string | null;
  } | null>(null);
  const [clipboard, setClipboard] = useState<{
    mode: "copy" | "cut";
    paths: string[];
  } | null>(null);
  /**
   * A paste in flight, for the status bar. Without this a move between two
   * drives sat there for minutes with nothing on screen to say it had started.
   */
  const [transfer, setTransfer] = useState<{
    mode: "copy" | "cut" | "delete";
    name: string;
    done: number;
    total: number;
    doneBytes: number;
    totalBytes: number;
    /** Cancelled, waiting for the operation to stop at its next chunk. */
    stopping: boolean;
  } | null>(null);
  const transferControl = useRef<{ cancel: () => void } | null>(null);
  const [properties, setProperties] = useState<PathProperties | null>(null);
  const [openWith, setOpenWith] = useState<{
    path: string;
    name: string;
    isDir: boolean;
    data: OpenWith | null;
    busy: boolean;
    error: string | null;
  } | null>(null);
  const [context, setContext] = useState<{
    x: number;
    y: number;
    /** A file-list row, when the menu is about one. */
    entry: Entry | null;
    /** A place from a tile, a drive card or the rail. */
    place?: Place;
    from?: "places" | "favorites" | "rail";
    builtIn?: boolean;
  } | null>(null);
  const [selection, setSelection] = useState<Selection>({
    selected: new Set(),
    focused: null,
    anchor: null,
  });

  const searchRef = useRef<HTMLInputElement | null>(null);
  const noticeTimer = useRef<number | null>(null);

  const active = tabs.find((tab) => tab.id === activeId) ?? tabs[0]!;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;
  const location = active.location;

  const folders = useMemo(
    () => places.filter((place) => place.kind === "folder"),
    [places],
  );
  const volumes = useMemo(
    () => places.filter((place) => place.kind !== "folder"),
    [places],
  );
  const pictures =
    folders.find((place) => place.name === "Pictures")?.path ?? home;
  // Quick access lists the known folders, not the home directory itself.
  const quickAccess = useMemo(
    () => folders.filter((place) => place.path !== home),
    [folders, home],
  );

  const showsFiles = location.kind === "folder" || location.kind === "gallery";
  // Any point in the content area that is not a specific row or breadcrumb is
  // a drop into this folder. Home and This PC are deliberately excluded.
  const contentDropZone = backgroundDropZone(location.path, showsFiles);
  const dir = useDirectory(
    showsFiles ? location.path : "",
    showHidden,
    showProtected,
    active.sort,
    nonce,
  );

  // Search works from any place. A folder searches itself; Home searches the
  // home directory; This PC searches every volume. A place with no directory
  // of its own has no local phase, so the walk carries the answer.
  const searchPath =
    location.kind === "thisPc"
      ? ""
      : location.kind === "home"
        ? home
        : location.path;
  const roots = useMemo(() => {
    if (location.kind === "thisPc") return volumes.map((volume) => volume.path);
    if (location.kind === "home") return [home];
    return [] as string[];
  }, [location.kind, home, volumes]);

  const searching = query.trim().length > 0;
  const search = useSearch(
    searchPath,
    roots,
    query,
    showHidden,
    showProtected,
    searching,
  );
  const highlights = useMemo(() => matchIndex(search.items), [search.items]);

  const patchTab = useCallback((patch: (tab: Tab) => Tab) => {
    const id = activeIdRef.current;
    setTabs((prev) => prev.map((tab) => (tab.id === id ? patch(tab) : tab)));
  }, []);

  const flash = useCallback((text: string) => {
    setNotice(text);
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(null), 4200);
  }, []);

  useEffect(
    () => () => {
      if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    },
    [],
  );

  const resetSelection = useCallback(() => {
    setSelection({ selected: new Set(), focused: null, anchor: null });
  }, []);

  const navigate = useCallback(
    (next: Location, options?: { view?: ViewMode }) => {
      patchTab((tab) => {
        const history = tab.history.slice(0, tab.index + 1);
        const current = history[history.length - 1];
        if (current && sameLocation(current, next)) {
          return { ...tab, location: next };
        }
        history.push(next);
        return {
          ...tab,
          location: next,
          history,
          index: history.length - 1,
          ...(options?.view ? { view: options.view } : {}),
        };
      });
      setQuery("");
      resetSelection();
      setDriveSelected(null);
    },
    [patchTab, resetSelection],
  );

  const refresh = useCallback(() => {
    setNonce((value) => value + 1);
    flash("Refreshed");
  }, [flash]);

  // --- changes made by another program ------------------------------------
  const currentPath = useRef(location.path);
  currentPath.current = location.path;
  const renamingRef = useRef(renaming);
  renamingRef.current = renaming;
  const pendingExternal = useRef(false);

  // One folder is watched at a time: the one behind the listing. Virtual pages
  // have no single folder, so they stop the watcher. It stays active during a
  // search so clearing the query does not reveal a stale listing.
  useEffect(() => {
    void watchFolder(showsFiles ? location.path : null);
  }, [location.path, showsFiles]);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let cancelled = false;
    let timer: number | null = null;
    let firstAt = 0;

    const flush = () => {
      timer = null;
      firstAt = 0;
      setNonce((value) => value + 1);
    };

    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<string>("folder-changed", (event) => {
          if (event.payload !== currentPath.current) return;
          // An inline rename owns a row. Replace the listing once it closes
          // rather than pulling the editor out from under the person typing.
          if (renamingRef.current !== null) {
            pendingExternal.current = true;
            return;
          }
          const now = Date.now();
          if (firstAt === 0) firstAt = now;
          // A copy of a thousand files is one refresh, not a thousand. The
          // upper bound keeps a long copy from postponing it indefinitely.
          const left = Math.max(0, 1200 - (now - firstAt));
          if (timer !== null) window.clearTimeout(timer);
          timer = window.setTimeout(flush, Math.min(180, left));
        }),
      )
      .then((unlisten) => {
        if (cancelled) unlisten();
        else stop = unlisten;
      })
      .catch(() => {
        // The browser preview has no event bridge; nothing to listen to.
      });

    return () => {
      cancelled = true;
      stop?.();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, []);

  // A change that arrived during a rename is applied once the editor closes.
  useEffect(() => {
    if (renaming === null && pendingExternal.current) {
      pendingExternal.current = false;
      setNonce((value) => value + 1);
    }
  }, [renaming]);

  useEffect(() => {
    if (!showsFiles) {
      setInfo(null);
      return;
    }
    let cancelled = false;
    void getPathInfo(location.path).then((next) => {
      if (!cancelled) setInfo(next);
    });
    return () => {
      cancelled = true;
    };
  }, [location.path, showsFiles, nonce]);

  useEffect(() => {
    resetSelection();
  }, [location.path, location.kind, resetSelection]);

  // Persist the view as it changes. Cheap enough to write on each change and
  // far simpler than tracking dirty fields.
  useEffect(() => {
    saveLibrary(library);
  }, [library]);

  useEffect(() => {
    savePrefs({
      sort: active.sort,
      view: active.view,
      groupByDate: active.groupByDate,
      mode,
      showHidden,
      showProtected,
      navOpen,
      navWidth,
      columns,
      promptDefaultManager: promptDefault,
      previewOpen,
      menuHidden,
      menuShown,
    });
  }, [
    active.sort,
    active.view,
    active.groupByDate,
    mode,
    showHidden,
    showProtected,
    navOpen,
    navWidth,
    columns,
    promptDefault,
    previewOpen,
    menuHidden,
    menuShown,
  ]);

  useEffect(() => {
    void graphicsSettings()
      .then((settings) => setReducedMemory(settings.reducedMemory))
      .catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    void fileManagerState()
      .then((state) => {
        if (cancelled) return;
        setManager(state);
        // Only offer when there is something to offer, and only if the person
        // has not asked not to be asked.
        if (!state.isDefault && promptDefault) {
          setShowManagerPrompt(true);
        }
      })
      .catch(() => {
        // No answer means no prompt; this must never block the window.
      });
    // Whether the Win+E agent runs at logon, so the menu reads correctly.
    void readWinEAgent()
      .then((enabled) => {
        if (!cancelled) setWinEAgentOn(enabled);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
    // Read once at startup on purpose.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onFocus = () => setWindowFocused(true);
    const onBlur = () => setWindowFocused(false);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  const goBack = useCallback(() => {
    patchTab((tab) =>
      tab.index > 0
        ? {
            ...tab,
            index: tab.index - 1,
            location: tab.history[tab.index - 1]!,
          }
        : tab,
    );
  }, [patchTab]);

  const goForward = useCallback(() => {
    patchTab((tab) =>
      tab.index < tab.history.length - 1
        ? {
            ...tab,
            index: tab.index + 1,
            location: tab.history[tab.index + 1]!,
          }
        : tab,
    );
  }, [patchTab]);

  const goUp = useCallback(() => {
    const parent = parentOf(location, info);
    if (parent) navigate(parent);
  }, [info, location, navigate]);

  const newTab = useCallback(() => {
    const tab = makeTab(homeLocation(home), {
      sort: active.sort,
      view: active.view,
      groupByDate: active.groupByDate,
    } as Prefs);
    setTabs((prev) => [...prev, tab]);
    setActiveId(tab.id);
    setQuery("");
    resetSelection();
  }, [home, resetSelection]);

  const openInNewTab = useCallback(
    (target: Location) => {
      const tab = makeTab(target, {
        sort: active.sort,
        view: active.view,
        groupByDate: active.groupByDate,
      } as Prefs);
      setTabs((prev) => [...prev, tab]);
      setActiveId(tab.id);
      setQuery("");
      resetSelection();
    },
    [active.groupByDate, active.sort, active.view, resetSelection],
  );

  const closeTab = useCallback(
    (id: string) => {
      const index = tabs.findIndex((tab) => tab.id === id);
      if (index === -1 || tabs.length === 1) return;
      const next = tabs.filter((tab) => tab.id !== id);
      setTabs(next);
      if (id === activeIdRef.current) {
        setActiveId(next[Math.min(index, next.length - 1)]!.id);
      }
    },
    [tabs],
  );

  const setView = useCallback(
    (view: ViewMode) => patchTab((tab) => ({ ...tab, view })),
    [patchTab],
  );
  const setSort = useCallback(
    (sort: SortState) => patchTab((tab) => ({ ...tab, sort })),
    [patchTab],
  );
  const setFilter = useCallback(
    (filter: KindFilter) => patchTab((tab) => ({ ...tab, filter })),
    [patchTab],
  );
  const setGroupByDate = useCallback(
    (groupByDate: boolean) =>
      patchTab((tab) => ({
        ...tab,
        groupByDate,
        // Grouping is by date, so the rows must be in date order for the
        // headers to be contiguous, exactly as Explorer orders them.
        sort: groupByDate ? { key: "modified", dir: -1 } : tab.sort,
      })),
    [patchTab],
  );

  

  const copyPath = useCallback(async () => {
    const paths = [...selection.selected];
    if (paths.length === 0) return;
    try {
      await navigator.clipboard.writeText(paths.join("\n"));
      flash(`Copied ${paths.length} path${paths.length === 1 ? "" : "s"}`);
    } catch {
      flash("The clipboard is not available in this window.");
    }
  }, [flash, selection.selected]);

  const filtering = searching;
  const visible = useMemo(() => {
    const matchesKind = (entry: Entry) =>
      active.filter === "all" || matchesFilter(entry, active.filter);

    // Ranked by relevance in Rust; the kind filter still applies on top.
    if (searching) return search.items.filter(matchesKind);
    // eslint-disable-next-line react-hooks/exhaustive-deps
    return dir.entries.filter(matchesKind);
    // `dir.version` advances when the streamed array is mutated in place.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searching, search.items, dir.entries, dir.version, active.filter]);

  /**
   * The two catalogues the menus read: one for the item that would be
   * right-clicked, one for the folder's own empty space. They are keyed by what
   * decides the answer, which is the file type or the folder itself.
   */
  const itemEntry = useMemo(() => {
    const path = selection.focused ?? visible[0]?.path ?? null;
    return path ? (visible.find((entry) => entry.path === path) ?? null) : null;
  }, [selection.focused, visible]);
  const itemTarget = itemEntry?.path ?? null;
  const itemShellKey = shellMenuKey(itemTarget, false, itemEntry?.isDir === true);
  const backgroundShellKey = shellMenuKey(location.path, true);

  useEffect(() => {
    const wanted: { key: string; path: string; background: boolean }[] = [];
    // A background menu belongs to a real folder, and there is none while a
    // search is what is being looked at.
    if (showsFiles) {
      wanted.push({
        key: backgroundShellKey,
        path: location.path,
        background: true,
      });
    }
    if (itemTarget) {
      wanted.push({
        key: itemShellKey,
        path: itemTarget,
        background: false,
      });
    }
    if (wanted.length === 0) return;

    let cancelled = false;
    for (const want of wanted) {
      void loadShellMenu(want.key, want.path, want.background).then((catalog) => {
        if (cancelled) return;
        setShellCatalogs((prev) =>
          prev[want.key] ? prev : { ...prev, [want.key]: catalog },
        );
      });
    }
    return () => {
      cancelled = true;
    };
  }, [backgroundShellKey, itemShellKey, itemTarget, location.path, showsFiles]);

  // The list can be asked for a menu that was never prefetched, which happens
  // when it is opened from the command bar on a page with no listing.
  useEffect(() => {
    if (!customize || shellCatalogs[customize.key]) return;
    const background = customize.scope === "background";
    if (customize.scope === "place") return;
    const path = background ? location.path : itemTarget;
    if (!path) return;
    let cancelled = false;
    void loadShellMenu(customize.key, path, background).then((catalog) => {
      if (cancelled) return;
      setShellCatalogs((prev) =>
        prev[customize.key] ? prev : { ...prev, [customize.key]: catalog },
      );
    });
    return () => {
      cancelled = true;
    };
  }, [customize, itemTarget, location.path, shellCatalogs]);

  const onOpen = useCallback(
    (entry: Entry) => {
      if (entry.isDir) navigate(folder(entry.path));
      else {
        void openPath(entry.path).catch((error: unknown) => {
          flash(error instanceof Error ? error.message : "Could not open that file.");
        });
      }
    },
    [flash, navigate],
  );

  // --- actions the interface can really perform ---------------------------
  const refreshAfterChange = useCallback(
    (select?: string) => {
      setNonce((value) => value + 1);
      if (select) {
        setSelection({
          selected: new Set([select]),
          focused: select,
          anchor: select,
        });
      }
    },
    [],
  );

  /**
   * Carry a dropped selection to wherever it landed.
   *
   * Every target resolves to a folder path, so one operation covers rows,
   * breadcrumbs, tabs, the rail and the empty area. Moving into the folder the
   * item already lives in is filtered out here as well as in Rust, so the
   * feedback says "nothing to do" instead of reporting a misleading success.
   */
  const runPaste = useCallback(
    (destination: string, sources: string[], mode: "copy" | "cut", conflict: Conflict) => {
      const verb = mode === "cut" ? "Moving" : "Copying";
      const only = sources.length === 1 ? sources[0].split(/[\\/]/).pop() ?? sources[0] : null;
      // Shown before the first message arrives, so the bar is there from the
      // moment the work starts rather than after the first megabyte.
      setTransfer({
        mode,
        name: only ?? `${sources.length} items`,
        done: 0,
        total: sources.length,
        doneBytes: 0,
        totalBytes: 0,
        stopping: false,
      });
      void pasteInto(destination, sources, mode, conflict, {
        onProgress: (done, total, name, doneBytes, totalBytes) => {
          setTransfer({
            mode,
            name: only ?? name,
            done,
            total,
            doneBytes,
            totalBytes,
            stopping: false,
          });
        },
        onDone: (pasted, skipped, warnings) => {
          setTransfer(null);
          transferControl.current = null;
          refreshAfterChange();
          if (mode === "cut") setClipboard(null);
          flash(
            warnings.length > 0
              ? warnings[0]
              : skipped.length > 0
              ? `${verb} ${pasted}, ${skipped.length} skipped`
              : `${mode === "cut" ? "Moved" : "Copied"} ${pasted} item${pasted === 1 ? "" : "s"}`,
          );
        },
        onFailed: (message) => {
          setTransfer(null);
          transferControl.current = null;
          flash(message);
        },
      })
        .then((control) => {
          transferControl.current = control;
        })
        .catch((error: unknown) => {
          setTransfer(null);
          flash(error instanceof Error ? error.message : "Could not finish the paste.");
        });
    },
    [flash, refreshAfterChange],
  );

  /** Ask about collisions before anything is written. */
  const requestPaste = useCallback(
    (destination: string, sources: string[], mode: "copy" | "cut") => {
      void pasteConflicts(destination, sources)
        .then((names) => {
          if (names.length === 0) {
            runPaste(destination, sources, mode, "rename");
          } else {
            setPendingPaste({ destination, sources, mode, names });
          }
        })
        .catch(() => runPaste(destination, sources, mode, "rename"));
    },
    [runPaste],
  );

  const handleDrop = useCallback(
    (target: DragTarget, mode: DropMode, paths: string[]) => {
      const destination = target.path;
      const sources = paths.filter(
        (path) =>
          path !== destination &&
          !destination.startsWith(`${path}/`) &&
          !destination.startsWith(`${path}\\`) &&
          !(mode === "move" && path.slice(0, path.lastIndexOf("/")) === destination),
      );

      if (sources.length === 0) {
        flash(
          mode === "move"
            ? "Those items are already in that folder."
            : "Nothing to copy.",
        );
        return;
      }

      requestPaste(destination, sources, mode === "move" ? "cut" : "copy");

      // Dropping onto a tab is also a request to look at that tab.
      if (target.kind === "tab" && target.id) setActiveId(target.id);
    },
    [flash, requestPaste],
  );

  /**
   * Between volumes, copying is the default and Shift moves; within one volume
   * it is the other way round. Dragging to a USB stick should not move a file
   * off the disk by accident.
   */
  const modeFor = useCallback(
    (paths: string[], target: DragTarget | null, shift: boolean): DropMode => {
      const local = target ? sameVolume(paths, target.path, volumes) : true;
      const base: DropMode = local ? "move" : "copy";
      if (!shift) return base;
      return base === "move" ? "copy" : "move";
    },
    [volumes],
  );

  const drag = useDrag(handleDrop, modeFor, (paths, mode) => {
    // The pointer left the window, so the drag is meant for another
    // application. Handing it to the platform from here means the same gesture
    // can finish in Discord or LocalSend.
    void startNativeDrag(paths, mode).then((error) => {
      if (error) flash(error);
    });
  });

  /**
   * Drops from outside the window arrive through Tauri with real paths. They
   * are always copies: the source may be on another volume or another machine,
   * and guessing that a drag between two windows means "move" is how people
   * lose files.
   */
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let cancelled = false;

    // DoDragDrop has to start while the button is down; load its modules now,
    // not on the first drag to the window edge.
    warmNativeDrag();

    void watchExternalDrops({
      onOver: (x, y) => setDropHighlight(x, y),
      onLeave: () => clearDropHighlight(),
      onDrop: (x, y, pathsPromise) => {
        // Where the files were released must not depend on a later repaint.
        const target = targetAt(x, y);
        if (!target) return;
        void pathsPromise.then((paths) => {
          if (paths.length === 0) return;
          handleDrop(target, "copy", paths);
        });
      },
    })
      .then((disposeFn) => {
        if (cancelled) disposeFn();
        else dispose = disposeFn;
      })
      .catch(() => {
        // No native drop support on this platform; internal dragging still works.
      });

    return () => {
      cancelled = true;
      dispose?.();
      clearDropHighlight();
    };
  }, [handleDrop]);

  const acceptManager = useCallback(() => {
    setManagerBusy(true);
    setManagerError(null);
    void becomeFileManager()
      .then((state) => {
        setManager(state);
        setShowManagerPrompt(false);
        setManagerBusy(false);
        flash("Folders now open in Lyre");
      })
      .catch((error: unknown) => {
        setManagerBusy(false);
        setManagerError(
          error instanceof Error ? error.message : "Could not change the handler.",
        );
      });
  }, [flash]);

  /** Dismissing without accepting still hides it for this run only. */
  const dismissManager = useCallback(() => setShowManagerPrompt(false), []);

  /** "Never" is a stored preference, so it can be turned back on in the menu. */
  const restoreManager = useCallback(() => {
    setManagerBusy(true);
    void restoreFileManager()
      .then((state) => {
        setManager(state);
        setManagerBusy(false);
        flash("Folders open in the previous file manager again");
      })
      .catch((error: unknown) => {
        setManagerBusy(false);
        flash(
          error instanceof Error ? error.message : "Could not undo the change.",
        );
      });
  }, [flash]);

  /** The logon agent that keeps Win+E here after every window is closed. */
  const toggleWinEAgent = useCallback(() => {
    const next = !winEAgent;
    void setWinEAgent(next)
      .then((enabled) => {
        setWinEAgentOn(enabled);
        flash(
          enabled
            ? "Win+E opens Lyre even when it is closed"
            : "Win+E is Explorer's again when Lyre is closed",
        );
      })
      .catch((error: unknown) => {
        flash(
          error instanceof Error ? error.message : "Could not change that.",
        );
      });
  }, [winEAgent, flash]);

  /** Ask the release page whether there is something newer. */
  const checkUpdates = useCallback(() => {
    void checkForUpdate()
      .then((offer) => {
        if (!offer) {
          flash("Lyre is up to date.");
          return;
        }
        setUpdate({
          offer,
          phase: "offered",
          downloaded: 0,
          total: null,
          error: null,
        });
      })
      .catch((error: unknown) =>
        flash(
          error instanceof Error
            ? error.message
            : "Could not check for updates.",
        ),
      );
  }, [flash]);

  /** Download the offered build and hand it to the installer. */
  const beginUpdate = useCallback((offer: UpdateOffer) => {
    setUpdate({
      offer,
      phase: "downloading",
      downloaded: 0,
      total: null,
      error: null,
    });
    void installUpdate({
      onProgress: (downloaded, total) =>
        setUpdate((current) =>
          current ? { ...current, downloaded, total } : current,
        ),
      onInstalling: () =>
        setUpdate((current) =>
          current ? { ...current, phase: "installing" } : current,
        ),
    })
      .then(() =>
        setUpdate((current) =>
          current ? { ...current, phase: "ready" } : current,
        ),
      )
      .catch((error: unknown) =>
        setUpdate((current) =>
          current
            ? {
                ...current,
                phase: "failed",
                error: error instanceof Error ? error.message : String(error),
              }
            : current,
        ),
      );
  }, []);

  const neverManager = useCallback(() => {
    setShowManagerPrompt(false);
    // The persisted preference is written by the effect that saves the rest of
    // the view state, so this only has to change the flag.
    setPromptDefault(false);
    flash("You can turn this offer back on from the More menu");
  }, [flash]);

  const newFolder = useCallback(() => {
    if (!showsFiles) {
      flash("Open a folder first, then create one with Ctrl+Shift+N.");
      return;
    }
    void createFolder(location.path, "New folder")
      .then((created) => {
        refreshAfterChange(created);
        // Explorer starts the name selected, so typing replaces it.
        setRenaming(created);
      })
      .catch((error: unknown) =>
        flash(error instanceof Error ? error.message : "Could not create the folder."),
      );
  }, [flash, location.path, refreshAfterChange, showsFiles]);

  const selectionPaths = useCallback(
    () => [...selection.selected],
    [selection.selected],
  );

  const copySelection = useCallback(
    (mode: "copy" | "cut") => {
      const paths = selectionPaths();
      if (paths.length === 0) {
        flash("Select something to " + (mode === "copy" ? "copy." : "cut."));
        return;
      }
      setClipboard({ mode, paths });
      flash(
        `${mode === "copy" ? "Copied" : "Cut"} ${paths.length} item${paths.length === 1 ? "" : "s"}`,
      );
    },
    [flash, selectionPaths],
  );

  const pasteHere = useCallback(() => {
    if (!clipboard) {
      flash("Nothing to paste.");
      return;
    }
    if (!showsFiles) {
      flash("Open a folder to paste into.");
      return;
    }
    requestPaste(location.path, clipboard.paths, clipboard.mode);
  }, [clipboard, flash, location.path, requestPaste, showsFiles]);

  const beginOpenWith = useCallback(
    (entry: Entry) => {
      setOpenWith({
        path: entry.path,
        name: entry.name,
        isDir: entry.isDir,
        data: null,
        busy: false,
        error: null,
      });
      void appsForFile(entry.path)
        .then((data) =>
          setOpenWith((prev) => (prev ? { ...prev, data } : prev)),
        )
        .catch((error: unknown) =>
          setOpenWith((prev) =>
            prev
              ? {
                  ...prev,
                  error:
                    error instanceof Error
                      ? error.message
                      : "Could not read the applications.",
                }
              : prev,
          ),
        );
    },
    [],
  );

  const launchWith = useCallback(
    (appId: string, remember: boolean) => {
      if (!openWith) return;
      const { path } = openWith;
      setOpenWith((prev) => (prev ? { ...prev, busy: true, error: null } : prev));
      const run = remember ? setDefaultApp(path, appId) : Promise.resolve(null);
      void run
        .then(() => openWithApp(path, appId))
        .then(() => {
          setOpenWith(null);
          flash(
            remember ? "Opened, and remembered as the default" : "Opened",
          );
        })
        .catch((error: unknown) => {
          setOpenWith((prev) =>
            prev
              ? {
                  ...prev,
                  busy: false,
                  error:
                    error instanceof Error ? error.message : "Could not open.",
                }
              : prev,
          );
        });
    },
    [flash, openWith],
  );

  const deleteSelection = useCallback(() => {
    const paths = selectionPaths();
    if (paths.length === 0) {
      flash("Select something to delete.");
      return;
    }
    const only = paths.length === 1 ? paths[0].split(/[\\/]/).pop() ?? paths[0] : null;
    // The same bar as a paste: a folder going to the bin can take a while, and
    // the wait deserves the same answer whether it is a move or a delete.
    setTransfer({
      mode: "delete",
      name: only ?? `${paths.length} items`,
      done: 0,
      total: paths.length,
      doneBytes: 0,
      totalBytes: 0,
      stopping: false,
    });
    void deletePaths(paths, {
      onProgress: (done, total, name, doneBytes, totalBytes) => {
        setTransfer({
          mode: "delete",
          name: only ?? name,
          done,
          total,
          doneBytes,
          totalBytes,
          stopping: false,
        });
      },
      onDone: (deleted, warnings) => {
        setTransfer(null);
        transferControl.current = null;
        resetSelection();
        refreshAfterChange();
        flash(
          warnings.length > 0
            ? warnings[0]
            : `Moved ${deleted} item${deleted === 1 ? "" : "s"} to the trash`,
        );
      },
    })
      .then((control) => {
        transferControl.current = control;
      })
      .catch((error: unknown) => {
        setTransfer(null);
        flash(error instanceof Error ? error.message : "Could not delete.");
      });
  }, [flash, refreshAfterChange, resetSelection, selectionPaths]);

  const beginRename = useCallback(() => {
    const target = selection.focused;
    if (!target || selection.selected.size !== 1) {
      flash("Select one item to rename.");
      return;
    }
    setRenaming(target);
  }, [flash, selection.focused, selection.selected.size]);

  const commitRename = useCallback(
    (path: string, name: string) => {
      setRenaming(null);
      const current = visible.find((entry) => entry.path === path);
      if (!current || name.trim() === current.name) return;
      void renamePath(path, name)
        .then((renamed) => refreshAfterChange(renamed))
        .catch((error: unknown) =>
          flash(error instanceof Error ? error.message : "Could not rename that item."),
        );
    },
    [flash, refreshAfterChange, visible],
  );

  const showProperties = useCallback(
    (path?: string) => {
      const target = path ?? selection.focused;
      if (!target) {
        flash("Select an item to see its properties.");
        return;
      }
      void pathProperties(target)
        .then(setProperties)
        .catch((error: unknown) => {
          // The browser preview has no filesystem to describe; the row it is
          // showing is data the scan already produced, so describe that.
          const known = visible.find((entry) => entry.path === target);
          if (!isTauri && known) {
            setProperties({
              name: known.name,
              path: known.path,
              parent: known.path.slice(0, Math.max(0, known.path.lastIndexOf("/"))),
              isDir: known.isDir,
              size: known.isDir ? null : known.size,
              items: null,
              modifiedMs: known.modifiedMs,
              readonly: false,
            });
            return;
          }
          flash(error instanceof Error ? error.message : "Could not read properties.");
        });
    },
    [flash, selection.focused],
  );

  /** The row the preview pane describes: the focused one, or a lone selection. */
  const previewEntry = useMemo(() => {
    const path =
      selection.focused ??
      (selection.selected.size === 1 ? [...selection.selected][0] : null);
    if (!path) return null;
    return visible.find((entry) => entry.path === path) ?? null;
  }, [selection.focused, selection.selected, visible]);

  const favorites = useMemo(
    () => library.favorites.map(placeForPath),
    [library.favorites],
  );

  const railPins = useMemo(() => {
    const known = places.filter((place) => place.kind === "folder").slice(1);
    const visible = known.filter(
      (place) => !library.hiddenPins.includes(place.path),
    );
    const extra = library.extraPins
      .filter(
        (path) =>
          !known.some((place) => place.path === path) &&
          !library.hiddenPins.includes(path),
      )
      .map(placeForPath);
    return [...visible, ...extra];
  }, [library.extraPins, library.hiddenPins, places]);

  const rememberFavorite = useCallback((path: string) => {
    setLibrary((prev) =>
      prev.favorites.includes(path)
        ? prev
        : { ...prev, favorites: [...prev.favorites, path] },
    );
  }, []);

  const forgetFavorite = useCallback((path: string) => {
    setLibrary((prev) => ({
      ...prev,
      favorites: prev.favorites.filter((item) => item !== path),
    }));
  }, []);

  const pinToRail = useCallback((path: string) => {
    setLibrary((prev) => {
      if (prev.hiddenPins.includes(path)) {
        return {
          ...prev,
          hiddenPins: prev.hiddenPins.filter((item) => item !== path),
        };
      }
      return prev.extraPins.includes(path)
        ? prev
        : { ...prev, extraPins: [...prev.extraPins, path] };
    });
  }, []);

  const unpinFromRail = useCallback((path: string, builtIn: boolean) => {
    setLibrary((prev) => ({
      ...prev,
      extraPins: prev.extraPins.filter((item) => item !== path),
      hiddenPins: builtIn
        ? [...new Set([...prev.hiddenPins, path])]
        : prev.hiddenPins,
    }));
  }, []);

  const restorePins = useCallback(() => {
    setLibrary((prev) => ({ ...prev, hiddenPins: [], extraPins: [] }));
    flash("Sidebar folders restored");
  }, [flash]);

  const eject = useCallback(
    (volume: Place) => {
      setEjecting(volume.path);
      void ejectVolume(volume.path).then((outcome) => {
        setEjecting(null);
        if (outcome.ejected) {
          flash(`${volume.name} can be removed safely`);
          onRefreshPlaces();
        } else {
          flash(outcome.error ?? `Could not eject ${volume.name}`);
        }
      });
    },
    [flash],
  );

  // --- what the menus are allowed to show ---------------------------------
  const hiddenIds = useMemo(() => new Set(menuHidden), [menuHidden]);
  const restoredIds = useMemo(() => new Set(menuShown), [menuShown]);
  const isMenuShown = useCallback(
    (id: string, defaultOn: boolean) =>
      menuItemShown(id, defaultOn, hiddenIds, restoredIds),
    [hiddenIds, restoredIds],
  );

  const toggleMenuItem = useCallback(
    (id: string, defaultOn: boolean, next: boolean) => {
      setMenuHidden((prev) =>
        next
          ? prev.filter((item) => item !== id)
          : prev.includes(id)
            ? prev
            : [...prev, id],
      );
      setMenuShown((prev) => {
        // An item that starts on needs no record of being on, and one that
        // starts off needs one only while it is on.
        if (!next || defaultOn) return prev.filter((item) => item !== id);
        return prev.includes(id) ? prev : [...prev, id];
      });
    },
    [],
  );

  const resetMenu = useCallback(() => {
    setMenuHidden([]);
    setMenuShown([]);
    flash("Context menus are back to their defaults");
  }, [flash]);

  /** The last item of every menu: the way into the list that shapes it. */
  const customizeEntry = useCallback(
    (scope: MenuScope): ContextItem => ({
      kind: "item",
      id: "customize-menu",
      label: "Customize this menu…",
      icon: "sliders",
      onSelect: () =>
        setCustomize({
          scope,
          key: scope === "background" ? backgroundShellKey : itemShellKey,
        }),
    }),
    [backgroundShellKey, itemShellKey],
  );

  /** Hand an item back to the program that registered it. */
  const runShellEntry = useCallback(
    (entry: ShellMenuEntry, paths: string[]) => {
      void runShellMenuItem(entry.id, paths).catch((error: unknown) =>
        flash(
          error instanceof Error ? error.message : "That item could not be run.",
        ),
      );
    },
    [flash],
  );

  /**
   * The system's own items for one target, already filtered. Empty when the
   * catalogue has not been read yet, which is why the menus are rebuilt whenever
   * one arrives rather than being fixed when they open.
   */
  const shellBlock = useCallback(
    (key: string, target: string, background: boolean): ContextItem[] => {
      const catalog = shellCatalogs[key];
      if (!catalog?.supported) return [];
      // A background command is written for the folder, so it is handed the
      // folder; an item's handler is handed the selection it will act on.
      const paths =
        background || selection.selected.size === 0
          ? [target]
          : [...selection.selected];
      return shellMenuItems(
        uniqueShellEntries(catalog.entries, background ? "background" : "item"),
        isMenuShown,
        (entry) => runShellEntry(entry, paths),
      );
    },
    [isMenuShown, runShellEntry, selection.selected, shellCatalogs],
  );

  /** Menu for a place: a rail row, a tile, or a drive card. */
  const placeItems = useCallback(
    (
      place: Place,
      from: "places" | "favorites" | "rail" | undefined,
      knownBuiltIn: boolean | undefined,
    ): ContextItem[] => {
      const builtIn =
        knownBuiltIn ?? places.some((item) => item.path === place.path);
      const isFavorite = library.favorites.includes(place.path);
      const isPinned = library.extraPins.includes(place.path);
      const isHidden = library.hiddenPins.includes(place.path);
      const isFolder = place.kind === "folder";

      const items: ContextItem[] = [
        {
          kind: "item",
          id: "place-open",
          label: "Open",
          onSelect: () => navigate(folder(place.path)),
        },
        ...(isFolder
          ? ([
              { kind: "sep" },
              {
                kind: "item",
                id: "place-favorite",
                label: isFavorite ? "Remove from favorites" : "Add to favorites",
                checked: isFavorite,
                onSelect: () =>
                  isFavorite
                    ? forgetFavorite(place.path)
                    : rememberFavorite(place.path),
              },
              {
                kind: "item",
                id: "place-pin",
                label:
                  from === "rail" || isPinned
                    ? "Remove from sidebar"
                    : isHidden
                      ? "Pin to sidebar"
                      : "Pin to sidebar",
                onSelect: () => {
                  if (from === "rail" || isPinned) {
                    unpinFromRail(place.path, builtIn);
                  } else if (isHidden) {
                    pinToRail(place.path);
                  } else {
                    unpinFromRail(place.path, true);
                    setLibrary((prev) => ({
                      ...prev,
                      extraPins: prev.extraPins.includes(place.path)
                        ? prev.extraPins
                        : [...prev.extraPins, place.path],
                    }));
                  }
                },
              },
            ] as ContextItem[])
          : []),
        ...(place.kind === "removable"
          ? ([
              { kind: "sep" },
              {
                kind: "item",
                id: "place-eject",
                label: `Eject ${place.name}`,
                icon: "eject",
                onSelect: () => eject(place),
              },
            ] as ContextItem[])
          : []),
        { kind: "sep" },
        {
          kind: "item",
          id: "place-copy",
          label: "Copy path",
          onSelect: () => {
            void navigator.clipboard?.writeText(place.path);
            flash("Path copied");
          },
        },
      ];

      return [
        ...filterMenu(items, isMenuShown),
        { kind: "sep" },
        customizeEntry("place"),
      ];
    },
    [
      customizeEntry,
      eject,
      flash,
      forgetFavorite,
      isMenuShown,
      library,
      navigate,
      pinToRail,
      places,
      rememberFavorite,
      unpinFromRail,
    ],
  );

  const contextItems = useCallback(
    (entry: Entry | null): ContextItem[] => {
      const editOps = (): ContextItem[] => [
        { kind: "sep" },
        {
          kind: "item",
          id: "cut",
          label: "Cut",
          icon: "cut",
          shortcut: "Ctrl+X",
          onSelect: () => copySelection("cut"),
        },
        {
          kind: "item",
          id: "copy",
          label: "Copy",
          icon: "copy",
          shortcut: "Ctrl+C",
          onSelect: () => copySelection("copy"),
        },
        {
          kind: "item",
          id: "delete",
          label: "Delete",
          icon: "trash",
          shortcut: "Delete",
          onSelect: deleteSelection,
        },
      ];

      if (!entry) {
        const items: ContextItem[] = [
          {
            kind: "item",
            id: "new-folder",
            label: "New folder",
            icon: "folderPlus",
            shortcut: "Ctrl+Shift+N",
            onSelect: newFolder,
          },
          {
            kind: "item",
            id: "paste",
            label: "Paste",
            icon: "paste",
            shortcut: "Ctrl+V",
            disabled: !clipboard,
            note: "Copy or cut something first.",
            onSelect: pasteHere,
          },
          { kind: "sep" },
          { kind: "label", text: "Sort by" },
          ...(
            [
              ["name", "Name"],
              ["modified", "Date modified"],
              ["type", "Type"],
              ["size", "Size"],
            ] as [SortKey, string][]
          ).map(([key, labelText]) => ({
            kind: "item" as const,
            id: `sort-${key}`,
            label: labelText,
            checked: active.sort.key === key,
            onSelect: () => setSort({ key, dir: active.sort.key === key ? active.sort.dir : 1 }),
          })),
          { kind: "sep" },
          { kind: "label", text: "View" },
          {
            kind: "item",
            id: "view-details",
            label: "Details",
            checked: active.view === "details",
            shortcut: "Ctrl+Shift+6",
            onSelect: () => setView("details"),
          },
          {
            kind: "item",
            id: "view-icons",
            label: "Large icons",
            checked: active.view === "grid",
            shortcut: "Ctrl+Shift+2",
            onSelect: () => setView("grid"),
          },
          { kind: "sep" },
          {
            kind: "item",
            id: "refresh",
            label: "Refresh",
            icon: "refresh",
            shortcut: "F5",
            onSelect: refresh,
          },
          {
            kind: "item",
            id: "select-all",
            label: "Select all",
            shortcut: "Ctrl+A",
            onSelect: () =>
              setSelection({
                selected: new Set(visible.map((item) => item.path)),
                focused: visible[0]?.path ?? null,
                anchor: visible[0]?.path ?? null,
              }),
          },
        ];

        // The folder's own menu ends with what other programs put there, then
        // the way into this list.
        return [
          ...insertMenuBlock(
            filterMenu(items, isMenuShown),
            null,
            shellBlock(backgroundShellKey, location.path, true),
          ),
          { kind: "sep" },
          customizeEntry("background"),
        ];
      }

      const items: ContextItem[] = [
        {
          kind: "item",
          id: "open",
          label: entry.isDir ? "Open" : "Open with default app",
          shortcut: "Enter",
          onSelect: () => onOpen(entry),
        },
        // Only folders can be opened in a tab; a file has no listing to show.
        ...(entry.isDir
          ? [
              {
                kind: "item" as const,
                id: "open-tab",
                label: "Open in new tab",
                onSelect: () => openInNewTab(folder(entry.path)),
              },
            ]
          : []),
        // Only files have an application to choose; a folder opens itself.
        ...(entry.isDir
          ? []
          : [
              {
                kind: "item" as const,
                id: "open-with",
                label: "Open with…",
                onSelect: () => beginOpenWith(entry),
              },
            ]),
        { kind: "sep" },
        ...(entry.isDir
          ? [
              {
                kind: "item" as const,
                id: "favorite",
                label: library.favorites.includes(entry.path)
                  ? "Remove from favorites"
                  : "Add to favorites",
                checked: library.favorites.includes(entry.path),
                onSelect: () =>
                  library.favorites.includes(entry.path)
                    ? forgetFavorite(entry.path)
                    : rememberFavorite(entry.path),
              },
              {
                kind: "item" as const,
                id: "pin",
                label: library.hiddenPins.includes(entry.path)
                  ? "Pin to sidebar"
                  : library.extraPins.includes(entry.path)
                    ? "Remove from sidebar"
                    : "Pin to sidebar",
                onSelect: () =>
                  library.extraPins.includes(entry.path)
                    ? unpinFromRail(entry.path, false)
                    : pinToRail(entry.path),
              },
            ]
          : []),
        {
          kind: "item",
          id: "copy-path",
          label: "Copy path",
          shortcut: "Ctrl+Shift+C",
          onSelect: () => void copyPath(),
        },
        {
          kind: "item",
          id: "rename",
          label: "Rename",
          icon: "rename",
          shortcut: "F2",
          onSelect: beginRename,
        },
        ...editOps(),
        { kind: "sep" },
        {
          kind: "item",
          id: "properties",
          label: "Properties",
          shortcut: "Alt+Enter",
          onSelect: () => showProperties(entry.path),
        },
      ];

      // Windows puts what other programs registered just above Properties, and
      // so does this.
      return [
        ...insertMenuBlock(
          filterMenu(items, isMenuShown),
          "properties",
          shellBlock(itemShellKey, entry.path, false),
        ),
        { kind: "sep" },
        customizeEntry("item"),
      ];
    },
    [
      active.sort.dir,
      active.sort.key,
      active.view,
      backgroundShellKey,
      beginOpenWith,
      beginRename,
      clipboard,
      copyPath,
      copySelection,
      customizeEntry,
      deleteSelection,
      forgetFavorite,
      isMenuShown,
      itemShellKey,
      library.favorites,
      library.extraPins,
      library.hiddenPins,
      location.path,
      pinToRail,
      rememberFavorite,
      unpinFromRail,
      pasteHere,
      newFolder,
      onOpen,
      openInNewTab,
      location,
      refresh,
      setSort,
      setView,
      shellBlock,
      showProperties,
      visible,
    ],
  );

  const toggleFullscreen = useCallback(() => windowChrome.toggleFullscreen(), []);

  const focusSearch = useCallback(() => {
    searchRef.current?.focus();
    searchRef.current?.select();
  }, []);

  const cycleTab = useCallback(
    (step: number) => {
      const index = tabs.findIndex((tab) => tab.id === activeIdRef.current);
      if (index === -1 || tabs.length < 2) return;
      const next = (index + step + tabs.length) % tabs.length;
      setActiveId(tabs[next]!.id);
    },
    [tabs],
  );

  /** Shift+F10: the menu opens over the focused row. */
  const openContextFromKeyboard = useCallback(() => {
    const focused = selection.focused;
    const row = focused
      ? document.querySelector(`.row[data-path="${CSS.escape(focused)}"]`)
      : null;
    const rect = row?.getBoundingClientRect();
    const entry = focused ? (visible.find((item) => item.path === focused) ?? null) : null;
    setContext({
      x: rect ? rect.left + 40 : window.innerWidth / 3,
      y: rect ? rect.top + rect.height : window.innerHeight / 3,
      entry,
    });
  }, [selection.focused, visible]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" || target?.tagName === "TEXTAREA";

      if (event.key === "F5" && !typing) {
        event.preventDefault();
        refresh();
        return;
      }
      // Ctrl+Shift comes first: N creates a folder, C copies the path, and the
      // number keys change the view.
      if ((event.ctrlKey || event.metaKey) && event.shiftKey) {
        const key = event.key.toLowerCase();
        if (key === "n") {
          event.preventDefault();
          newFolder();
          return;
        }
        if (key === "c") {
          event.preventDefault();
          void copyPath();
          return;
        }
        if (key === "tab") {
          event.preventDefault();
          cycleTab(-1);
          return;
        }
        if (event.key === "6") {
          event.preventDefault();
          setView("details");
          return;
        }
        if (event.key === "2") {
          event.preventDefault();
          setView("grid");
          return;
        }
        return;
      }

      if ((event.ctrlKey || event.metaKey) && !event.shiftKey) {
        const key = event.key.toLowerCase();
        switch (key) {
          case "t":
            event.preventDefault();
            newTab();
            return;
          case "w":
            event.preventDefault();
            closeTab(activeIdRef.current);
            return;
          case "b":
            event.preventDefault();
            setNavOpen((value) => !value);
            return;
          case "l":
            event.preventDefault();
            setEditSignal((value) => value + 1);
            return;
          case "f":
          case "e":
            event.preventDefault();
            focusSearch();
            return;
          case "c":
            event.preventDefault();
            copySelection("copy");
            return;
          case "x":
            event.preventDefault();
            copySelection("cut");
            return;
          case "v":
            event.preventDefault();
            pasteHere();
            return;
          case "tab":
            event.preventDefault();
            cycleTab(1);
            return;
          default:
            break;
        }
        // Ctrl+1..9 jumps to a tab, as in a browser and in Explorer's tabs.
        if (/^[1-9]$/.test(event.key)) {
          const index = Number(event.key) - 1;
          const target = tabs[index];
          if (target) {
            event.preventDefault();
            setActiveId(target.id);
          }
          return;
        }
        return;
      }

      if (event.altKey) {
        if (event.key === "ArrowLeft") {
          event.preventDefault();
          goBack();
        } else if (event.key === "ArrowRight") {
          event.preventDefault();
          goForward();
        } else if (event.key === "ArrowUp") {
          event.preventDefault();
          goUp();
        } else if (event.key.toLowerCase() === "d") {
          event.preventDefault();
          setEditSignal((value) => value + 1);
        } else if (event.key === "Enter") {
          event.preventDefault();
          showProperties();
        }
        return;
      }

      if (event.key === "Delete" && !typing && selection.selected.size > 0) {
        event.preventDefault();
        deleteSelection();
        return;
      }
      if (event.key === "F2" && !typing) {
        event.preventDefault();
        beginRename();
        return;
      }
      if (event.key === "F3" && !typing) {
        event.preventDefault();
        focusSearch();
        return;
      }
      if (event.key === "F4" && !typing) {
        event.preventDefault();
        setEditSignal((value) => value + 1);
        return;
      }
      if (event.key === "F11") {
        event.preventDefault();
        void toggleFullscreen();
        return;
      }
      // Shift+F10 is the keyboard's context menu key.
      if (event.shiftKey && event.key === "F10" && !typing) {
        event.preventDefault();
        openContextFromKeyboard();
        return;
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    beginRename,
    closeTab,
    copyPath,
    copySelection,
    cycleTab,
    deleteSelection,
    pasteHere,
    selection.selected.size,
    focusSearch,
    goBack,
    goForward,
    goUp,
    newFolder,
    newTab,
    openContextFromKeyboard,
    refresh,
    setView,
    showProperties,
    tabs,
  ]);

  // The two side buttons a mouse keeps for back and forward. Both engines report
  // them as buttons 3 and 4, and cancelling the default is what stops the
  // webview treating them as its own history: the history that matters is the
  // tab's, and the webview only ever holds one page. The action is on release,
  // so a press that wanders off the window does nothing.
  useEffect(() => {
    const side = (event: MouseEvent) => event.button === 3 || event.button === 4;
    const press = (event: MouseEvent) => {
      if (side(event)) event.preventDefault();
    };
    const release = (event: MouseEvent) => {
      if (!side(event)) return;
      event.preventDefault();
      if (event.button === 3) goBack();
      else goForward();
    };
    window.addEventListener("mousedown", press);
    window.addEventListener("mouseup", release);
    window.addEventListener("auxclick", press);
    return () => {
      window.removeEventListener("mousedown", press);
      window.removeEventListener("mouseup", release);
      window.removeEventListener("auxclick", press);
    };
  }, [goBack, goForward]);

  const pageTotal =
    location.kind === "home"
      ? folders.length
      : location.kind === "thisPc"
        ? volumes.length
        : dir.status === "ready"
          ? dir.total
          : dir.scanned;
  const pageSelected =
    location.kind === "thisPc"
      ? driveSelected
        ? 1
        : 0
      : location.kind === "home"
        ? 0
        : selection.selected.size;

  const breadcrumbGlyph =
    location.kind === "folder"
      ? volumes.some((volume) => volume.path === location.path)
        ? "drive"
        : "folder"
      : locationGlyph(location);

  return (
    <div
      className="app"
      data-window-focused={windowFocused ? "true" : "false"}
      // No shell context menus yet, and the webview's own menu exposes
      // reload and inspect, which do not belong in a file manager.
      onContextMenu={(event) => event.preventDefault()}
    >
      <ResizeEdges />

      <TitleBar
        tabs={tabs}
        activeId={activeId}
        onSelect={(id) => {
          setActiveId(id);
          setQuery("");
        }}
        onClose={closeTab}
        onNewTab={newTab}
      />

      <AddressBar
        location={location}
        glyph={breadcrumbGlyph}
        info={info}
        entries={visible}
        canBack={active.index > 0}
        canForward={active.index < active.history.length - 1}
        canUp={Boolean(parentOf(location, info))}
        onBack={goBack}
        onForward={goForward}
        onUp={goUp}
        onRefresh={refresh}
        onNavigate={navigate}
        query={query}
        onQuery={(value) => {
          setQuery(value);
          resetSelection();
        }}
        editSignal={editSignal}
        searchRef={searchRef}
      />

      <CommandBar
        view={active.view}
        onView={setView}
        mode={mode}
        onMode={setMode}
        sort={active.sort}
        onSort={setSort}
        filter={active.filter}
        onFilter={setFilter}
        groupByDate={active.groupByDate}
        onGroupByDate={setGroupByDate}
        navOpen={navOpen}
        onToggleNav={() => setNavOpen((value) => !value)}
        showHidden={showHidden}
        onToggleHidden={() => setShowHidden((value) => !value)}
        showProtected={showProtected}
        onToggleProtected={() => setShowProtected((value) => !value)}
        selectedCount={selection.selected.size}
        onSelectAll={() =>
          setSelection({
            selected: new Set(visible.map((entry) => entry.path)),
            focused: visible[0]?.path ?? null,
            anchor: visible[0]?.path ?? null,
          })
        }
        onCopyPath={() => void copyPath()}
        onRefresh={refresh}
        theme={theme.pref}
        onTheme={theme.setPref}
        isDefaultManager={manager?.isDefault === true}
        managerUnsupported={Boolean(manager?.unsupported)}
        promptDefaultManager={promptDefault}
        onSetDefaultManager={() => {
          setManagerError(null);
          setShowManagerPrompt(true);
        }}
        onTogglePromptManager={() => setPromptDefault((value) => !value)}
        canRestoreFileManager={
          manager?.isDefault === true && manager?.restoreSupported === true
        }
        onRestoreFileManager={restoreManager}
        winESupported={
          manager !== null && manager.desktopFile === null && !manager.unsupported
        }
        winEAgent={winEAgent}
        onToggleWinEAgent={toggleWinEAgent}
        onCheckUpdates={checkUpdates}
        reducedMemory={reducedMemory}
        onToggleReducedMemory={() => {
          const next = !reducedMemory;
          void setGraphicsSettings(next)
            .then((settings) => {
              setReducedMemory(settings.reducedMemory);
              flash(
                settings.needsRestart
                  ? "Saved. Restart Lyre to apply it."
                  : "Saved.",
              );
            })
            .catch((error: unknown) =>
              flash(error instanceof Error ? error.message : "Could not save."),
            );
        }}
        previewOpen={previewOpen}
        onTogglePreview={() => setPreviewOpen((value) => !value)}
        canRestorePins={
          library.hiddenPins.length > 0 || library.extraPins.length > 0
        }
        onRestorePins={restorePins}
        frameRate={showFrameRate}
        onToggleFrameRate={() => setShowFrameRate((value) => !value)}
        onCustomizeMenu={() =>
          setCustomize(
            itemTarget
              ? { scope: "item", key: itemShellKey }
              : { scope: "background", key: backgroundShellKey },
          )
        }
        notice={flash}
      />

      <div
        className="body"
        data-nav={navOpen ? "true" : "false"}
        data-preview={previewOpen && showsFiles ? "true" : "false"}
        style={
          navOpen
            ? ({ ["--nav-w" as string]: `${navWidth}px` } as React.CSSProperties)
            : undefined
        }
      >
        {navOpen ? (
          <NavRail
            places={places}
            pins={railPins}
            location={location}
            pictures={pictures}
            onNavigate={navigate}
            width={navWidth}
            onWidth={setNavWidth}
            onUnavailable={flash}
            onContextMenu={(event, place, builtIn) => {
              event.preventDefault();
              event.stopPropagation();
              setContext({
                x: event.clientX,
                y: event.clientY,
                entry: null,
                place,
                from: "rail",
                builtIn,
              });
            }}
          />
        ) : null}
        <main className="content" {...contentDropZone}>
          {location.kind === "home" && !searching ? (
            <HomeView
              places={quickAccess}
              favorites={favorites}
              onNavigate={(path) => navigate(folder(path))}
              onUnavailable={flash}
              onContextMenu={(event, place, from) => {
                if (!place) return;
                event.preventDefault();
                event.stopPropagation();
                setContext({
                  x: event.clientX,
                  y: event.clientY,
                  entry: null,
                  place,
                  from,
                });
              }}
            />
          ) : location.kind === "thisPc" && !searching ? (
            <ThisPcView
              places={volumes}
              selected={driveSelected}
              onSelect={setDriveSelected}
              onOpen={(path) => navigate(folder(path))}
              onEject={eject}
              ejecting={ejecting}
              onContextMenu={(event, place) => {
                event.preventDefault();
                event.stopPropagation();
                setContext({
                  x: event.clientX,
                  y: event.clientY,
                  entry: null,
                  place,
                  from: "places",
                });
              }}
            />
          ) : (
            <FileList
              entries={visible}
              version={dir.version}
              view={active.view}
              density={mode}
              sort={active.sort}
              onSort={setSort}
              selection={selection}
              onSelection={setSelection}
              onOpen={onOpen}
              onUp={goUp}
              status={
                searching
                  ? search.phase === "done" || search.items.length > 0
                    ? "ready"
                    : "loading"
                  : dir.status
              }
              error={searching ? search.error : dir.error}
              path={searching ? searchPath || location.kind : location.path}
              filtering={filtering}
              columns={columns}
              onColumns={setColumns}
              grouped={!searching && active.groupByDate}
              matched={searching ? highlights : undefined}
              sortable={!searching}
              editing={renaming}
              onRename={commitRename}
              onCancelRename={() => setRenaming(null)}
              onDragBegin={drag.begin}
              onContextMenu={(event, entry) => {
                event.preventDefault();
                event.stopPropagation();
                if (entry && !selection.selected.has(entry.path)) {
                  setSelection({
                    selected: new Set([entry.path]),
                    focused: entry.path,
                    anchor: entry.path,
                  });
                }
                setContext({ x: event.clientX, y: event.clientY, entry });
              }}
              onRetry={() => {
                setNonce((value) => value + 1);
                onRefreshPlaces();
              }}
            />
          )}
        </main>
        {previewOpen && showsFiles ? (
          <PreviewPane
            entry={previewEntry}
            onClose={() => setPreviewOpen(false)}
            dropZone={contentDropZone}
          />
        ) : null}

      </div>

      {showManagerPrompt && manager ? (
        <DefaultManagerPrompt
          state={manager}
          busy={managerBusy}
          error={managerError}
          onAccept={acceptManager}
          onDismiss={dismissManager}
          onNever={neverManager}
        />
      ) : null}

      <div className="drag-badge" hidden ref={drag.badge}>
        <span className="drag-badge-icon" aria-hidden="true" />
        <span data-role="label">Move</span>
      </div>

      {context ? (
        <ContextMenu
          x={context.x}
          y={context.y}
          items={
            context.place
              ? placeItems(context.place, context.from, context.builtIn)
              : contextItems(context.entry)
          }
          onClose={() => setContext(null)}
        />
      ) : null}

      {pendingPaste ? (
        <ConflictDialog
          destinationName={pendingPaste.destination.split(/[\\/]/).filter(Boolean).pop() ?? pendingPaste.destination}
          names={pendingPaste.names}
          onCancel={() => setPendingPaste(null)}
          onKeepBoth={() => {
            runPaste(pendingPaste.destination, pendingPaste.sources, pendingPaste.mode, "rename");
            setPendingPaste(null);
          }}
          onOverwrite={() => {
            runPaste(pendingPaste.destination, pendingPaste.sources, pendingPaste.mode, "overwrite");
            setPendingPaste(null);
          }}
        />
      ) : null}

      {openWith ? (
        <OpenWithDialog
          fileName={openWith.name}
          isDir={openWith.isDir}
          data={openWith.data}
          busy={openWith.busy}
          error={openWith.error}
          onOpen={launchWith}
          onUseSystem={() => {
            if (!openWith) return;
            void chooseAppWithSystem(openWith.path)
              .then(() => setOpenWith(null))
              .catch((error: unknown) =>
                setOpenWith((prev) =>
                  prev
                    ? {
                        ...prev,
                        error:
                          error instanceof Error
                            ? error.message
                            : "Could not open the picker.",
                      }
                    : prev,
                ),
              );
          }}
          onCancel={() => setOpenWith(null)}
        />
      ) : null}

      {properties ? (
        <PropertiesDialog
          properties={properties}
          onClose={() => setProperties(null)}
        />
      ) : null}

      {customize ? (
        <CustomizeMenuDialog
          scope={customize.scope}
          shell={
            customize.scope === "place"
              ? null
              : (shellCatalogs[customize.key] ?? null)
          }
          hidden={menuHidden}
          shown={menuShown}
          onToggle={toggleMenuItem}
          onReset={resetMenu}
          onClose={() => setCustomize(null)}
        />
      ) : null}

      {update ? (
        <UpdateDialog
          offer={update.offer}
          phase={update.phase}
          downloaded={update.downloaded}
          total={update.total}
          error={update.error}
          onInstall={() => beginUpdate(update.offer)}
          onRestart={() => void restartApp()}
          onClose={() => setUpdate(null)}
        />
      ) : null}

      <StatusBar
        status={location.kind === "folder" || location.kind === "gallery" ? dir.status : "ready"}
        scanned={dir.scanned}
        total={pageTotal}
        elapsedMs={
          location.kind === "folder" || location.kind === "gallery"
            ? dir.elapsedMs
            : null
        }
        selectedCount={pageSelected}
        visibleCount={visible.length}
        filtering={filtering}
        notice={notice}
        search={searching ? search : null}
        showFrameRate={showFrameRate}
        transfer={transfer}
        onCancelTransfer={() => {
          // The bar stays until the operation actually stops, because the item
          // being written has to finish its chunk and a partial file has to be
          // removed. Saying "Stopping" is honest; hiding the bar is not.
          setTransfer((current) =>
            current ? { ...current, stopping: true } : current,
          );
          transferControl.current?.cancel();
        }}
      />
    </div>
  );
}

export { locationLabel };
