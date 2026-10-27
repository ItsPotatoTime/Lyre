import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ColumnWidths,
  Entry,
  KindFilter,
  Location,
  PathInfo,
  Place,
  Selection,
  SortState,
  Tab,
  ViewMode,
} from "@/types";
import { getHomeDir, getPathInfo, getPlaces, openPath } from "@/lib/ipc";
import { useDirectory } from "@/lib/useDirectory";
import { matchIndex, useSearch } from "@/lib/useSearch";
import { useReveal } from "@/lib/hooks";
import { useTheme } from "@/lib/theme";
import { DEFAULT_COLUMNS, type Mode } from "@/lib/modes";
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
import { Icon } from "@/components/Icons";

let tabSeq = 0;

function makeTab(location: Location): Tab {
  tabSeq += 1;
  return {
    id: `tab-${tabSeq}`,
    location,
    history: [location],
    index: 0,
    sort: { key: "name", dir: 1 },
    view: "details",
    filter: "all",
    groupByDate: false,
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

  const [tabs, setTabs] = useState<Tab[]>(() => [
    makeTab(homeLocation(home)),
  ]);
  const [activeId, setActiveId] = useState<string>(tabs[0]!.id);
  const [query, setQuery] = useState("");
  const [navOpen, setNavOpen] = useState(true);
  const [navWidth, setNavWidth] = useState(160);
  const [showHidden, setShowHidden] = useState(false);
  const [mode, setMode] = useState<Mode>("explorer");
  const [columns, setColumns] = useState<ColumnWidths>(DEFAULT_COLUMNS);
  const [notice, setNotice] = useState<string | null>(null);
  const [info, setInfo] = useState<PathInfo | null>(null);
  const [nonce, setNonce] = useState(0);
  const [editSignal, setEditSignal] = useState(0);
  const [driveSelected, setDriveSelected] = useState<string | null>(null);
  const [windowFocused, setWindowFocused] = useState(true);
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
    const tab = makeTab(homeLocation(home));
    setTabs((prev) => [...prev, tab]);
    setActiveId(tab.id);
    setQuery("");
    resetSelection();
  }, [home, resetSelection]);

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
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey) {
        switch (event.key.toLowerCase()) {
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
            event.preventDefault();
            searchRef.current?.focus();
            searchRef.current?.select();
            return;
          default:
            break;
        }
      }
      if ((event.ctrlKey || event.metaKey) && event.shiftKey) {
        if (event.key === "1") {
          event.preventDefault();
          setView("details");
        } else if (event.key === "2") {
          event.preventDefault();
          setView("grid");
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
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [closeTab, goBack, goForward, goUp, newTab, refresh, setView]);

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
          {searching ? (
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
              status={search.phase === "done" || search.items.length > 0 ? "ready" : "loading"}
              error={search.error}
              path={searchPath || location.kind}
              filtering
              columns={columns}
              onColumns={setColumns}
              grouped={false}
              matched={highlights}
              sortable={false}
              onRetry={refresh}
            />
          ) : location.kind === "home" ? (
            <HomeView
              places={quickAccess}
              onNavigate={(path) => navigate(folder(path))}
              onUnavailable={flash}
            />
          ) : location.kind === "thisPc" ? (
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
              status={dir.status}
              error={dir.error}
              path={location.path}
              filtering={filtering}
              columns={columns}
              onColumns={setColumns}
              grouped={active.groupByDate}
              matched={searching ? highlights : undefined}
              sortable={!searching}
              onRetry={() => {
                setNonce((value) => value + 1);
                onRefreshPlaces();
              }}
            />
          )}
        </main>
      </div>

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
