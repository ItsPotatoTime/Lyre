/**
 * Layout density. Explorer ships exactly these two: the default comfortable
 * list and the compact one from View > Compact view. The numbers live here
 * rather than in CSS alone because the virtualiser has to agree with the
 * stylesheet, and a mismatch shows up as rows drifting out of their slots.
 */

export type Mode = "explorer" | "compact";

export interface Metrics {
  rowHeight: number;
  columnHeaderHeight: number;
  tileWidth: number;
  tileHeight: number;
  gridGap: number;
}

export const METRICS: Record<Mode, Metrics> = {
  explorer: {
    rowHeight: 30,
    columnHeaderHeight: 32,
    tileWidth: 104,
    tileHeight: 112,
    gridGap: 0,
  },
  compact: {
    rowHeight: 24,
    columnHeaderHeight: 28,
    tileWidth: 88,
    tileHeight: 96,
    gridGap: 0,
  },
};

export const metricsFor = (mode: Mode): Metrics => METRICS[mode];
