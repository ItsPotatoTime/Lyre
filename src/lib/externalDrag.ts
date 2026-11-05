import { isTauri, stagedDropPaths } from "./ipc";
import { isNativeDragActive } from "./nativeDrag";

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
 * The paths are exchanged for their staged counterparts before the drop is
 * handed on. An archiver can delete the files it named between the native drop
 * and this callback, so Rust takes the snapshot earlier and this is only the
 * handover.
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
  /**
   * Files were released. The paths arrive as a promise because collecting
   * Rust's snapshot takes an IPC round trip; resolve the drop target where the
   * pointer was released before awaiting it, since the listing may repaint.
   */
  onDrop: (x: number, y: number, paths: Promise<string[]>) => void;
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
      // While Lyre is the drag *source*, OLE can deliver the release to this
      // same window if the pointer never made it out. That is not an import.
      if (isNativeDragActive()) return;
      // An archiver's extraction folder may already be gone by now. Rust took
      // a snapshot inside IDropTarget::Drop; handing the promise over lets the
      // caller resolve the target synchronously while the snapshot is
      // collected, so the paste never reads a path the source has deleted.
      handlers.onDrop(x, y, stagedDropPaths(payload.paths));
    } else {
      handlers.onOver(x, y);
    }
  });
}
