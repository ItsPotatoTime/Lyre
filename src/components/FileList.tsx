import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ColumnWidths,
  Entry,
  Selection,
  SortKey,
  SortState,
  ViewMode,
} from "@/types";
import type { DirStatus } from "@/lib/useDirectory";
import { MIN_COLUMN, metricsFor, type Mode } from "@/lib/modes";
import { formatDate, formatSize, typeLabel } from "@/lib/format";
import { useDelayed, useVirtual } from "@/lib/hooks";
import { FileGlyph } from "./FileGlyph";
import { Icon } from "./Icons";

const COLUMNS: { key: SortKey; label: string; min: number }[] = [
  { key: "name", label: "Name", min: 160 },
  { key: "modified", label: "Date modified", min: MIN_COLUMN },
  { key: "type", label: "Type", min: MIN_COLUMN },
  { key: "size", label: "Size", min: 46 },
];

/** One display row: either a date group header or a file. */
type Row =
  | { kind: "group"; label: string; count: number; key: string }
  | { kind: "entry"; entry: Entry; index: number };

const DAY = 86_400_000;

/** Explorer's date buckets, in the order it writes them. */
function bucketOf(ms: number | null, now: number): string {
  if (ms === null) return "A long time ago";
  const age = now - ms;
  if (age < DAY) return "Today";
  if (age < 2 * DAY) return "Yesterday";
  if (age < 7 * DAY) return "Last week";
  if (age < 30 * DAY) return "Earlier this month";
  if (age < 60 * DAY) return "Last month";
  if (age < 365 * DAY) return "Earlier this year";
  return "A long time ago";
}

function buildGroups(entries: Entry[]): Row[] {
  const now = Date.now();
  const rows: Row[] = [];
  let bucket: string | null = null;
  let group: Extract<Row, { kind: "group" }> | null = null;

  for (let i = 0; i < entries.length; i += 1) {
    const entry = entries[i]!;
    const next = bucketOf(entry.modifiedMs, now);
    if (next !== bucket) {
      bucket = next;
      group = { kind: "group", label: next, count: 0, key: `${next}-${i}` };
      rows.push(group);
    }
    if (group) group.count += 1;
    rows.push({ kind: "entry", entry, index: i });
  }
  return rows;
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
  columns,
  onColumns,
  grouped,
  matched,
  sortable = true,
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
  filtering: boolean;
  columns: ColumnWidths;
  onColumns: (next: ColumnWidths) => void;
  grouped: boolean;
  /** Character offsets to emphasise, keyed by path. Present while searching. */
  matched?: Map<string, number[]> | undefined;
  sortable?: boolean;
  onRetry: () => void;
}) {
  const metrics = metricsFor(density);
  const rowHeight = view === "grid" ? metrics.tileHeight : metrics.rowHeight;
  const tableWidth = columns.reduce((sum, width) => sum + width, 0);
  const template = columns.map((width) => `${width}px`).join(" ");

  const display = useMemo(
    () => (grouped && view === "details" ? buildGroups(entries) : null),
    // `version` advances when the streamed array is mutated in place.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [grouped, view, entries, version],
  );

  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  useEffect(() => setCollapsed(new Set()), [path]);

  const rows = useMemo(() => {
    if (!display) return null;
    if (collapsed.size === 0) return display;
    const visible: Row[] = [];
    let hiding = false;
    for (const row of display) {
      if (row.kind === "group") {
        hiding = collapsed.has(row.label);
        visible.push(row);
      } else if (!hiding) {
        visible.push(row);
      }
    }
    return visible;
  }, [display, collapsed]);

  const [gridColumns, setGridColumns] = useState(1);
  const rowCount = rows
    ? rows.length
    : view === "grid"
      ? Math.ceil(entries.length / Math.max(1, gridColumns))
      : entries.length;

  const { ref, bounds } = useVirtual(rowCount, rowHeight, view === "grid" ? 1 : 6);
  const scrolling = ref as React.RefObject<HTMLDivElement | null>;

  useEffect(() => {
    if (view !== "grid") {
      setGridColumns(1);
      return;
    }
    const element = scrolling.current;
    if (!element) return;
    const update = () =>
      setGridColumns(
        Math.max(
          1,
          Math.floor((element.clientWidth - 12) / metrics.tileWidth),
        ),
      );
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [view, metrics.tileWidth, scrolling]);

  // A new directory starts at the top.
  useEffect(() => {
    const element = scrolling.current;
    if (element) element.scrollTop = 0;
  }, [path, view, scrolling]);

  const firstIndex = view === "grid" ? bounds.start * gridColumns : bounds.start;
  const lastIndex =
    view === "grid"
      ? Math.min(entries.length, bounds.end * gridColumns)
      : Math.min(entries.length, bounds.end);

  const findIndex = useCallback(
    (target: string) => {
      for (let i = 0; i < entries.length; i += 1) {
        if (entries[i]!.path === target) return i;
      }
      return -1;
    },
    [entries],
  );

  const focusedIndex = useMemo(
    () => (selection.focused === null ? -1 : findIndex(selection.focused)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entries, version, selection.focused, findIndex],
  );
  const anchorIndex = useMemo(
    () => (selection.anchor === null ? -1 : findIndex(selection.anchor)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entries, version, selection.anchor, findIndex],
  );

  const selectSingle = useCallback(
    (entry: Entry) =>
      onSelection({
        selected: new Set([entry.path]),
        focused: entry.path,
        anchor: entry.path,
      }),
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

  const handleRowPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>, index: number) => {
      if (event.button !== 0) return;
      const entry = entries[index]!;
      if (event.shiftKey && anchorIndex >= 0) selectRange(index, anchorIndex);
      else if (event.ctrlKey || event.metaKey) toggle(entry);
      else selectSingle(entry);
    },
    [anchorIndex, entries, selectRange, selectSingle, toggle],
  );

  const focusIndex = useCallback(
    (next: number, extend: boolean) => {
      if (entries.length === 0) return;
      const clamped = Math.min(entries.length - 1, Math.max(0, next));
      const base = extend && anchorIndex >= 0 ? anchorIndex : clamped;
      if (extend) selectRange(clamped, base);
      else selectSingle(entries[clamped]!);
    },
    [anchorIndex, entries, selectRange, selectSingle],
  );

  // Reveal the focused row. Instant, never smooth: this is a keyboard action.
  useEffect(() => {
    const element = scrolling.current;
    if (!element || focusedIndex < 0) return;
    const row =
      view === "grid"
        ? Math.floor(focusedIndex / Math.max(1, gridColumns))
        : focusedIndex;
    const top = row * rowHeight;
    if (top < element.scrollTop) element.scrollTop = top;
    else if (top + rowHeight > element.scrollTop + element.clientHeight) {
      element.scrollTop = top + rowHeight - element.clientHeight;
    }
  }, [focusedIndex, rowHeight, view, gridColumns, scrolling]);

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
      const advance = view === "grid" ? Math.max(1, gridColumns) : 1;
      const page =
        Math.max(
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
          const entry = entries[focusedIndex < 0 ? 0 : focusedIndex];
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
        default:
          break;
      }

      if (event.key.length === 1) {
        const now = performance.now();
        const state = typeahead.current;
        const buffer = now - state.at > 700 ? event.key : state.buffer + event.key;
        typeahead.current = { buffer, at: now };
        const needle = buffer.toLowerCase();
        const start = Math.max(0, focusedIndex);
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
    entries,
    focusIndex,
    focusedIndex,
    gridColumns,
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

  const startResize = (index: number) => (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startWidth = columns[index]!;
    const onMove = (move: PointerEvent) => {
      const next = [...columns] as ColumnWidths;
      next[index] = Math.max(COLUMNS[index]!.min, startWidth + move.clientX - startX);
      onColumns(next);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

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
          style={{ gridTemplateColumns: template }}
        >
          {COLUMNS.map((column, index) => (
            sortable ? (
              <button
                key={column.key}
                type="button"
                className="col"
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
                    size={9}
                    strokeWidth={1.8}
                    className="col-sort"
                  />
                ) : null}
                {index < COLUMNS.length - 1 ? (
                  <span
                    className="col-grip"
                    onPointerDown={startResize(index)}
                    onDoubleClick={() => {
                      const next = [...columns] as ColumnWidths;
                      next[index] = [271, 122, 122, 82][index]!;
                      onColumns(next);
                    }}
                  />
                ) : null}
              </button>
            ) : (
              // Results are ranked by relevance; a sort arrow would contradict
              // the order the user is looking at.
              <div key={column.key} className="col" data-static="true">
                <span className="col-label">{column.label}</span>
              </div>
            )
          ))}
        </div>
      ) : null}

      {showError ? (
        <div className="state">
          <Icon name="folder" size={36} strokeWidth={1} />
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
          <Icon name="folder" size={36} strokeWidth={1} />
          {filtering ? (
            <>
              <div className="state-title">No items match your search</div>
              <div className="state-body">
                Nothing in this folder matches what you typed or the filter you
                chose. Clear them to see everything again.
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
            if (event.target === event.currentTarget) {
              onSelection({ selected: new Set(), focused: null, anchor: null });
            }
          }}
        >
          <div
            className={view === "grid" ? "grid-spacer" : "list-spacer"}
            style={{
              height: rowCount * rowHeight,
              width: view === "grid" ? undefined : tableWidth + 14,
            }}
          >
            {view === "grid"
              ? entries.slice(firstIndex, lastIndex).map((entry, offset) => {
                  const index = firstIndex + offset;
                  const column = index % Math.max(1, gridColumns);
                  const row = Math.floor(index / Math.max(1, gridColumns));
                  return (
                    <div
                      key={entry.path}
                      className="tile"
                      role="option"
                      aria-selected={selection.selected.has(entry.path)}
                      data-selected={
                        selection.selected.has(entry.path) ? "true" : "false"
                      }
                      data-focused={index === focusedIndex ? "true" : "false"}
                      style={{
                        width: metrics.tileWidth,
                        height: metrics.tileHeight,
                        transform: `translate(${column * metrics.tileWidth}px, ${row * metrics.tileHeight}px)`,
                      }}
                      title={entry.name}
                      onPointerDown={(event) => handleRowPointerDown(event, index)}
                      onDoubleClick={() => onOpen(entry)}
                    >
                      <div className="tile-glyph">
                        <FileGlyph
                          kind={entry.kind}
                          size={40}
                          isSymlink={entry.isSymlink}
                        />
                      </div>
                      <div className="tile-label">
                        <MatchedName name={entry.name} indices={matched?.get(entry.path)} />
                      </div>
                    </div>
                  );
                })
              : rows
                ? rows.slice(firstIndex, lastIndex).map((row, offset) => {
                    const position = firstIndex + offset;
                    if (row.kind === "group") {
                      return (
                        <GroupRow
                          key={row.key}
                          label={row.label}
                          count={row.count}
                          top={position * rowHeight}
                          collapsed={collapsed.has(row.label)}
                          onToggle={() =>
                            setCollapsed((prev) => {
                              const next = new Set(prev);
                              if (next.has(row.label)) next.delete(row.label);
                              else next.add(row.label);
                              return next;
                            })
                          }
                        />
                      );
                    }
                    return (
                      <DetailRow
                        key={row.entry.path}
                        entry={row.entry}
                        top={position * rowHeight}
                        template={template}
                        width={tableWidth}
                        rowHeight={rowHeight}
                        selected={selection.selected.has(row.entry.path)}
                        focused={row.index === focusedIndex}
                        indices={matched?.get(row.entry.path)}
                        onPointerDown={(event) =>
                          handleRowPointerDown(event, row.index)
                        }
                        onDoubleClick={() => onOpen(row.entry)}
                      />
                    );
                  })
                : entries.slice(firstIndex, lastIndex).map((entry, offset) => {
                    const index = firstIndex + offset;
                    return (
                      <DetailRow
                        key={entry.path}
                        entry={entry}
                        top={index * rowHeight}
                        template={template}
                        width={tableWidth}
                        rowHeight={rowHeight}
                        selected={selection.selected.has(entry.path)}
                        focused={index === focusedIndex}
                        indices={matched?.get(entry.path)}
                        onPointerDown={(event) =>
                          handleRowPointerDown(event, index)
                        }
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
  top,
  template,
  width,
  rowHeight,
  selected,
  focused,
  indices,
  onPointerDown,
  onDoubleClick,
}: {
  entry: Entry;
  top: number;
  template: string;
  width: number;
  rowHeight: number;
  selected: boolean;
  focused: boolean;
  indices?: number[] | undefined;
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
        gridTemplateColumns: template,
        width,
        height: rowHeight,
        transform: `translateY(${top}px)`,
      }}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
    >
      <div className="cell cell-name">
        <FileGlyph kind={entry.kind} size={16} isSymlink={entry.isSymlink} />
        <MatchedName name={entry.name} indices={indices} />
      </div>
      <div className="cell cell-muted tabular">{formatDate(entry.modifiedMs)}</div>
      <div className="cell cell-muted">
        <span className="cell-text">{typeLabel(entry)}</span>
      </div>
      <div className="cell cell-muted tabular">
        {formatSize(entry.size, entry.isDir)}
      </div>
    </div>
  );
}

function GroupRow({
  label,
  count,
  top,
  collapsed,
  onToggle,
}: {
  label: string;
  count: number;
  top: number;
  collapsed: boolean;
  onToggle: () => void;
}) {
  return (
    <div
      className="group"
      data-collapsed={collapsed ? "true" : "false"}
      style={{ transform: `translateY(${top}px)` }}
      onClick={onToggle}
      role="presentation"
    >
      <Icon
        name="chevronDown"
        size={12}
        strokeWidth={1.6}
        className="group-chevron"
      />
      <span className="group-label">{label}</span>
      {collapsed ? <span className="group-count">{count}</span> : null}
    </div>
  );
}

/**
 * A file name with the searched characters emphasised, the way Explorer marks
 * what matched. `indices` are character offsets, so the string is walked as
 * code points rather than UTF-16 units, and runs of hits collapse into one
 * element instead of one per character.
 */
function MatchedName({
  name,
  indices,
}: {
  name: string;
  indices?: number[] | undefined;
}) {
  if (!indices || indices.length === 0) {
    return (
      <span className="cell-text" title={name}>
        {name}
      </span>
    );
  }

  const chars = Array.from(name);
  const marked = new Set(indices);
  const parts: { text: string; hit: boolean }[] = [];
  for (let i = 0; i < chars.length; i += 1) {
    const hit = marked.has(i);
    const last = parts[parts.length - 1];
    if (last && last.hit === hit) last.text += chars[i];
    else parts.push({ text: chars[i]!, hit });
  }

  return (
    <span className="cell-text" title={name}>
      {parts.map((part, index) =>
        part.hit ? (
          <span key={index} className="hit">
            {part.text}
          </span>
        ) : (
          part.text
        ),
      )}
    </span>
  );
}
