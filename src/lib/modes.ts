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

/** Default column widths, measured from where each column's text starts. */
export const DEFAULT_COLUMNS: [number, number, number, number] = [271, 122, 122, 82];

export const MIN_COLUMN = 56;
