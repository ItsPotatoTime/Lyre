/**
 * Layout density. Explorer ships two: the default list and the compact one
 * from View > Compact view.
 *
 * These numbers are also read by the virtualiser, so the stylesheet and the
 * scroll maths cannot drift apart. Row height is measured from the reference
 * captures: the ink clusters of consecutive rows sit 28px apart.
 */

export type Mode = "explorer" | "compact";

export interface Metrics {
  rowHeight: number;
  columnHeaderHeight: number;
  tileWidth: number;
  tileHeight: number;
}

export const METRICS: Record<Mode, Metrics> = {
  explorer: { rowHeight: 28, columnHeaderHeight: 26, tileWidth: 96, tileHeight: 104 },
  compact: { rowHeight: 22, columnHeaderHeight: 22, tileWidth: 80, tileHeight: 88 },
};

export const metricsFor = (mode: Mode): Metrics => METRICS[mode];

/**
 * Default column widths, measured from what each column has to show rather than
 * from where its text starts: a date is 149px at this size, "Application" 84,
 * and a size 62. The date column used to be 122, which cut the "AM" off every
 * row in the list.
 */
export const DEFAULT_COLUMNS: [number, number, number, number] = [271, 149, 90, 66];

export const MIN_COLUMN = 56;

/**
 * The order columns give up width in when the table has to fit a narrower list:
 * the name first, because a name with an ellipsis is still a name, then the
 * type, then the size, and the date last, because a date cut in half says
 * nothing at all. Indexes into a `ColumnWidths`.
 */
const SHRINK_ORDER = [0, 2, 3, 1] as const;

/**
 * A column dragged to `width`, with the difference paid for by the other
 * columns in `SHRINK_ORDER` so the table still fits the list.
 *
 * Without this, widening a column in a window that already needs every pixel
 * does nothing at all: the drag is stored, the fit hands the width straight
 * back to the column being dragged, and the grip appears dead. Widening is
 * granted only as far as the other columns have room above their minimums,
 * which is the same rule that keeps the table inside the list.
 */
export function resizeColumns(
  columns: [number, number, number, number],
  minimums: [number, number, number, number],
  index: number,
  width: number,
): [number, number, number, number] {
  const next = [...columns] as [number, number, number, number];
  const target = Math.max(minimums[index]!, width);
  let wanted = target - next[index]!;
  if (wanted <= 0) {
    next[index] = target;
    return next;
  }

  for (const other of SHRINK_ORDER) {
    if (wanted === 0) break;
    if (other === index) continue;
    const room = Math.max(0, next[other]! - minimums[other]!);
    const given = Math.min(room, wanted);
    next[other] = next[other]! - given;
    wanted -= given;
  }
  next[index] = target - wanted;
  return next;
}

/**
 * Column widths narrowed to fit the space the list actually has.
 *
 * The stored widths are what the person chose, so they are handed back
 * untouched whenever they fit. When they do not, which is any ordinary window
 * once the preview pane is open, the table gives up exactly as much as it is
 * over by, taken in `SHRINK_ORDER` and never below a column's minimum. That
 * keeps all four columns on screen instead of losing the last two off the edge,
 * under the pane, where nothing can be read or scrolled to.
 *
 * Below the sum of the minimums there is nothing left to give up. The table is
 * then as narrow as it can be and scrolls, which is what a scrollbar is for.
 */
export function fitColumns(
  columns: [number, number, number, number],
  minimums: [number, number, number, number],
  available: number,
): [number, number, number, number] {
  const total = columns.reduce((sum, width) => sum + width, 0);
  // Not measured yet, or already fits: leave the chosen widths alone.
  if (available <= 0 || total <= available) return columns;

  const fitted = [...columns] as [number, number, number, number];
  let over = total - available;
  for (const index of SHRINK_ORDER) {
    if (over <= 0) break;
    const room = Math.max(0, fitted[index]! - minimums[index]!);
    const given = Math.min(room, over);
    fitted[index] = fitted[index]! - given;
    over -= given;
  }
  return fitted;
}
