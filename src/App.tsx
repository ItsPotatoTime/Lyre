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
  deleteToTrash,
  pasteInto,
  getHomeDir,
  getPathInfo,
  getPlaces,
  openPath,
  pathProperties,
  renamePath,
  appsForFile,
  openWithApp,
  setDefaultApp,
  type OpenWith,
  type PathProperties,
} from "@/lib/ipc";
import { useDirectory } from "@/lib/useDirectory";
import { matchIndex, useSearch } from "@/lib/useSearch";
import { useReveal } from "@/lib/hooks";
import { useTheme } from "@/lib/theme";
import { loadPrefs, savePrefs, type Prefs } from "@/lib/prefs";
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
import { PropertiesDialog } from "@/components/PropertiesDialog";
import { OpenWithDialog } from "@/components/OpenWithDialog";
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
      setHome(start);
      setPlaces(list);
    });
    return () => {
      cancelled = true;
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
  const [activeId, setActiveId] = useState<string>(tabs[0]!.id);
  const [query, setQuery] = useState("");
  const [navOpen, setNavOpen] = useState(prefs.navOpen);
  const [navWidth, setNavWidth] = useState(prefs.navWidth);
  const [showHidden, setShowHidden] = useState(prefs.showHidden);
  const [mode, setMode] = useState<Mode>(prefs.mode);
  const [columns, setColumns] = useState<ColumnWidths>(prefs.columns);
  const [notice, setNotice] = useState<string | null>(null);
  const [info, setInfo] = useState<PathInfo | null>(null);
  const [nonce, setNonce] = useState(0);
  const [editSignal, setEditSignal] = useState(0);
  const [driveSelected, setDriveSelected] = useState<string | null>(null);
  const [windowFocused, setWindowFocused] = useState(true);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [clipboard, setClipboard] = useState<{
    mode: "copy" | "cut";
    paths: string[];
  } | null>(null);
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
    entry: Entry | null;
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
  const dir = useDirectory(
    showsFiles ? location.path : "",
    showHidden,
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
  const search = useSearch(searchPath, roots, query, showHidden, searching);
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
    savePrefs({
      sort: active.sort,
      view: active.view,
      groupByDate: active.groupByDate,
      mode,
      showHidden,
      navOpen,
      navWidth,
      columns,
    });
  }, [
    active.sort,
    active.view,
    active.groupByDate,
    mode,
    showHidden,
    navOpen,
    navWidth,
    columns,
  ]);

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
    void pasteInto(location.path, clipboard.paths, clipboard.mode, {
      onProgress: (done, total, name) => {
        if (total > 1) flash(`Pasting ${name} (${done + 1} of ${total})`);
      },
      onDone: (pasted, skipped) => {
        refreshAfterChange();
        if (clipboard.mode === "cut") setClipboard(null);
        flash(
          skipped.length > 0
            ? `Pasted ${pasted}, ${skipped.length} could not be copied`
            : `Pasted ${pasted} item${pasted === 1 ? "" : "s"}`,
        );
      },
      onFailed: (message) => flash(message),
    }).catch((error: unknown) =>
      flash(error instanceof Error ? error.message : "Could not paste."),
    );
  }, [clipboard, flash, location.path, refreshAfterChange, showsFiles]);

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
    void deleteToTrash(paths)
      .then((moved) => {
        resetSelection();
        refreshAfterChange();
        flash(`Moved ${moved} item${moved === 1 ? "" : "s"} to the trash`);
      })
      .catch((error: unknown) =>
        flash(error instanceof Error ? error.message : "Could not delete."),
      );
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
        return [
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
      }

      return [
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
    },
    [
      active.sort.dir,
      active.sort.key,
      active.view,
      beginOpenWith,
      beginRename,
      clipboard,
      copyPath,
      copySelection,
      deleteSelection,
      location.path,
      pasteHere,
      newFolder,
      onOpen,
      openInNewTab,
      location,
      refresh,
      setSort,
      setView,
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
        notice={flash}
      />

      <div
        className="body"
        data-nav={navOpen ? "true" : "false"}
        style={
          navOpen
            ? ({ ["--nav-w" as string]: `${navWidth}px` } as React.CSSProperties)
            : undefined
        }
      >
        {navOpen ? (
          <NavRail
            places={places}
            location={location}
            pictures={pictures}
            onNavigate={navigate}
            width={navWidth}
            onWidth={setNavWidth}
            onUnavailable={flash}
          />
        ) : null}
        <main className="content">
          {location.kind === "home" && !searching ? (
            <HomeView
              places={quickAccess}
              onNavigate={(path) => navigate(folder(path))}
              onUnavailable={flash}
            />
          ) : location.kind === "thisPc" && !searching ? (
            <ThisPcView
              places={volumes}
              selected={driveSelected}
              onSelect={setDriveSelected}
              onOpen={(path) => navigate(folder(path))}
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
      </div>

      {context ? (
        <ContextMenu
          x={context.x}
          y={context.y}
          items={contextItems(context.entry)}
          onClose={() => setContext(null)}
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
          onCancel={() => setOpenWith(null)}
        />
      ) : null}

      {properties ? (
        <PropertiesDialog
          properties={properties}
          onClose={() => setProperties(null)}
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
      />
    </div>
  );
}

export { locationLabel };
