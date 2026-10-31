import type { DirStatus } from "@/lib/useDirectory";
import type { SearchState } from "@/types";
import { formatCount } from "@/lib/format";
import { useFrameRate } from "@/lib/frameRate";

/**
 * Explorer's status bar: item count, then selection, separated by a rule.
 * The measurements on the right are the one addition, because speed is the
 * whole claim the product makes: how long the folder took to read, and, when
 * asked for, what the paint loop is doing right now.
 */
export function StatusBar({
  status,
  scanned,
  total,
  elapsedMs,
  selectedCount,
  visibleCount,
  filtering,
  notice,
  search,
  showFrameRate,
}: {
  status: DirStatus;
  scanned: number;
  total: number;
  elapsedMs: number | null;
  selectedCount: number;
  visibleCount: number;
  filtering: boolean;
  notice: string | null;
  search: SearchState | null;
  /** Whether the paint loop is being reported on the right. */
  showFrameRate: boolean;
}) {
  const scanning = status === "loading";
  const plural = (n: number) => (n === 1 ? "item" : "items");
  // Sampled here rather than in the window, so a readout that updates once a
  // second redraws this footer and nothing else.
  const meter = useFrameRate(showFrameRate);
  const readout = meter ? (
    <span
      className="tabular"
      title={`${formatCount(meter.frames)} frames in the last second, median ${meter.interval.toFixed(1)} ms, fastest ${meter.fastest.toFixed(1)} ms, slowest ${meter.slowest.toFixed(1)} ms`}
    >
      {meter.hz > 0 ? `${meter.hz} Hz` : "measuring"}
    </span>
  ) : null;

  // While searching, the count that matters is matches, not items, and the
  // walk reports how much of the tree it has looked at.
  if (search && search.phase !== "idle") {
    const busy = search.phase === "local" || search.phase === "walking";
    return (
      <footer className="statusbar">
        {search.error ? (
          <span>{search.error}</span>
        ) : (
          <>
            <span className="tabular">
              {formatCount(search.matched)}{" "}
              {search.matched === 1 ? "result" : "results"}
              {search.truncated ? " (showing the best)" : ""}
            </span>
            {search.scanned > 0 ? (
              <>
                <span className="status-sep">|</span>
                <span className="tabular">
                  {formatCount(search.scanned)} examined
                  {search.truncated && search.phase === "done" ? ", stopped early" : ""}
                </span>
              </>
            ) : null}
            <span className="status-spacer" />
            {readout}
            {busy ? (
              <span className="status-scan">
                <span className="status-dot" />
                Searching
              </span>
            ) : null}
            {search.elapsedMs !== null ? (
              <span className="tabular" title="Time to return results">
                {search.elapsedMs < 1 ? "<1" : formatCount(search.elapsedMs)} ms
              </span>
            ) : null}
          </>
        )}
      </footer>
    );
  }

  return (
    <footer className="statusbar">
      {notice ? (
        <span className="status-notice">{notice}</span>
      ) : scanning ? (
        <>
          <span className="status-scan">
            <span className="status-dot" />
            Reading folder
          </span>
          <span className="tabular">{formatCount(scanned)} items</span>
        </>
      ) : status === "error" ? (
        <span>Folder unavailable</span>
      ) : (
        <>
          <span className="tabular">
            {formatCount(total)} {plural(total)}
          </span>
          {selectedCount > 0 ? (
            <>
              <span className="status-sep">|</span>
              <span className="tabular">
                {formatCount(selectedCount)} {plural(selectedCount)} selected
              </span>
            </>
          ) : null}
        </>
      )}

      <span className="status-spacer" />

      {readout}

      {filtering ? (
        <span className="tabular">
          {formatCount(visibleCount)} shown
        </span>
      ) : null}

      {elapsedMs !== null && !scanning ? (
        <span className="tabular" title="Time to read this folder">
          {elapsedMs < 1 ? "<1" : formatCount(elapsedMs)} ms
        </span>
      ) : null}
    </footer>
  );
}
