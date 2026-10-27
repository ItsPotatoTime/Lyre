import type { DirStatus } from "@/lib/useDirectory";
import { formatCount } from "@/lib/format";

/**
 * Explorer's status bar, plus one addition: how long the directory took to
 * read. That number is the product's whole claim, so it is stated as plain
 * fact next to the item count rather than hidden in a debug panel.
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
            <strong>{formatCount(total)}</strong>{" "}
            {total === 1 ? "item" : "items"}
          </span>
          {selectedCount > 0 ? (
            <span className="tabular">
              <strong>{formatCount(selectedCount)}</strong> selected
            </span>
          ) : null}
        </>
      )}

      <span className="status-spacer" />

      {filtering ? (
        <span className="tabular">
          {formatCount(visibleCount)} of {formatCount(total)} match
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
