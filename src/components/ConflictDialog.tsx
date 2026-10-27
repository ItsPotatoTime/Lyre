import { useEffect, useRef } from "react";
import { Icon } from "./Icons";

/**
 * Asked when a paste would land on an existing name.
 *
 * Both answers are safe, which is the point of asking: keeping both numbers the
 * incoming item, and replacing merges a folder rather than discarding what the
 * destination already had. Nothing is written until one of them is chosen.
 */
export function ConflictDialog({
  destinationName,
  names,
  onKeepBoth,
  onOverwrite,
  onCancel,
}: {
  /** The folder being pasted into, for the sentence. */
  destinationName: string;
  /** The names that already exist there. */
  names: string[];
  onKeepBoth: () => void;
  onOverwrite: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    ref.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        event.preventDefault();
        onCancel();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [onCancel]);

  const one = names.length === 1;
  const shown = names.slice(0, 4);

  return (
    <div
      className="dialog-scrim"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <div
        className="dialog dialog-conflict"
        role="dialog"
        aria-modal="true"
        aria-label="Replace or keep both"
        tabIndex={-1}
        ref={ref}
      >
        <header className="dialog-head">
          <span className="dialog-glyph">
            <Icon name="folder" size={24} strokeWidth={1.4} />
          </span>
          <span className="dialog-title">
            {one ? "This item already exists" : "These items already exist"}
          </span>
        </header>

        <p className="dialog-note">
          {destinationName} already has {one ? "an item" : `${names.length} items`} with
          {one ? " this name" : " these names"}:
        </p>

        <ul className="conflict-list">
          {shown.map((name) => (
            <li key={name} title={name}>
              {name}
            </li>
          ))}
          {names.length > shown.length ? (
            <li className="conflict-more">
              and {names.length - shown.length} more
            </li>
          ) : null}
        </ul>

        <p className="dialog-hint">
          Replacing writes over the files that are already there and keeps
          everything else in the folder. It never deletes the folder first.
        </p>

        <footer className="dialog-foot">
          <button type="button" className="dialog-action" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="dialog-action" onClick={onKeepBoth}>
            Keep both
          </button>
          <button
            type="button"
            className="dialog-action dialog-action-primary"
            onClick={onOverwrite}
          >
            Replace
          </button>
        </footer>
      </div>
    </div>
  );
}
