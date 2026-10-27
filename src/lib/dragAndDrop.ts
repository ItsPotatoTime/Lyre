import { useCallback, useEffect, useRef } from "react";

/**
 * Dragging, driven by pointer events rather than HTML5 drag and drop.
 *
 * That is not a stylistic choice. Tauri's native drag handler is the only way
 * to receive real filesystem paths for a drop coming from the OS, and it
 * *replaces* WebView2's HTML5 drag/drop handler on Windows, which would leave
 * internal dragging working on Linux and macOS and dead on Windows. Pointer
 * events are ours on every platform, and they let us draw the move/copy badge
 * and the drop highlight exactly.
 *
 * What is not covered, because it needs an operating-system drag source that
 * Tauri does not provide: dragging an item *out* of Lyre, into Explorer, the
 * desktop, or another Lyre window.
 */

export type DragTarget =
  | { kind: "folder"; path: string }
  | { kind: "tab"; id: string; path: string }
  | { kind: "background"; path: string };

export type DropMode = "move" | "copy";

/** Movement before a press becomes a drag. Below this a press is a click. */
const THRESHOLD = 4;

interface DragSession {
  paths: string[];
  startX: number;
  startY: number;
  /** Last pointer position, so Shift can update the badge while stationary. */
  x: number;
  y: number;
  /**
   * Whether Shift is held, tracked from the key events themselves.
   *
   * WebKit reports `shiftKey: true` on the keyup of the Shift key, so reading
   * that field there says "still held" after the person let go. Chromium
   * reports false, which is why this only shows up on the real webview.
   */
  shiftHeld: boolean;
  active: boolean;
  target: DragTarget | null;
  hovered: Element | null;
  mode: DropMode;
}

const EMPTY: DragTarget | null = null;

/**
 * Decide what a point is over, from the `data-drop` attributes the interface
 * puts on its drop zones. One mechanism covers rows, breadcrumbs, tabs, the
 * navigation rail and the empty area behind them.
 */
export function targetAt(x: number, y: number): DragTarget | null {
  const element = document.elementFromPoint(x, y);
  const zone = element?.closest("[data-drop]") as HTMLElement | null;
  if (!zone) return null;

  const kind = zone.dataset.drop;
  const path = zone.dataset.path ?? "";
  if (kind === "folder" && path) return { kind: "folder", path };
  if (kind === "tab" && path) {
    return { kind: "tab", id: zone.dataset.tab ?? "", path };
  }
  if (kind === "background" && path) return { kind: "background", path };
  return null;
}

export function useDrag(
  onDrop: (target: DragTarget, mode: DropMode, paths: string[]) => void,
  /**
   * Decides move or copy for a drag. The caller owns this because it depends on
   * which volumes the paths are on, and holding Shift inverts whatever the
   * default is rather than always meaning one thing.
   */
  modeFor: (paths: string[], target: DragTarget | null, shift: boolean) => DropMode = (
    _paths,
    _target,
    shift,
  ) => (shift ? "copy" : "move"),
) {
  const session = useRef<DragSession | null>(null);
  const badge = useRef<HTMLDivElement>(null);
  const callback = useRef(onDrop);
  callback.current = onDrop;
  const decide = useRef(modeFor);
  decide.current = modeFor;

  const paintBadge = useCallback((x: number, y: number, mode: DropMode, count: number) => {
    const element = badge.current;
    if (!element) return;
    element.hidden = false;
    element.dataset.mode = mode;
    element.style.transform = `translate(${x + 14}px, ${y + 16}px)`;
    const label = element.querySelector("[data-role='label']");
    if (label) {
      label.textContent = `${mode === "move" ? "Move" : "Copy"} ${count} item${count === 1 ? "" : "s"}`;
    }
  }, []);

  const hideBadge = useCallback(() => {
    if (badge.current) badge.current.hidden = true;
  }, []);

  const clearHighlight = useCallback(() => {
    const current = session.current;
    if (current?.hovered) current.hovered.removeAttribute("data-drop-hover");
    if (current) current.hovered = null;
  }, []);

  /** Begin a drag from a press that has already selected its items. */
  const begin = useCallback(
    (paths: string[], event: React.PointerEvent) => {
      if (paths.length === 0) return;
      session.current = {
        paths,
        startX: event.clientX,
        startY: event.clientY,
        x: event.clientX,
        y: event.clientY,
        shiftHeld: event.shiftKey,
        active: false,
        target: EMPTY,
        hovered: null,
        mode: decide.current(paths, null, event.shiftKey),
      };
    },
    [],
  );

  const cancel = useCallback(() => {
    clearHighlight();
    hideBadge();
    // The body flag drives the cursor for the whole window; leaving it set
    // would keep the drag cursor after a cancelled drag.
    delete document.body.dataset.dragging;
    session.current = null;
  }, [clearHighlight, hideBadge]);

  useEffect(() => {
    const onMove = (event: PointerEvent) => {
      const current = session.current;
      if (!current) return;

      // Holding Shift switches a drag from moving to copying, and the badge
      // says which one will happen before the button is released. Pointer
      // events do report modifiers correctly, so they are the reliable source.
      current.shiftHeld = event.shiftKey;
      current.x = event.clientX;
      current.y = event.clientY;

      if (!current.active) {
        const moved =
          Math.abs(event.clientX - current.startX) + Math.abs(event.clientY - current.startY);
        if (moved < THRESHOLD) return;
        current.active = true;
      }

      const target = targetAt(event.clientX, event.clientY);
      // A folder cannot be dropped into itself or into its own subtree.
      const usable =
        target && current.paths.some((path) => target.path.startsWith(path))
          ? null
          : target;

      current.target = usable;
      // What this drag does depends on where it would land, so the mode is
      // worked out against the zone under the pointer rather than up front.
      current.mode = decide.current(current.paths, usable, current.shiftHeld);

      const zone = usable ? document.elementFromPoint(event.clientX, event.clientY) : null;
      const zoneElement = (zone?.closest("[data-drop]") as Element | null) ?? null;
      if (zoneElement !== current.hovered) {
        current.hovered?.removeAttribute("data-drop-hover");
        zoneElement?.setAttribute("data-drop-hover", "");
        current.hovered = zoneElement;
      }

      paintBadge(event.clientX, event.clientY, current.mode, current.paths.length);
      document.body.dataset.dragging = current.mode;
    };

    const onUp = (event: PointerEvent) => {
      const current = session.current;
      if (!current) return;
      // Clear the highlight while the session still knows which element it
      // marked; clearing after the reset would leave a ring on screen.
      clearHighlight();
      session.current = null;
      delete document.body.dataset.dragging;
      hideBadge();
      if (!current.active) return;

      // Re-run the hit test at the release point: the last pointermove may have
      // been a few pixels away from where the button actually came up.
      const target = targetAt(event.clientX, event.clientY);
      if (!target) return;
      if (current.paths.some((path) => target.path.startsWith(path))) return;
      callback.current(target, current.mode, current.paths);
    };

    const onKey = (event: KeyboardEvent) => {
      const current = session.current;
      if (!current) return;

      if (event.key === "Escape") {
        event.stopPropagation();
        event.preventDefault();
        cancel();
        return;
      }

      // Pausing over a folder and then pressing Shift is a normal way to
      // decide; nothing moves, so this has to update the badge itself instead
      // of waiting for the next pointermove.
      if (event.key === "Shift") {
        // The event's own type decides which way the key went. Its `shiftKey`
        // field cannot be trusted for the Shift keyup on WebKit.
        current.shiftHeld = event.type === "keydown";
        current.mode = decide.current(current.paths, current.target, current.shiftHeld);
        if (current.active) {
          paintBadge(current.x, current.y, current.mode, current.paths.length);
          document.body.dataset.dragging = current.mode;
        }
      }
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", cancel);
    const onBlur = () => {
      const current = session.current;
      if (!current || !current.active) return;
      current.shiftHeld = false;
      current.mode = decide.current(current.paths, current.target, false);
      paintBadge(current.x, current.y, current.mode, current.paths.length);
      document.body.dataset.dragging = current.mode;
    };

    window.addEventListener("keydown", onKey, true);
    // Shift going up has to be seen too, or the badge keeps saying Copy after
    // the key is released.
    window.addEventListener("keyup", onKey, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
      window.removeEventListener("blur", onBlur);
    };
  }, [cancel, clearHighlight, hideBadge, paintBadge]);

  return { begin, cancel, badge };
}
