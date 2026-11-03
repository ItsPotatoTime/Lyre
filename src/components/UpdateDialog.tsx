import { useEffect, useRef } from "react";
import { Icon } from "./Icons";
import { formatSize } from "@/lib/format";
import type { UpdateOffer } from "@/lib/ipc";

/**
 * What the update check found, and what is happening to it.
 *
 * The download is shown as bytes rather than a spinner because a build is a few
 * megabytes over whatever connection the machine has, and a spinner that runs
 * for a minute says less than a number that moves. Nothing is installed until
 * the signature checks out, which is why the Install button can be trusted with
 * a file fetched from the internet.
 */
export function UpdateDialog({
  offer,
  phase,
  downloaded,
  total,
  error,
  onInstall,
  onRestart,
  onClose,
}: {
  offer: UpdateOffer;
  /** offered, downloading, installing, ready, or failed. */
  phase: "offered" | "downloading" | "installing" | "ready" | "failed";
  downloaded: number;
  total: number | null;
  error: string | null;
  onInstall: () => void;
  onRestart: () => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    ref.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && phase !== "downloading" && phase !== "installing") {
        event.stopPropagation();
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [onClose, phase]);

  const percent =
    total && total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : null;
  const busy = phase === "downloading" || phase === "installing";

  return (
    <div
      className="dialog-scrim"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-label={`Lyre ${offer.version}`}
        tabIndex={-1}
        ref={ref}
      >
        <header className="dialog-head">
          <span className="dialog-glyph">
            <Icon name="arrowDown" size={22} strokeWidth={1.4} />
          </span>
          <span className="dialog-title">
            {phase === "ready"
              ? `Lyre ${offer.version} is installed`
              : `Lyre ${offer.version} is available`}
          </span>
        </header>

        {phase === "ready" ? (
          <p className="dialog-note">
            It starts when you restart. Nothing you have open is affected.
          </p>
        ) : phase === "failed" ? (
          <p className="prompt-error">{error ?? "The update could not be installed."}</p>
        ) : (
          <>
            <dl className="dialog-rows">
              <div className="dialog-row">
                <dt>Installed</dt>
                <dd>{offer.current}</dd>
              </div>
              <div className="dialog-row">
                <dt>Available</dt>
                <dd>{offer.version}</dd>
              </div>
            </dl>
            {offer.notes ? (
              <p className="dialog-note update-notes">{offer.notes}</p>
            ) : null}
          </>
        )}

        {busy ? (
          <div className="update-progress">
            <span className="update-progress-track">
              <span
                className="update-progress-fill"
                data-known={percent !== null ? "true" : "false"}
                style={percent !== null ? { width: `${percent}%` } : undefined}
              />
            </span>
            <span className="update-progress-label tabular">
              {phase === "installing"
                ? "Installing"
                : percent !== null
                  ? `${formatSize(downloaded, false)} of ${formatSize(total ?? 0, false)}, ${percent}%`
                  : "Downloading"}
            </span>
          </div>
        ) : null}

        <footer className="dialog-foot">
          {phase === "ready" ? (
            <button
              type="button"
              className="dialog-action dialog-action-primary"
              onClick={onRestart}
            >
              Restart now
            </button>
          ) : busy ? (
            <button type="button" className="dialog-action" disabled>
              {phase === "installing" ? "Installing" : "Downloading"}
            </button>
          ) : (
            <>
              <button type="button" className="dialog-action" onClick={onClose}>
                {phase === "failed" ? "Close" : "Later"}
              </button>
              <button
                type="button"
                className="dialog-action dialog-action-primary"
                onClick={onInstall}
              >
                {phase === "failed" ? "Try again" : "Install"}
              </button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}
