import { isTauri } from "./ipc";

/**
 * Handing a drag to the operating system.
 *
 * Internal dragging is pointer-driven, which cannot by itself start a drag that
 * other applications can receive: that needs an operating-system drag source.
 * The plugin provides one, called at the moment a drag leaves the window, so a
 * drag that starts inside Lyre can be finished in Explorer, Firefox, Discord
 * or a mail client.
 *
 * `DoDragDrop` is a modal loop that has to start while the mouse button is
 * still down. Resolving two dynamic imports and an IPC call at the moment of
 * handoff used to take long enough that the button was often already up, and
 * the drop landed on whatever window happened to be under the cursor —
 * sometimes Lyre itself. Everything that can be loaded is therefore warmed at
 * startup, leaving exactly one IPC call on the handoff path.
 *
 * The carry mode goes with it for the cursor; the patched drag crate advertises
 * both copy and move so the target, not Lyre, decides which one is allowed.
 *
 * Platform support is the plugin's: Windows, macOS and Linux X11 have it. On
 * Wayland the request can be refused, and that is reported rather than dropped
 * silently.
 */

type StartDrag = typeof import("@crabnebula/tauri-plugin-drag")["startDrag"];
type Invoke = typeof import("@tauri-apps/api/core")["invoke"];

interface Ready {
  startDrag: StartDrag;
  invoke: Invoke;
  icon: string;
}

let ready: Promise<Ready> | null = null;
let active = false;
/** Incremented on every pointer release, so a late drag can be abandoned. */
let releases = 0;

/** Load the plugin module and prepare the drag icon before anyone needs them. */
export function warmNativeDrag(): void {
  if (!isTauri) return;
  void prepare().catch(() => {
    ready = null;
  });
}

function prepare(): Promise<Ready> {
  if (!ready) {
    ready = (async () => {
      const [plugin, core] = await Promise.all([
        import("@crabnebula/tauri-plugin-drag"),
        import("@tauri-apps/api/core"),
      ]);
      const icon = await core.invoke<string>("drag_icon");
      return { startDrag: plugin.startDrag, invoke: core.invoke, icon };
    })().catch((error) => {
      ready = null;
      throw error;
    });
  }
  return ready;
}

/** A pointer went up. Used to tell a stale handoff from a live gesture. */
export function notePointerRelease(): void {
  releases += 1;
}

/** Whether Lyre is currently the source of an operating-system drag. */
export function isNativeDragActive(): boolean {
  return active;
}

export async function startNativeDrag(
  paths: string[],
  mode: "copy" | "move",
): Promise<string | null> {
  if (!isTauri || paths.length === 0) return null;
  const releaseAtStart = releases;
  try {
    const { startDrag, invoke, icon } = await prepare();
    // If the button came up while the round trip was in flight, starting the
    // modal loop now would drop the files at the current cursor position
    // rather than where the person meant to let go.
    if (releases !== releaseAtStart) return null;
    void invoke("trace", {
      message: `drag out: ${paths.length} item(s), mode ${mode}`,
    }).catch(() => {});
    active = true;
    try {
      await startDrag({ item: paths, icon, mode });
    } finally {
      active = false;
    }
    void invoke("trace", { message: "drag out: started" }).catch(() => {});
    return null;
  } catch (error) {
    active = false;
    const message = error instanceof Error ? error.message : String(error);
    void import("@tauri-apps/api/core")
      .then(({ invoke }) =>
        invoke("trace", { message: `drag out: failed: ${message}` }),
      )
      .catch(() => {});
    return message;
  }
}
