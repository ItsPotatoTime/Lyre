import { useEffect, useRef, useState, type RefObject } from "react";
import type { PathInfo } from "@/types";
import { Icon } from "./Icons";

export function AddressBar({
  info,
  canBack,
  canForward,
  canUp,
  onBack,
  onForward,
  onUp,
  onNavigate,
  onRefresh,
  query,
  onQuery,
  editSignal,
  searchRef: externalSearchRef,
}: {
  info: PathInfo | null;
  canBack: boolean;
  canForward: boolean;
  canUp: boolean;
  onBack: () => void;
  onForward: () => void;
  onUp: () => void;
  onNavigate: (path: string) => void;
  onRefresh: () => void;
  query: string;
  onQuery: (value: string) => void;
  /** Increment to open the editable address field (Ctrl+L). */
  editSignal: number;
  searchRef: RefObject<HTMLInputElement | null>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const searchRef = externalSearchRef;

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  useEffect(() => {
    if (editSignal === 0) return;
    setDraft(info?.path ?? "");
    setEditing(true);
    // Only the signal should open the field, not a later path change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editSignal]);

  const commit = () => {
    const next = draft.trim();
    setEditing(false);
    if (next && next !== info?.path) onNavigate(next);
  };

  const segments = info?.segments ?? [];
  // Explorer keeps the tail of a deep path visible and folds the middle into
  // an ellipsis that opens the editable form.
  const folded =
    segments.length > 5
      ? [
          segments[0]!,
          null,
          ...segments.slice(segments.length - 3),
        ]
      : segments.map((segment) => segment);

  return (
    <div className="addressbar">
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
          // Clicking the field itself (not a crumb) turns it into a path input.
          if (event.target === event.currentTarget) {
            setDraft(info?.path ?? "");
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
          <div className="crumbs">
            {folded.map((segment, index) =>
              segment === null ? (
                <button
                  key={`fold-${index}`}
                  type="button"
                  className="crumb"
                  aria-label="Edit address"
                  onClick={() => {
                    setDraft(info?.path ?? "");
                    setEditing(true);
                  }}
                >
                  …
                </button>
              ) : (
                <span key={segment.path} className="crumb-wrap">
                  <button
                    type="button"
                    className="crumb"
                    title={segment.path}
                    onClick={() => onNavigate(segment.path)}
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
            {segments.length === 0 ? (
              <button
                type="button"
                className="crumb-plain"
                onPointerDown={() => {
                  setDraft(info?.path ?? "");
                  setEditing(true);
                }}
              >
                {info?.path ?? "This PC"}
              </button>
            ) : null}
          </div>
        )}
      </div>

      <div className="search">
        <Icon name="search" size={14} />
        <input
          ref={searchRef}
          type="text"
          placeholder="Search"
          aria-label="Search this folder"
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
            className="search-clear"
            aria-label="Clear search"
            onClick={() => {
              onQuery("");
              searchRef.current?.focus();
            }}
          >
            <Icon name="close" size={10} strokeWidth={1.8} />
          </button>
        ) : null}
      </div>
    </div>
  );
}
