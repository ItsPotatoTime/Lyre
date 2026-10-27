import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Entry, PathInfo, Place, Selection, Tab, ViewMode } from "@/types";
import { getHomeDir, getPathInfo, getPlaces } from "@/lib/ipc";
import { useDirectory } from "@/lib/useDirectory";
import { useReveal } from "@/lib/hooks";
import { useTheme } from "@/lib/theme";
import type { Mode } from "@/lib/modes";
import { TitleBar, ResizeEdges } from "@/components/TitleBar";
import { CommandBar } from "@/components/CommandBar";
import { AddressBar } from "@/components/AddressBar";
import { NavRail } from "@/components/NavRail";
import { FileList } from "@/components/FileList";
import { StatusBar } from "@/components/StatusBar";
import { Icon } from "@/components/Icons";

let tabSeq = 0;

function makeTab(path: string): Tab {
  tabSeq += 1;
  return {
    id: `tab-${tabSeq}`,
    path,
    history: [path],
    index: 0,
    sort: { key: "name", dir: 1 },
    view: "details",
  };
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

  return <Browser home={home} places={places} onRefreshPlaces={setPlaces} />;
}

function Browser({
  home,
  places,
  onRefreshPlaces,
}: {
  home: string;
  places: Place[];
  onRefreshPlaces: (next: Place[]) => void;
}) {
  const theme = useTheme();
  useReveal();

  const [tabs, setTabs] = useState<Tab[]>(() => [makeTab(home)]);
  const [activeId, setActiveId] = useState<string>(tabs[0]!.id);
  const [query, setQuery] = useState("");
  const [navOpen, setNavOpen] = useState(true);
  const [navWidth, setNavWidth] = useState(240);
  const [showHidden, setShowHidden] = useState(false);
  const [mode, setMode] = useState<Mode>("explorer");
  const [notice, setNotice] = useState<string | null>(null);
  const [info, setInfo] = useState<PathInfo | null>(null);
  const [nonce, setNonce] = useState(0);
  const [editSignal, setEditSignal] = useState(0);
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

  const dir = useDirectory(active.path, showHidden, active.sort, nonce);

  const patchTab = useCallback(
    (id: string, patch: (tab: Tab) => Tab) => {
      setTabs((prev) => prev.map((tab) => (tab.id === id ? patch(tab) : tab)));
    },
    [],
  );

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
    (path: string, options?: { replace?: boolean }) => {
      if (!path) return;
      patchTab(activeIdRef.current, (tab) => {
        const history = tab.history.slice(0, tab.index + 1);
        if (options?.replace) history[history.length - 1] = path;
        else if (history[history.length - 1] !== path) history.push(path);
        else return { ...tab, path };
        return { ...tab, path, history, index: history.length - 1 };
      });
      setQuery("");
      resetSelection();
    },
    [patchTab, resetSelection],
  );

  const refresh = useCallback(() => {
    setNonce((value) => value + 1);
    void getPathInfo(active.path).then(setInfo);
    flash("Refreshed");
  }, [active.path, flash]);

  // Path info drives the breadcrumb, so it is refreshed with the listing.
  useEffect(() => {
    let cancelled = false;
    void getPathInfo(active.path).then((next) => {
      if (!cancelled) setInfo(next);
    });
    return () => {
      cancelled = true;
    };
  }, [active.path, nonce]);

  // Selection belongs to a folder; carrying it across folders would let the
  // status bar report items that are no longer on screen.
  useEffect(() => {
    resetSelection();
  }, [active.path, resetSelection]);

  const goBack = useCallback(() => {
    patchTab(activeIdRef.current, (tab) =>
      tab.index > 0
        ? { ...tab, index: tab.index - 1, path: tab.history[tab.index - 1]! }
        : tab,
    );
  }, [patchTab]);

  const goForward = useCallback(() => {
    patchTab(activeIdRef.current, (tab) =>
      tab.index < tab.history.length - 1
        ? { ...tab, index: tab.index + 1, path: tab.history[tab.index + 1]! }
        : tab,
    );
  }, [patchTab]);

  const goUp = useCallback(() => {
    if (info?.parent) navigate(info.parent);
  }, [info?.parent, navigate]);

  const newTab = useCallback(() => {
    // Explorer opens a new tab at Home, not at the folder being left behind.
    const tab = makeTab(home);
    setTabs((prev) => [...prev, tab]);
    setActiveId(tab.id);
    setQuery("");
    resetSelection();
  }, [home, resetSelection]);

  const closeTab = useCallback(
    (id: string) => {
      const index = tabs.findIndex((tab) => tab.id === id);
      // Explorer refuses to close the last tab; the window closes instead.
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
    (view: ViewMode) => patchTab(activeIdRef.current, (tab) => ({ ...tab, view })),
    [patchTab],
  );

  const setSort = useCallback(
    (sort: Tab["sort"]) => patchTab(activeIdRef.current, (tab) => ({ ...tab, sort })),
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

  // Application shortcuts. Row and selection keys live in the file list; these
  // are the ones that must work regardless of what has focus.
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

  const filtering = query.trim().length > 0;
  const visible = useMemo(() => {
    if (!filtering) return dir.entries;
    const needle = query.trim().toLowerCase();
    return dir.entries.filter((entry) =>
      entry.name.toLowerCase().includes(needle),
    );
    // `dir.version` advances when the streamed array is mutated in place.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dir.entries, dir.version, query, filtering]);

  const onOpen = useCallback(
    (entry: Entry) => {
      if (entry.isDir) {
        navigate(entry.path);
      } else {
        flash("Opening files arrives with shell integration.");
      }
    },
    [flash, navigate],
  );

  return (
    <div className="app">
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

      <CommandBar
        view={active.view}
        onView={setView}
        sort={active.sort}
        onSort={setSort}
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
        mode={mode}
        onMode={setMode}
        theme={theme.pref}
        onTheme={theme.setPref}
        notice={flash}
      />

      <AddressBar
        info={info}
        canBack={active.index > 0}
        canForward={active.index < active.history.length - 1}
        canUp={Boolean(info?.parent)}
        onBack={goBack}
        onForward={goForward}
        onUp={goUp}
        onNavigate={(path) => navigate(path)}
        onRefresh={refresh}
        query={query}
        onQuery={(value) => {
          setQuery(value);
          resetSelection();
        }}
        editSignal={editSignal}
        searchRef={searchRef}
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
            activePath={active.path}
            onNavigate={navigate}
            width={navWidth}
            onWidth={setNavWidth}
          />
        ) : null}
        <main className="content">
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
            path={active.path}
            filtering={filtering}
            onRetry={() => {
              setNonce((value) => value + 1);
              void getPlaces().then(onRefreshPlaces);
            }}
          />
        </main>
      </div>

      <StatusBar
        status={dir.status}
        scanned={dir.scanned}
        total={dir.status === "ready" ? dir.total : dir.scanned}
        elapsedMs={dir.elapsedMs}
        selectedCount={selection.selected.size}
        visibleCount={visible.length}
        filtering={filtering}
        notice={notice ?? null}
      />
    </div>
  );
}
