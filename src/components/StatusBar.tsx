import type { DirStatus } from "@/lib/useDirectory";
import { formatCount } from "@/lib/format";

/**
 * Explorer's status bar: item count, then selection, separated by a rule.
 * The scan time on the right is the one addition, because it is the whole
 * claim the product makes.
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
}: {
  status: DirStatus;
  scanned: number;
  total: number;
  elapsedMs: number | null;
  selectedCount: number;
  visibleCount: number;
  filtering: boolean;
  notice: string | null;
}) {
  const scanning = status === "loading";
  const plural = (n: number) => (n === 1 ? "item" : "items");

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
