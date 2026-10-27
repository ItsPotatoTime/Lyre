import { useEffect, useState } from "react";
import { previewThumbnail } from "./ipc";

/**
 * Thumbnails for the rows and tiles themselves.
 *
 * Three things make this safe to use inside a virtualized list:
 *
 * - the work is requested only for entries that are actually rendered, and a
 *   row that scrolls away before its thumbnail arrives simply discards it;
 * - at most a few requests are in flight at once, because each one may spawn
 *   `ffmpeg`, and a screenful of icons must not start thirty of them;
 * - results are kept in a bounded cache, since a folder can hold thousands of
 *   pictures and holding every thumbnail forever is the memory problem this app
 *   spent a session undoing.
 */

const MAX_CACHED = 240;
const MAX_IN_FLIGHT = 4;

const cache = new Map<string, string>();
const queue: Array<() => void> = [];
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

function next() {
  if (inFlight >= MAX_IN_FLIGHT) return;
  const task = queue.shift();
  if (!task) return;
  inFlight += 1;
  task();
}

function release() {
  inFlight -= 1;
  next();
}

/** Ask for one thumbnail, respecting the concurrency limit. */
function request(path: string): Promise<void> {
  return new Promise((resolve) => {
    const run = () => {
      previewThumbnail(path)
        .then((url) => {
          if (url) remember(path, url);
        })
        .catch(() => {
          // A file that cannot be previewed is not an error worth showing per
          // row; the generic glyph stays.
        })
        .finally(() => {
          release();
          resolve();
        });
    };
    queue.push(run);
    next();
  });
}

/** Any image or video extension worth previewing in a row. */
export function previewable(name: string): boolean {
  return /\.(png|jpe?g|gif|webp|bmp|avif|heic|svg|mp4|mkv|webm|mov|m4v|pdf)$/i.test(name);
}

export function useThumbnail(path: string, enabled: boolean): string | null {
  const [url, setUrl] = useState<string | null>(() =>
    enabled ? (cache.get(path) ?? null) : null,
  );

  useEffect(() => {
    if (!enabled) {
      setUrl(null);
      return;
    }
    const cached = cache.get(path);
    if (cached) {
      setUrl(cached);
      return;
    }
    let cancelled = false;
    setUrl(null);
    void request(path).then(() => {
      if (cancelled) return;
      const ready = cache.get(path);
      if (ready) setUrl(ready);
    });
    return () => {
      cancelled = true;
    };
  }, [path, enabled]);

  return url;
}
