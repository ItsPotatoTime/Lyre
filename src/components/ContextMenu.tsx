import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon, type IconName } from "./Icons";

export type ContextItem =
  | {
      kind: "item";
      id: string;
      label: string;
      icon?: IconName;
      shortcut?: string;
      disabled?: boolean;
      /** Shown as a dimmed reason when the item cannot be used. */
      note?: string;
      checked?: boolean;
      onSelect: () => void;
    }
  | { kind: "sep" }
  | { kind: "label"; text: string };

/**
 * A menu at the pointer.
 *
 * Positioned from the click and then clamped to the window, so a right click
 * near an edge opens inward rather than off-screen. It closes on the next click
 * anywhere, on Escape, and on scroll, because a menu detached from its target
 * is worse than no menu.
 */
export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: ContextItem[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const margin = 8;
    const left = Math.min(x, window.innerWidth - rect.width - margin);
    const top = Math.min(y, window.innerHeight - rect.height - margin);
    setPos({
      left: Math.max(margin, left),
      top: Math.max(margin, top),
    });
  }, [x, y]);

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        event.preventDefault();
        onClose();
      }
    };
    const onScroll = () => onClose();

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [onClose]);

  return (
    <div
      className="menu menu-context"
      role="menu"
      ref={ref}
      style={{ left: pos.left, top: pos.top }}
    >
      {items.map((item, index) => {
        if (item.kind === "sep") {
          return <div key={`sep-${index}`} className="menu-sep" />;
        }
        if (item.kind === "label") {
          return (
            <div key={`label-${index}`} className="menu-label">
              {item.text}
            </div>
          );
        }
        return (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            className="menu-item"
            data-checked={item.checked ? "true" : "false"}
            data-disabled={item.disabled ? "true" : "false"}
            disabled={item.disabled}
            title={item.disabled ? item.note : undefined}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            <span className="menu-item-icon">
              {item.checked ? (
                <Icon name="check" size={14} />
              ) : item.icon ? (
                <Icon name={item.icon} size={14} />
              ) : null}
            </span>
            <span className="menu-item-label">{item.label}</span>
            {item.shortcut ? (
              <span className="menu-item-hint">{item.shortcut}</span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
