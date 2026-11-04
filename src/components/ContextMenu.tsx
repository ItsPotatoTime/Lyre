import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { createPortal } from "react-dom";
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
      /**
       * The items of a submenu. A parent is not invoked, it opens: this is how
       * a program that adds a whole group to the system's menu appears.
       */
      children?: ContextItem[];
      /**
       * Whether the item shows before anyone has chosen anything. Only the
       * items the system's own menu contributes ever start off.
       */
      defaultOn?: boolean;
      onSelect?: () => void;
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
 *
 * A menu listing what other programs registered can be taller than the window,
 * so it scrolls rather than running off the bottom. Its submenus cannot live
 * inside a scrolling box, because they would be clipped by it, which is why they
 * are drawn into the document instead.
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
    const margin = 8;
    // The menu grows after it opens when the system's own items arrive, so this
    // is re-clamped on every resize rather than only once at the pointer.
    const clamp = () => {
      const rect = element.getBoundingClientRect();
      const left = Math.max(margin, Math.min(x, window.innerWidth - rect.width - margin));
      const top = Math.max(margin, Math.min(y, window.innerHeight - rect.height - margin));
      setPos((prev) => (prev.left === left && prev.top === top ? prev : { left, top }));
    };
    clamp();
    const observer = new ResizeObserver(clamp);
    observer.observe(element);
    return () => observer.disconnect();
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
    // Scrolling the menu itself is how a long one is read; scrolling anything
    // else moves the thing it was opened over, so the menu goes.
    const onScroll = (event: Event) => {
      if (ref.current?.contains(event.target as Node)) return;
      onClose();
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  return (
    <div
      className="menu menu-context"
      role="menu"
      ref={ref}
      style={{ left: pos.left, top: pos.top }}
    >
      <MenuRows items={items} onClose={onClose} />
    </div>
  );
}

/** The invokable shape, which is the only one that can own a submenu. */
type ParentItem = Extract<ContextItem, { kind: "item" }>;

/**
 * One level of a menu.
 *
 * Each level owns which of its rows is expanded, so a submenu closes when the
 * pointer moves to a sibling without anything having to be told to close it. The
 * close is delayed because the panel is drawn outside this element, and crossing
 * into it counts as leaving.
 */
function MenuRows({ items, onClose }: { items: ContextItem[]; onClose: () => void }) {
  const [open, setOpen] = useState<{ id: string; anchor: DOMRect } | null>(null);
  const timer = useRef<number | null>(null);

  const stay = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  };

  const leave = () => {
    stay();
    timer.current = window.setTimeout(() => setOpen(null), 140);
  };

  useEffect(() => () => stay(), []);

  const expand = (item: ParentItem, element: HTMLElement) => {
    stay();
    setOpen((current) =>
      current?.id === item.id
        ? current
        : { id: item.id, anchor: element.getBoundingClientRect() },
    );
  };

  return (
    <div className="menu-rows" onPointerLeave={leave}>
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

        const showing = open?.id === item.id;
        const icon = item.checked ? (
          <Icon name="check" size={14} />
        ) : item.icon ? (
          <Icon name={item.icon} size={14} />
        ) : null;

        if (item.children) {
          return (
            <div
              key={item.id}
              className="menu-sub"
              onPointerEnter={(event) => expand(item, event.currentTarget)}
            >
              <button
                type="button"
                role="menuitem"
                aria-haspopup="menu"
                aria-expanded={showing}
                className="menu-item"
                data-open={showing ? "true" : "false"}
                onClick={(event) => expand(item, event.currentTarget)}
              >
                <span className="menu-item-icon">{icon}</span>
                <span className="menu-item-label">{item.label}</span>
                <span className="menu-item-arrow">
                  <Icon name="chevronRight" size={10} strokeWidth={1.6} />
                </span>
              </button>
              {showing && open ? (
                <SubMenu
                  anchor={open.anchor}
                  items={item.children}
                  onClose={onClose}
                  onStay={stay}
                  onLeave={leave}
                />
              ) : null}
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
              item.onSelect?.();
            }}
          >
            <span className="menu-item-icon">{icon}</span>
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

/**
 * The panel a submenu opens into.
 *
 * It is drawn into the document rather than beside its row, because the menu it
 * belongs to scrolls when it is long and would clip it. It is placed from the
 * row's rectangle: to the right of the row, or to the left when the right edge
 * is closer than it is wide, and lifted when it would leave the bottom.
 */
function SubMenu({
  anchor,
  items,
  onClose,
  onStay,
  onLeave,
}: {
  anchor: DOMRect;
  items: ContextItem[];
  onClose: () => void;
  onStay: () => void;
  onLeave: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const margin = 8;
    const width = element.offsetWidth;
    const height = element.offsetHeight;
    const flip = anchor.right + width - 2 > window.innerWidth - margin;
    const left = flip
      ? Math.max(margin, anchor.left - width + 2)
      : Math.min(anchor.right - 2, window.innerWidth - width - margin);
    const top = Math.max(margin, Math.min(anchor.top - 5, window.innerHeight - height - margin));
    setPlace({ left, top });
  }, [anchor]);

  return createPortal(
    <div
      className="menu menu-submenu"
      role="menu"
      ref={ref}
      onPointerEnter={onStay}
      onPointerLeave={onLeave}
      style={
        {
          left: place?.left ?? 0,
          top: place?.top ?? 0,
          visibility: place ? "visible" : "hidden",
        } as CSSProperties
      }
    >
      <MenuRows items={items} onClose={onClose} />
    </div>,
    document.body,
  );
}
