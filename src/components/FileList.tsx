import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Entry, Selection, SortKey, SortState, ViewMode } from "@/types";
import type { DirStatus } from "@/lib/useDirectory";
import { metricsFor, type Mode } from "@/lib/modes";
import { formatDate, formatSize, typeLabel } from "@/lib/format";
import { sortLabel } from "@/lib/sort";
import { useDelayed, useVirtual } from "@/lib/hooks";
import { FileGlyph } from "./FileGlyph";
import { Icon } from "./Icons";

const COLUMNS: {
  key: SortKey;
  label: string;
  width: string;
  align: "start" | "end";
}[] = [
  { key: "name", label: "Name", width: "minmax(200px, 1fr)", align: "start" },
  { key: "modified", label: "Date modified", width: "184px", align: "start" },
  { key: "type", label: "Type", width: "150px", align: "start" },
  { key: "size", label: "Size", width: "104px", align: "end" },
];

const TEMPLATE = COLUMNS.map((column) => column.width).join(" ");

function findIndex(entries: Entry[], path: string): number {
  for (let i = 0; i < entries.length; i += 1) {
    if (entries[i]!.path === path) return i;
  }
  return -1;
}

export function FileList({
  entries,
  version,
  view,
  density,
  sort,
  onSort,
  selection,
  onSelection,
  onOpen,
  onUp,
  status,
  error,
  path,
  filtering,
  onRetry,
}: {
  entries: Entry[];
  version: number;
  view: ViewMode;
  density: Mode;
  sort: SortState;
  onSort: (next: SortState) => void;
  selection: Selection;
  onSelection: (next: Selection) => void;
  onOpen: (entry: Entry) => void;
  onUp: () => void;
  status: DirStatus;
  error: string | null;
  path: string;
  /** True when a search is scoping the list, so an empty result says why. */
  filtering: boolean;
  onRetry: () => void;
}) {
  const metrics = metricsFor(density);
  const rowHeight = view === "grid" ? metrics.tileHeight : metrics.rowHeight;
  const [columns, setColumns] = useState(1);
  // The virtualiser counts rows, not items: in grid view one row holds a whole
  // line of tiles, so the scroll height and the render window are both in row
  // units. Getting this wrong makes the scrollbar lie about the folder size.
  const rowCount =
    view === "grid" ? Math.ceil(entries.length / Math.max(1, columns)) : entries.length;
  const { ref, bounds } = useVirtual(rowCount, rowHeight, view === "grid" ? 1 : 8);
  const scrolling = ref as React.RefObject<HTMLDivElement | null>;

  useEffect(() => {
    if (view !== "grid") {
      setColumns(1);
      return;
    }
    const element = scrolling.current;
    if (!element) return;
    const update = () =>
      setColumns(
        Math.max(1, Math.floor((element.clientWidth - 12) / metrics.tileWidth)),
      );
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [view, metrics.tileWidth, scrolling]);

  const firstIndex = view === "grid" ? bounds.start * columns : bounds.start;
  const lastIndex =
    view === "grid"
      ? Math.min(entries.length, bounds.end * columns)
      : Math.min(entries.length, bounds.end);

  // A new directory starts at the top, as in Explorer.
  useEffect(() => {
    const element = scrolling.current;
    if (element) element.scrollTop = 0;
  }, [path, view, scrolling]);

  // A linear scan, not a path-keyed Map: during a long scan the row array is
  // republished many times a second, and building a 50,000 entry Map on each
  // republish costs more in allocation than these lookups ever do. This runs
  // only when the focused row or the array changes.
  const focusedIndex = useMemo(
    () => (selection.focused === null ? -1 : findIndex(entries, selection.focused)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entries, version, selection.focused],
  );
  const anchorIndex = useMemo(
    () => (selection.anchor === null ? -1 : findIndex(entries, selection.anchor)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entries, version, selection.anchor],
  );

  const selectSingle = useCallback(
    (entry: Entry) => {
      onSelection({
        selected: new Set([entry.path]),
        focused: entry.path,
        anchor: entry.path,
      });
    },
    [onSelection],
  );

  const selectRange = useCallback(
    (to: number, base: number) => {
      const from = Math.min(base, to);
      const end = Math.max(base, to);
      const next = new Set<string>();
      for (let i = from; i <= end; i += 1) next.add(entries[i]!.path);
      onSelection({
        selected: next,
        focused: entries[to]?.path ?? null,
        anchor: entries[base]?.path ?? null,
      });
    },
    [entries, onSelection],
  );

  const toggle = useCallback(
    (entry: Entry) => {
      const next = new Set(selection.selected);
      if (next.has(entry.path)) next.delete(entry.path);
      else next.add(entry.path);
      onSelection({ selected: next, focused: entry.path, anchor: entry.path });
    },
    [onSelection, selection.selected],
  );

  const focusIndex = useCallback(
    (next: number, extend: boolean) => {
      if (entries.length === 0) return;
      const clamped = Math.min(entries.length - 1, Math.max(0, next));
      const entry = entries[clamped]!;
      const base = extend && anchorIndex >= 0 ? anchorIndex : clamped;
      if (extend) {
        selectRange(clamped, base);
      } else {
        selectSingle(entry);
      }
    },
    [anchorIndex, entries, selectRange, selectSingle],
  );

  // Reveal the focused row. Instant, never smooth: this is a keyboard action.
  useEffect(() => {
    const element = scrolling.current;
    if (!element || focusedIndex < 0) return;
    const row = view === "grid" ? Math.floor(focusedIndex / columns) : focusedIndex;
    const top = row * rowHeight;
    if (top < element.scrollTop) {
      element.scrollTop = top;
    } else if (top + rowHeight > element.scrollTop + element.clientHeight) {
      element.scrollTop = top + rowHeight - element.clientHeight;
    }
  }, [focusedIndex, rowHeight, view, columns, scrolling]);

  // Type-ahead: Explorer's incremental jump-to-name.
  const typeahead = useRef({ buffer: "", at: 0 });

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      if (event.metaKey || event.ctrlKey) {
        if (event.key.toLowerCase() === "a") {
          event.preventDefault();
          onSelection({
            selected: new Set(entries.map((entry) => entry.path)),
            focused: selection.focused ?? entries[0]?.path ?? null,
            anchor: entries[0]?.path ?? null,
          });
        }
        return;
      }

      const total = entries.length;
      if (total === 0) return;
      const advance = view === "grid" ? columns : 1;
      const page = Math.max(
        1,
        Math.floor((scrolling.current?.clientHeight ?? 400) / rowHeight) - 1,
      ) * advance;

      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          focusIndex(focusedIndex < 0 ? 0 : focusedIndex + advance, event.shiftKey);
          return;
        case "ArrowUp":
          event.preventDefault();
          focusIndex(focusedIndex < 0 ? 0 : focusedIndex - advance, event.shiftKey);
          return;
        case "ArrowRight":
          if (view !== "grid") return;
          event.preventDefault();
          focusIndex(focusedIndex < 0 ? 0 : focusedIndex + 1, event.shiftKey);
          return;
        case "ArrowLeft":
          if (view !== "grid") return;
          event.preventDefault();
          focusIndex(focusedIndex < 0 ? 0 : focusedIndex - 1, event.shiftKey);
          return;
        case "Home":
          event.preventDefault();
          focusIndex(0, event.shiftKey);
          return;
        case "End":
          event.preventDefault();
          focusIndex(total - 1, event.shiftKey);
          return;
        case "PageDown":
          event.preventDefault();
          focusIndex((focusedIndex < 0 ? 0 : focusedIndex) + page, event.shiftKey);
          return;
        case "PageUp":
          event.preventDefault();
          focusIndex((focusedIndex < 0 ? 0 : focusedIndex) - page, event.shiftKey);
          return;
        case "Enter": {
          event.preventDefault();
          const entry = entries[focusedIndex];
          if (entry) onOpen(entry);
          return;
        }
        case " ":
          event.preventDefault();
          if (focusedIndex >= 0) {
            const entry = entries[focusedIndex]!;
            if (selection.selected.has(entry.path) && selection.selected.size > 1) {
              const next = new Set(selection.selected);
              next.delete(entry.path);
              onSelection({ selected: next, focused: entry.path, anchor: entry.path });
            } else {
              toggle(entry);
            }
          }
          return;
        case "Escape":
          event.preventDefault();
          onSelection({ selected: new Set(), focused: null, anchor: null });
          return;
        case "Backspace":
          event.preventDefault();
          onUp();
          return;
      }

      // Printable characters accumulate for type-ahead.
      if (event.key.length === 1) {
        const now = performance.now();
        const state = typeahead.current;
        const buffer = now - state.at > 700 ? event.key : state.buffer + event.key;
        typeahead.current = { buffer, at: now };
        const needle = buffer.toLowerCase();
        const start = Math.max(0, focusedIndex);
        // Search forward from the focused row, wrapping once.
        for (let step = 0; step < total; step += 1) {
          const index = (start + step) % total;
          if (entries[index]!.name.toLowerCase().startsWith(needle)) {
            focusIndex(index, false);
            return;
          }
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    columns,
    entries,
    focusIndex,
    focusedIndex,
    onOpen,
    onSelection,
    onUp,
    rowHeight,
    scrolling,
    selection.focused,
    selection.selected,
    toggle,
    view,
  ]);

  const ready = status === "ready";
  const waiting = useDelayed(status === "loading" && entries.length === 0);
  const showSkeleton = !ready && entries.length === 0 && waiting;
  const showEmpty = ready && entries.length === 0;
  const showError = status === "error";

  return (
    <>
      {view === "details" ? (
        <div
          className="columns"
          style={{
            gridTemplateColumns: TEMPLATE,
            height: metrics.columnHeaderHeight,
          }}
        >
          {COLUMNS.map((column) => (
            <button
              key={column.key}
              type="button"
              className="col"
              data-align={column.align}
              data-sorted={sort.key === column.key ? "true" : "false"}
              onClick={() =>
                onSort({
                  key: column.key,
                  dir: sort.key === column.key && sort.dir === 1 ? -1 : 1,
                })
              }
              title={`Sort by ${column.label.toLowerCase()}`}
            >
              <span className="col-label">{column.label}</span>
              {sort.key === column.key ? (
                <Icon
                  name={sort.dir === 1 ? "chevronUp" : "chevronDown"}
                  size={10}
                  strokeWidth={1.8}
                  className="col-sort"
                />
              ) : null}
            </button>
          ))}
        </div>
      ) : null}

      {showError ? (
        <div className="state">
          <Icon name="folder" size={40} strokeWidth={1} />
          <div className="state-title">This folder can’t be opened</div>
          <div className="state-body">{error}</div>
          <button type="button" className="state-action" onClick={onRetry}>
            Try again
          </button>
        </div>
      ) : showSkeleton ? (
        <div className="skeleton" aria-live="polite" aria-label="Reading folder">
          {Array.from({ length: 12 }, (_, i) => (
            <div className="skeleton-row" key={i}>
              <div
                className="skeleton-bar"
                style={{ width: 16, height: 16, borderRadius: 3 }}
              />
              <div
                className="skeleton-bar"
                style={{ width: `${30 + ((i * 37) % 40)}%` }}
              />
            </div>
          ))}
        </div>
      ) : showEmpty ? (
        <div className="state">
          <Icon name="folder" size={40} strokeWidth={1} />
          {filtering ? (
            <>
              <div className="state-title">No items match your search</div>
              <div className="state-body">
                Nothing in this folder matches what you typed. Clear the search
                to see everything again.
              </div>
            </>
          ) : (
            <>
              <div className="state-title">This folder is empty</div>
              <div className="state-body">
                There is nothing here. Hidden items may still be present: turn
                them on from the View menu.
              </div>
            </>
          )}
        </div>
      ) : (
        <div
          className={view === "grid" ? "grid" : "list"}
          ref={scrolling}
          role="listbox"
          aria-label="Folder contents"
          aria-multiselectable="true"
          tabIndex={0}
          style={
            view === "grid"
              ? undefined
              : ({ ["--row-h" as string]: `${rowHeight}px` } as React.CSSProperties)
          }
          onPointerDown={(event) => {
            // Clicking the empty area below the rows clears the selection.
            if (event.target === event.currentTarget) {
              onSelection({ selected: new Set(), focused: null, anchor: null });
            }
          }}
        >
          <div
            className={view === "grid" ? "grid-spacer" : "list-spacer"}
            style={{ height: rowCount * rowHeight }}
          >
            {view === "details"
              ? entries.slice(firstIndex, lastIndex).map((entry, offset) => {
                  const index = firstIndex + offset;
                  return (
                    <DetailRow
                      key={entry.path}
                      entry={entry}
                      index={index}
                      rowHeight={rowHeight}
                      selected={selection.selected.has(entry.path)}
                      focused={index === focusedIndex}
                      onPointerDown={(event) => {
                        if (event.button !== 0) return;
                        if (event.shiftKey && anchorIndex >= 0) {
                          selectRange(index, anchorIndex);
                        } else if (event.ctrlKey || event.metaKey) {
                          toggle(entry);
                        } else {
                          selectSingle(entry);
                        }
                      }}
                      onDoubleClick={() => onOpen(entry)}
                    />
                  );
                })
              : entries.slice(firstIndex, lastIndex).map((entry, offset) => {
                  const index = firstIndex + offset;
                  return (
                    <GridTile
                      key={entry.path}
                      entry={entry}
                      index={index}
                      columns={columns}
                      tile={{
                        width: metrics.tileWidth,
                        height: metrics.tileHeight,
                      }}
                      selected={selection.selected.has(entry.path)}
                      focused={index === focusedIndex}
                      onPointerDown={(event) => {
                        if (event.button !== 0) return;
                        if (event.shiftKey && anchorIndex >= 0) {
                          selectRange(index, anchorIndex);
                        } else if (event.ctrlKey || event.metaKey) {
                          toggle(entry);
                        } else {
                          selectSingle(entry);
                        }
                      }}
                      onDoubleClick={() => onOpen(entry)}
                    />
                  );
                })}
          </div>
        </div>
      )}
    </>
  );
}

function DetailRow({
  entry,
  index,
  rowHeight,
  selected,
  focused,
  onPointerDown,
  onDoubleClick,
}: {
  entry: Entry;
  index: number;
  rowHeight: number;
  selected: boolean;
  focused: boolean;
  onPointerDown: (event: React.PointerEvent<HTMLDivElement>) => void;
  onDoubleClick: () => void;
}) {
  return (
    <div
      className="row"
      role="option"
      aria-selected={selected}
      data-selected={selected ? "true" : "false"}
      data-focused={focused ? "true" : "false"}
      style={{
        gridTemplateColumns: TEMPLATE,
        height: rowHeight,
        transform: `translateY(${index * rowHeight}px)`,
      }}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
    >
      <div className="cell cell-name">
        <FileGlyph kind={entry.kind} size={16} isSymlink={entry.isSymlink} />
        <span className="cell-text" title={entry.name}>
          {entry.name}
        </span>
      </div>
      <div className="cell cell-muted tabular">{formatDate(entry.modifiedMs)}</div>
      <div className="cell cell-muted">
        <span className="cell-text">{typeLabel(entry)}</span>
      </div>
      <div className="cell cell-muted tabular" data-align="end">
        {formatSize(entry.size, entry.isDir)}
      </div>
    </div>
  );
}

function GridTile({
  entry,
  index,
  columns,
  tile,
  selected,
  focused,
  onPointerDown,
  onDoubleClick,
}: {
  entry: Entry;
  index: number;
  columns: number;
  tile: { width: number; height: number };
  selected: boolean;
  focused: boolean;
  onPointerDown: (event: React.PointerEvent<HTMLDivElement>) => void;
  onDoubleClick: () => void;
}) {
  const column = index % columns;
  const row = Math.floor(index / columns);
  return (
    <div
      className="tile"
      role="option"
      aria-selected={selected}
      data-selected={selected ? "true" : "false"}
      data-focused={focused ? "true" : "false"}
      style={{
        width: tile.width,
        height: tile.height,
        transform: `translate(${column * tile.width}px, ${row * tile.height}px)`,
      }}
      title={entry.name}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
    >
      <div className="tile-glyph">
        <FileGlyph kind={entry.kind} size={48} isSymlink={entry.isSymlink} />
      </div>
      <div className="tile-label">{entry.name}</div>
    </div>
  );
}

export { sortLabel };
