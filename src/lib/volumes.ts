import type { Place } from "@/types";

/**
 * Which volume a path lives on.
 *
 * The answer decides what a drag does before any modifier is pressed: within
 * one volume moving is what people mean, and across volumes copying is, because
 * a move would mean reading and rewriting the whole thing anyway. That is the
 * same rule Windows Explorer follows, and the one a person who has ever dragged
 * a file onto a USB stick expects.
 *
 * The identity is the device rather than the mount path, so two folders mounted
 * from one disk count as the same volume.
 */
export function volumeKeyOf(path: string, places: Place[]): string | null {
  let best: Place | null = null;

  for (const place of places) {
    // Only volumes; a pinned folder says nothing about the storage under it.
    if (place.kind === "folder") continue;

    const root = place.path;
    if (!root) continue;
    const boundary = root.endsWith("/") || root.endsWith("\\") ? root : `${root}/`;
    const contains = path === root || path.startsWith(boundary);
    if (!contains) continue;

    if (!best || root.length > best.path.length) best = place;
  }

  if (!best) return null;
  return best.detail ?? best.path;
}

/** True when every path lives on the same volume as the target. */
export function sameVolume(paths: string[], target: string, places: Place[]): boolean {
  const destination = volumeKeyOf(target, places);
  if (destination === null) return true;
  return paths.every((path) => {
    const source = volumeKeyOf(path, places);
    // An unknown source is treated as local rather than surprising the person
    // by silently copying when they meant to move.
    return source === null || source === destination;
  });
}
