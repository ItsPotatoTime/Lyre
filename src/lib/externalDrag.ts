import { isTauri } from "./ipc";

/**
 * Drops coming from outside the window: Explorer, Finder, a file manager on the
 * desktop, or another Lyre window.
 *
 * Tauri intercepts these at the webview level and hands over real filesystem
 * paths, which is the only way this works: a browser drop event carries a File
 * object with no path on it. The cost is that Tauri's handler replaces WebView2's
 * HTML5 drag/drop on Windows, which is why dragging *inside* the window is
 * driven by pointer events instead of HTML5.
 *
 * Positions arrive in physical pixels; they are divided by the device pixel
 * ratio to land where the pointer actually is in CSS space.
 */

let hovered: Element | null = null;

/** Mark the drop zone under a point, clearing whichever was marked before. */
export function setDropHighlight(x: number, y: number): void {
  const zone = (document.elementFromPoint(x, y)?.closest("[data-drop]") ??
    null) as Element | null;
  if (zone === hovered) return;
  hovered?.removeAttribute("data-drop-hover");
  zone?.setAttribute("data-drop-hover", "");
  hovered = zone;
}

export function clearDropHighlight(): void {
  hovered?.removeAttribute("data-drop-hover");
  hovered = null;
}

export interface ExternalDropHandlers {
  /** A drag entered or moved; `x`/`y` are CSS pixels. */
  onOver: (x: number, y: number) => void;
  /** Files were released. */
  onDrop: (x: number, y: number, paths: string[]) => void;
  onLeave: () => void;
}

/**
 * Subscribe to native drops. Returns a disposer; outside the desktop app there
 * is nothing to subscribe to, because the browser preview has no filesystem.
 */
export async function watchExternalDrops(
  handlers: ExternalDropHandlers,
): Promise<() => void> {
  if (!isTauri) return () => {};

  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  const ratio = window.devicePixelRatio || 1;

  return getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    if (payload.type === "leave") {
      clearDropHighlight();
      handlers.onLeave();
      return;
    }
    const x = payload.position.x / ratio;
    const y = payload.position.y / ratio;
    if (payload.type === "drop") {
      clearDropHighlight();
      handlers.onDrop(x, y, payload.paths);
    } else {
      handlers.onOver(x, y);
    }
  });
}
