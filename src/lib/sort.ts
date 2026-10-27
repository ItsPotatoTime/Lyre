import type { Entry, SortKey, SortState } from "@/types";

/**
 * ICU's collator, not a hand-written comparator: it is correct for every
 * locale the user might run in, treats digit runs as numbers, and is
 * implemented in C++ rather than in the loop that sorts 100,000 rows.
 */
const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

export function compareEntries(a: Entry, b: Entry, sort: SortState): number {
  // Folders group above files in every Explorer sort order.
  if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;

  const sign = sort.dir;
  switch (sort.key) {
    case "name": {
      const byName = collator.compare(a.name, b.name);
      return (byName !== 0 ? byName : compareRaw(a.name, b.name)) * sign;
    }
    case "modified": {
      const am = a.modifiedMs ?? 0;
      const bm = b.modifiedMs ?? 0;
      if (am !== bm) return (am - bm) * sign;
      return collator.compare(a.name, b.name);
    }
    case "size": {
      if (a.size !== b.size) return (a.size - b.size) * sign;
      return collator.compare(a.name, b.name);
    }
    case "type": {
      const at = a.isDir ? 0 : 1;
      const bt = b.isDir ? 0 : 1;
      if (at !== bt) return at - bt;
      const byKind = collator.compare(a.kind, b.kind);
      if (byKind !== 0) return byKind * sign;
      const byExt = collator.compare(a.ext, b.ext);
      if (byExt !== 0) return byExt * sign;
      return collator.compare(a.name, b.name);
    }
  }
}

function compareRaw(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

export function sortEntries(entries: Entry[], sort: SortState): Entry[] {
  // `slice` because the caller may hold the array across renders.
  return entries.slice().sort((a, b) => compareEntries(a, b, sort));
}

/**
 * Merge two already-sorted runs in one pass. Batches arrive every few
 * milliseconds during a long scan, so the list is kept sorted incrementally
 * instead of being re-sorted from scratch on each arrival.
 */
export function mergeSorted(into: Entry[], add: Entry[], sort: SortState): Entry[] {
  if (add.length === 0) return into;
  if (into.length === 0) return add;
  // Appending is the common case for a scan whose directory order already
  // matches the comparator, and it is O(add).
  if (compareEntries(into[into.length - 1]!, add[0]!, sort) <= 0) {
    into.push(...add);
    return into;
  }
  const out: Entry[] = new Array(into.length + add.length);
  let i = 0;
  let j = 0;
  let k = 0;
  while (i < into.length && j < add.length) {
    if (compareEntries(into[i]!, add[j]!, sort) <= 0) out[k++] = into[i++]!;
    else out[k++] = add[j++]!;
  }
  while (i < into.length) out[k++] = into[i++]!;
  while (j < add.length) out[k++] = add[j++]!;
  return out;
}

export function sortLabel(key: SortKey): string {
  switch (key) {
    case "name":
      return "Name";
    case "modified":
      return "Date modified";
    case "type":
      return "Type";
    case "size":
      return "Size";
  }
}
