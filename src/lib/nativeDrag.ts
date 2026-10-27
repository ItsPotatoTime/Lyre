import { isTauri } from "./ipc";

/**
 * Handing a drag to the operating system.
 *
 * Internal dragging is pointer-driven, which cannot by itself start a drag that
 * other applications can receive: that needs an operating-system drag source.
 * The plugin provides one, called at the moment a drag leaves the window, so a
 * drag that starts inside Lyre can be finished in Discord, a mail client or
 * LocalSend.
 *
 * The carry mode goes with it, so the platform shows the same copy or move
 * cursor the badge inside the window was showing.
 *
 * Platform support is the plugin's: Windows, macOS and Linux X11 have it. On
 * Wayland the request can be refused, and that is reported rather than dropped
 * silently.
 */
export async function startNativeDrag(
  paths: string[],
  mode: "copy" | "move",
): Promise<string | null> {
  if (!isTauri || paths.length === 0) return null;
  try {
    const [{ startDrag }, { invoke }] = await Promise.all([
      import("@crabnebula/tauri-plugin-drag"),
      import("@tauri-apps/api/core"),
    ]);
    void invoke("trace", {
      message: `drag out: ${paths.length} item(s), mode ${mode}`,
    }).catch(() => {});
    const icon = await invoke<string>("drag_icon");
    await startDrag({ item: paths, icon, mode });
    void invoke("trace", { message: "drag out: started" }).catch(() => {});
    return null;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    void import("@tauri-apps/api/core")
      .then(({ invoke }) =>
        invoke("trace", { message: `drag out: failed: ${message}` }),
      )
      .catch(() => {});
    return message;
  }
}
