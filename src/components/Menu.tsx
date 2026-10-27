import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "./Icons";

export type MenuEntry =
  | {
      kind: "item";
      id: string;
      label: string;
      icon?: IconName;
      checked?: boolean;
      hint?: string;
      disabled?: boolean;
      onSelect: () => void;
    }
  | { kind: "sep" }
  | { kind: "label"; text: string };

/**
 * A Flyout. Opens on click, closes on outside click or Escape, and never takes
 * focus away from the file list until the user picks something.
 */
export function Menu({
  trigger,
  entries,
  align = "start",
}: {
  trigger: (state: { open: boolean; toggle: () => void }) => ReactNode;
  entries: MenuEntry[];
  align?: "start" | "end";
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        event.preventDefault();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);

  return (
    <div className="menu-anchor" ref={ref}>
      {trigger({ open, toggle: () => setOpen((v) => !v) })}
      {open ? (
        <div className="menu" data-align={align} role="menu">
          {entries.map((entry, index) => {
            if (entry.kind === "sep") {
              return <div key={`sep-${index}`} className="menu-sep" />;
            }
            if (entry.kind === "label") {
              return (
                <div key={`label-${index}`} className="menu-label">
                  {entry.text}
                </div>
              );
            }
            return (
              <button
                key={entry.id}
                type="button"
                role="menuitem"
                className="menu-item"
                data-checked={entry.checked ? "true" : "false"}
                disabled={entry.disabled}
                style={entry.disabled ? { color: "var(--text-disabled)" } : undefined}
                onClick={() => {
                  setOpen(false);
                  entry.onSelect();
                }}
              >
                <span className="menu-item-check">
                  {entry.checked ? <Icon name="check" size={14} /> : null}
                </span>
                <span className="menu-item-label">{entry.label}</span>
                {entry.hint ? (
                  <span className="menu-item-hint">{entry.hint}</span>
                ) : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
