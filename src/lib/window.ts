/**
 * Window chrome plumbing. The window is undecorated (Explorer's tab strip
 * lives in the title bar, which a native frame cannot host), so the page owns
 * dragging, the caption buttons, and the resize edges.
 *
 * Every function is a no-op outside Tauri, which is what lets the same
 * interface run in a browser for design work.
 */

import { isTauri } from "./ipc";

export type ResizeEdge =
  | "North"
  | "South"
  | "East"
  | "West"
  | "NorthEast"
  | "NorthWest"
  | "SouthEast"
  | "SouthWest";

async function current() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export const windowChrome = {
  minimize() {
    if (!isTauri) return;
    void current().then((w) => w.minimize());
  },

  async toggleMaximize() {
    if (!isTauri) return false;
    const w = await current();
    await w.toggleMaximize();
    return w.isMaximized();
  },

  close() {
    if (!isTauri) return;
    void current().then((w) => w.close());
  },

  async isMaximized(): Promise<boolean> {
    if (!isTauri) return false;
    const w = await current();
    return w.isMaximized();
  },

  onMaximizeChange(handler: (maximized: boolean) => void): () => void {
    if (!isTauri) return () => {};
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void current().then(async (w) => {
      const stop = await w.onResized(async () => {
        const maximized = await w.isMaximized();
        if (!disposed) handler(maximized);
      });
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  },

  async toggleFullscreen() {
    if (!isTauri) return;
    const w = await current();
    const fullscreen = await w.isFullscreen();
    await w.setFullscreen(!fullscreen);
  },

  startResize(edge: ResizeEdge) {
    if (!isTauri) return;
    void current().then((w) => w.startResizeDragging(edge));
  },
};
