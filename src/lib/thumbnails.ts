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
 */

const MAX_CACHED = 240;
const MAX_IN_FLIGHT = 3;
/// How long a row must stay on screen before its thumbnail is worth making.
const SETTLE_MS = 140;
/// Row icons skip files this large: decoding them costs more than it is worth
/// on a row, and the preview pane still shows them at full quality.
const ROW_MAX_BYTES = 24 * 1024 * 1024;

interface Task {
  path: string;
  cancelled: boolean;
}

const cache = new Map<string, string>();
const queue: Task[] = [];
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
}

function pump() {
  while (inFlight < MAX_IN_FLIGHT) {
    const task = queue.shift();
    if (!task) return;
    if (task.cancelled) continue;
    inFlight += 1;
    previewThumbnail(task.path)
      .then((url) => {
        if (url && !task.cancelled) remember(task.path, url);
        else if (url) URL.revokeObjectURL(url);
      })
      .catch(() => {
        // A file that cannot be previewed keeps its glyph.
      })
      .finally(() => {
        inFlight -= 1;
        pump();
      });
  }
}

/** Queue a thumbnail; the returned function cancels it properly. */
function enqueue(path: string): () => void {
  const task: Task = { path, cancelled: false };
  queue.push(task);
  pump();
  return () => {
    task.cancelled = true;
    const at = queue.indexOf(task);
    if (at >= 0) queue.splice(at, 1);
  };
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

    let cancelTask: (() => void) | null = null;
    // Wait for the row to settle, so a fast scroll never asks for a picture it
    // has already scrolled past.
    const settle = window.setTimeout(() => {
      cancelTask = enqueue(path);
    }, SETTLE_MS);

    return () => {
      window.clearTimeout(settle);
      cancelTask?.();
      // The result, if it already arrived, stays in the cache for next time.
    };
  }, [path, enabled, bytes]);

  return url;
}
