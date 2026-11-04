import { shellMenu, type ShellMenuCatalog } from "./ipc";

/**
 * What the system's own menu offers, kept until it is likely to have changed.
 *
 * Reading it means loading every handler registered for that file type, which is
 * tens of milliseconds and belongs nowhere near the moment a menu opens. So the
 * menu draws whatever is already known, and the answer is fetched when the
 * selection changes, which is always just before someone right-clicks.
 *
 * The key is the file type rather than the path because that is what decides
 * which registrations apply, and because a folder of two hundred photographs
 * must not cost two hundred reads.
 */
interface Cached {
  at: number;
  catalog: ShellMenuCatalog;
}

const READ_AGAIN_AFTER = 5 * 60 * 1000;
const LIMIT = 48;

const cache = new Map<string, Cached>();
const running = new Map<string, Promise<ShellMenuCatalog>>();

export function shellMenuKey(
  path: string | null,
  background: boolean,
  isDir = false,
): string {
  if (background) return `background:${path ?? ""}`;
  if (!path) return "item:";
  // A folder has its own class rather than an extension, and a file with no
  // extension is not a folder, so the two must not share an answer.
  if (isDir) return "item:folder";
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const dot = path.lastIndexOf(".");
  const extension = dot > slash ? path.slice(dot).toLowerCase() : "";
  return `item:file:${extension}`;
}

/** What is already known, for a menu that has to draw now. */
export function knownShellMenu(key: string): ShellMenuCatalog | null {
  const entry = cache.get(key);
  if (!entry) return null;
  if (Date.now() - entry.at > READ_AGAIN_AFTER) {
    cache.delete(key);
    return null;
  }
  return entry.catalog;
}

export function loadShellMenu(
  key: string,
  path: string | null,
  background: boolean,
): Promise<ShellMenuCatalog> {
  const ready = knownShellMenu(key);
  if (ready) return Promise.resolve(ready);
  const started = running.get(key);
  if (started) return started;

  const request = shellMenu(path, background)
    .then((catalog) => {
      cache.set(key, { at: Date.now(), catalog });
      // Bounded, oldest first: a machine that browses every file type on a
      // volume must not accumulate one entry per type forever.
      while (cache.size > LIMIT) {
        const oldest = cache.keys().next().value;
        if (oldest === undefined) break;
        cache.delete(oldest);
      }
      return catalog;
    })
    .catch((error: unknown) => ({
      supported: false,
      note:
        error instanceof Error
          ? error.message
          : "The system's own menu could not be read.",
      entries: [],
      warnings: [],
    }))
    .finally(() => running.delete(key));

  running.set(key, request);
  return request;
}
