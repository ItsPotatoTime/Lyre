import type { ManagerState } from "@/lib/ipc";
import { Icon } from "./Icons";

/**
 * The offer to become the desktop's folder handler, shown once at startup.
 *
 * It sits over the content rather than interrupting the window, because it is
 * an offer and not a gate: the app is fully usable without answering it.
 */
export function DefaultManagerPrompt({
  state,
  onAccept,
  onDismiss,
  onNever,
  busy,
  error,
}: {
  state: ManagerState;
  onAccept: () => void;
  onDismiss: () => void;
  onNever: () => void;
  busy: boolean;
  error: string | null;
}) {
  const unsupported = state.unsupported;
  // The registry route is real but unusual enough to explain rather than hide.
  const windows = state.desktopFile === null && !unsupported;

  return (
    <div className="prompt" role="dialog" aria-label="Default file manager">
      <div className="prompt-head">
        <span className="prompt-glyph">
          <Icon name="folder" size={18} strokeWidth={1.3} />
        </span>
        <span className="prompt-title">
          {unsupported ? "Lyre cannot be the default here" : "Open folders with Lyre"}
        </span>
        <button
          type="button"
          className="prompt-close"
          aria-label="Dismiss"
          onClick={onDismiss}
        >
          <Icon name="close" size={9} strokeWidth={1.8} />
        </button>
      </div>

      <p className="prompt-body">
        {unsupported ??
          (windows
            ? "Windows has no setting for this: Explorer is the shell, and folders are part of it rather than a file type that can be associated. The way every third-party file manager does it is to register a folder handler of its own, so double-clicking a folder opens here instead. It changes nothing else, and it can be undone from the More menu."
            : "Making Lyre the default means folders open here instead of the previous file manager, whether you double-click one on the desktop or another application asks to browse a folder.")}
      </p>

      {error ? <p className="prompt-error">{error}</p> : null}

      {state.current && !unsupported ? (
        <p className="prompt-current">
          Currently handled by <strong>{state.current}</strong>
        </p>
      ) : null}

      <div className="prompt-actions">
        {unsupported ? (
          <button type="button" className="prompt-button-primary" onClick={onDismiss}>
            Got it
          </button>
        ) : (
          <button
            type="button"
            className="prompt-button-primary"
            onClick={onAccept}
            disabled={busy}
          >
            {busy ? "Setting…" : windows ? "Open folders with Lyre" : "Make Lyre the default"}
          </button>
        )}
        <button type="button" className="prompt-button" onClick={onDismiss} disabled={busy}>
          Not now
        </button>
        <button type="button" className="prompt-link" onClick={onNever} disabled={busy}>
          Never show this again
        </button>
      </div>
    </div>
  );
}
