import { useEffect, useState } from "react";
import { previewThumbnail } from "./ipc";

/**
 * Thumbnails for the rows and tiles themselves.
 *
 * The rules here exist because the naive version of this froze the application:
 *
 * - A request is made only for a row that is still on screen shortly after it
 *   appears, so scrolling past a folder of photographs does not queue work for
 *   every one of them.
 * - Cancelling removes the task from the queue. Marking it cancelled but
 *   leaving it queued meant ffmpeg still ran for every row ever rendered.
 * - At most a few run at once, because each may spawn a process that decodes an
 *   entire image.
 * - Results live in a bounded cache, since a folder can hold thousands of
 *   pictures and keeping every thumbnail forever is the memory problem this app
 *   already spent a session undoing.
 * - A finished picture is handed to the rows waiting for it, not only stored.
 *   Storing alone is invisible to a row that has already rendered: it read the
 *   cache when it mounted, found nothing, and was never told otherwise. That is
 *   why a picture appeared only after something remounted the row, which is what
 *   made the icons in Details look like they needed a trip through Large icons
 *   to acquire their pictures.
 */

const MAX_CACHED = 240;
const MAX_IN_FLIGHT = 3;
/// How long a row must stay on screen before its thumbnail is worth making.
const SETTLE_MS = 140;
/// Row icons skip files this large: decoding them costs more than it is worth
/// on a row, and the preview pane still shows them at full quality.
const ROW_MAX_BYTES = 24 * 1024 * 1024;

const cache = new Map<string, string>();
/// Rows waiting for a picture, keyed by path. One request serves every row
/// showing the same file, and a row that goes away stops waiting.
type Listener = (url: string | null) => void;
const waiting = new Map<string, Set<Listener>>();
/// Paths with a request in the queue, and paths being fetched right now. Both
/// keep a second row from asking for the same file again.
const queued = new Set<string>();
const running = new Set<string>();
const queue: string[] = [];
let inFlight = 0;

function remember(path: string, url: string) {
  cache.set(path, url);
  while (cache.size > MAX_CACHED) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    const stale = cache.get(oldest);
    cache.delete(oldest);
    if (stale) URL.revokeObjectURL(stale);
  }
  for (const listener of waiting.get(path) ?? []) listener(url);
}

function pump() {
  while (inFlight < MAX_IN_FLIGHT) {
    const path = queue.shift();
    if (path === undefined) return;
    queued.delete(path);
    inFlight += 1;
    running.add(path);
    previewThumbnail(path)
      .then((url) => {
        if (url) remember(path, url);
      })
      .catch(() => {
        // A file that cannot be previewed keeps its glyph.
      })
      .finally(() => {
        inFlight -= 1;
        running.delete(path);
        pump();
      });
  }
}

/** Ask for one picture, unless it is queued, in flight, or already here. */
function request(path: string) {
  if (queued.has(path) || running.has(path) || cache.has(path)) return;
  queue.push(path);
  queued.add(path);
  pump();
}

/** Drop a queued request that nothing on screen is waiting for any more. */
function withdraw(path: string) {
  const at = queue.indexOf(path);
  if (at >= 0) {
    queue.splice(at, 1);
    queued.delete(path);
  }
}

/** Extensions worth showing a picture for. */
export function previewable(name: string): boolean {
  return /\.(png|jpe?g|gif|webp|bmp|avif|heic|mp4|mkv|webm|mov|m4v|pdf)$/i.test(name);
}

export function useThumbnail(
  path: string,
  enabled: boolean,
  bytes: number,
): string | null {
  const [url, setUrl] = useState<string | null>(() =>
    enabled ? (cache.get(path) ?? null) : null,
  );

  useEffect(() => {
    if (!enabled || bytes > ROW_MAX_BYTES) {
      setUrl(null);
      return;
    }
    const cached = cache.get(path);
    if (cached) {
      setUrl(cached);
      return;
    }
    setUrl(null);

    let joined = waiting.get(path);
    if (!joined) {
      joined = new Set();
      waiting.set(path, joined);
    }
    const listener: Listener = (next) => setUrl(next);
    joined.add(listener);

    // Wait for the row to settle, so a fast scroll never asks for a picture it
    // has already scrolled past.
    const settle = window.setTimeout(() => request(path), SETTLE_MS);

    return () => {
      window.clearTimeout(settle);
      joined.delete(listener);
      if (joined.size === 0) {
        waiting.delete(path);
        withdraw(path);
      }
      // A picture that already arrived stays in the cache for next time.
    };
  }, [path, enabled, bytes]);

  return url;
}
