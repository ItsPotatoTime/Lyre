import { useEffect, useRef, useState, type RefObject } from "react";
import type { Entry, Location, PathInfo } from "@/types";
import { locationLabel } from "@/lib/location";
import { Icon } from "./Icons";
import { Menu } from "./Menu";
import { PlaceGlyph } from "./FileGlyph";

/**
 * Navigation, the address field, and search. The reference keeps the magnifier
 * on the right edge of the search field and puts a small location glyph at the
 * start of the breadcrumb.
 */
export function AddressBar({
  location,
  glyph,
  info,
  entries,
  canBack,
  canForward,
  canUp,
  onBack,
  onForward,
  onUp,
  onRefresh,
  onNavigate,
  query,
  onQuery,
  editSignal,
  searchRef,
}: {
  location: Location;
  glyph: string;
  info: PathInfo | null;
  entries: Entry[];
  canBack: boolean;
  canForward: boolean;
  canUp: boolean;
  onBack: () => void;
  onForward: () => void;
  onUp: () => void;
  onRefresh: () => void;
  onNavigate: (location: Location) => void;
  query: string;
  onQuery: (value: string) => void;
  /** Increment to open the editable address field (Ctrl+L). */
  editSignal: number;
  searchRef: RefObject<HTMLInputElement | null>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [editing]);

  useEffect(() => {
    if (editSignal === 0) return;
    setDraft(info?.path ?? location.path);
    setEditing(true);
    // Only the signal opens the field, not a later path change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editSignal]);

  const label = locationLabel(location);
  const isVirtual = location.kind === "home" || location.kind === "thisPc";
  const segments = isVirtual
    ? [{ name: label, path: location.path }]
    : (info?.segments ?? []);
  // Deep paths keep their tail and fold the middle, as Explorer does.
  const folded =
    segments.length > 5 ? [segments[0]!, null, ...segments.slice(-3)] : segments;

  const commit = () => {
    const next = draft.trim();
    setEditing(false);
    if (!next || next === location.path) return;
    onNavigate({ kind: "folder", path: next });
  };

  const children = entries.filter((entry) => entry.isDir).slice(0, 30);

  return (
    <div className="toolbar">
      <button
        type="button"
        className="nav-btn"
        data-disabled={canBack ? "false" : "true"}
        aria-label="Back"
        title="Back (Alt+Left)"
        onClick={onBack}
      >
        <Icon name="arrowLeft" size={16} />
      </button>
      <button
        type="button"
        className="nav-btn"
        data-disabled={canForward ? "false" : "true"}
        aria-label="Forward"
        title="Forward (Alt+Right)"
        onClick={onForward}
      >
        <Icon name="arrowRight" size={16} />
      </button>
      <button
        type="button"
        className="nav-btn"
        data-disabled={canUp ? "false" : "true"}
        aria-label="Up"
        title="Up one level (Backspace)"
        onClick={onUp}
      >
        <Icon name="arrowUp" size={16} />
      </button>
      <button
        type="button"
        className="nav-btn"
        aria-label="Refresh"
        title="Refresh (F5)"
        onClick={onRefresh}
      >
        <Icon name="refresh" size={16} />
      </button>

      <div
        className="address-field"
        data-editing={editing ? "true" : "false"}
        onPointerDown={(event) => {
          if (editing) return;
          if (event.target === event.currentTarget) {
            setDraft(location.path || label);
            setEditing(true);
          }
        }}
      >
        {editing ? (
          <input
            ref={inputRef}
            className="address-input"
            value={draft}
            spellCheck={false}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                commit();
              } else if (event.key === "Escape") {
                event.preventDefault();
                setEditing(false);
              }
            }}
            onBlur={() => setEditing(false)}
          />
        ) : (
          <>
            <span className="address-glyph">
              <PlaceGlyph icon={glyph} size={16} />
            </span>
            <div className="crumbs">
              {folded.map((segment, index) =>
                segment === null ? (
                  <span key={`fold-${index}`} className="crumb-wrap">
                    <button
                      type="button"
                      className="crumb"
                      aria-label="Edit address"
                      onClick={() => {
                        setDraft(location.path);
                        setEditing(true);
                      }}
                    >
                      …
                    </button>
                    <Icon
                      name="chevronRight"
                      size={10}
                      className="crumb-sep"
                      strokeWidth={1.6}
                    />
                  </span>
                ) : (
                  <span key={`${segment.path}-${index}`} className="crumb-wrap">
                    <button
                      type="button"
                      className="crumb"
                      title={segment.path}
                      data-drop="folder"
                      data-path={segment.path}
                      onClick={() =>
                        onNavigate({ kind: "folder", path: segment.path })
                      }
                    >
                      <span className="crumb-label">{segment.name}</span>
                    </button>
                    {index < folded.length - 1 ? (
                      <Icon
                        name="chevronRight"
                        size={10}
                        className="crumb-sep"
                        strokeWidth={1.6}
                      />
                    ) : null}
                  </span>
                ),
              )}
            </div>
            {children.length > 0 ? (
              <Menu
                align="end"
                entries={children.map((child) => ({
                  kind: "item" as const,
                  id: child.path,
                  label: child.name,
                  icon: "folder" as const,
                  onSelect: () =>
                    onNavigate({ kind: "folder", path: child.path }),
                }))}
                trigger={({ toggle }) => (
                  <button
                    type="button"
                    className="nav-btn"
                    aria-label="Open a subfolder"
                    title="Open a subfolder"
                    style={{ width: 22, height: 22 }}
                    onClick={toggle}
                  >
                    <Icon name="chevronDown" size={10} strokeWidth={1.6} />
                  </button>
                )}
              />
            ) : null}
          </>
        )}
      </div>

      <div className="search">
        <input
          ref={searchRef}
          type="text"
          placeholder={`Search ${label}`}
          aria-label={`Search ${label}`}
          value={query}
          spellCheck={false}
          onChange={(event) => onQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.stopPropagation();
              onQuery("");
              searchRef.current?.blur();
            }
          }}
        />
        {query ? (
          <button
            type="button"
            aria-label="Clear search"
            onClick={() => {
              onQuery("");
              searchRef.current?.focus();
            }}
          >
            <Icon name="close" size={9} strokeWidth={1.8} />
          </button>
        ) : (
          <Icon name="search" size={14} />
        )}
      </div>
    </div>
  );
}
