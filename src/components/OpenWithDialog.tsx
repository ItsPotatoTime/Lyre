import { useEffect, useRef, useState } from "react";
import type { OpenWith } from "@/lib/ipc";
import { Icon } from "./Icons";
import { FileGlyph } from "./FileGlyph";

/**
 * Explorer's "Open with" dialog.
 *
 * The list is what the desktop itself says supports this file type, and the
 * current default is preselected. The checkbox is Explorer's "Always use this
 * app", and it is the only thing that changes the system default; opening
 * without it is a one-off.
 */
export function OpenWithDialog({
  fileName,
  isDir,
  data,
  busy,
  error,
  onOpen,
  onUseSystem,
  onCancel,
}: {
  fileName: string;
  isDir: boolean;
  data: OpenWith | null;
  busy: boolean;
  error: string | null;
  onOpen: (appId: string, remember: boolean) => void;
  onUseSystem: () => void;
  onCancel: () => void;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [remember, setRemember] = useState(true);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (data && selected === null) {
      setSelected(data.defaultApp ?? data.apps[0]?.id ?? null);
    }
  }, [data, selected]);

  useEffect(() => {
    ref.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) {
        event.stopPropagation();
        event.preventDefault();
        onCancel();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [busy, onCancel]);

  const unavailable = data?.unsupported ?? null;
  const apps = data?.apps ?? [];
  // Windows keeps its application list in package manifests and hash-protects
  // the default, so its own picker is the only correct answer there.
  const systemPicker = data?.systemPicker === true;

  return (
    <div
      className="dialog-scrim"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
    >
      <div
        className="dialog dialog-wide"
        role="dialog"
        aria-modal="true"
        aria-label={`Open ${fileName} with`}
        tabIndex={-1}
        ref={ref}
      >
        <header className="dialog-head">
          <span className="dialog-glyph">
            <FileGlyph kind={isDir ? "folder" : "file"} size={28} />
          </span>
          <span className="dialog-title" title={fileName}>
            Open “{fileName}” with
          </span>
          <button
            type="button"
            className="dialog-close"
            aria-label="Close"
            onClick={onCancel}
            disabled={busy}
          >
            <Icon name="close" size={10} strokeWidth={1.6} />
          </button>
        </header>

        {error || unavailable ? (
          <p className="dialog-note">{error ?? unavailable}</p>
        ) : systemPicker ? (
          <>
            <p className="dialog-note">
              Windows keeps the list of applications and protects the default,
              so it chooses here. Pick an application and tick{" "}
              <strong>Always use this app</strong> to make it the default
              {data?.defaultApp ? (
                <>
                  {" "}
                  (currently <strong>{data.defaultApp}</strong>)
                </>
              ) : null}
              .
            </p>
          </>
        ) : data === null ? (
          <p className="dialog-note">Looking for applications that can open this…</p>
        ) : apps.length === 0 ? (
          <p className="dialog-note">
            No application on this system says it can open {data.mime} files.
          </p>
        ) : (
          <div className="app-list" role="radiogroup" aria-label="Applications">
            {apps.map((app) => (
              <button
                key={app.id}
                type="button"
                role="radio"
                aria-checked={selected === app.id}
                className="app-row"
                data-selected={selected === app.id ? "true" : "false"}
                onClick={() => setSelected(app.id)}
                onDoubleClick={() => onOpen(app.id, remember)}
              >
                <span className="app-radio" aria-hidden="true">
                  {selected === app.id ? <span className="app-dot" /> : null}
                </span>
                <span className="app-name">{app.name}</span>
                {data.defaultApp === app.id ? (
                  <span className="app-default">Default</span>
                ) : null}
              </button>
            ))}
          </div>
        )}

        {systemPicker ? null : (
          <label className="dialog-check">
            <input
              type="checkbox"
              checked={remember}
              disabled={apps.length === 0 || busy}
              onChange={(event) => setRemember(event.target.checked)}
            />
            Always use this application to open {data?.mime ?? "these"} files
          </label>
        )}

        <footer className="dialog-foot">
          <button
            type="button"
            className="dialog-action"
            onClick={onCancel}
            disabled={busy}
          >
            Cancel
          </button>
          {systemPicker ? (
            <button
              type="button"
              className="dialog-action dialog-action-primary"
              disabled={busy}
              onClick={onUseSystem}
            >
              Choose an app…
            </button>
          ) : (
            <button
              type="button"
              className="dialog-action dialog-action-primary"
              disabled={!selected || busy || apps.length === 0}
              onClick={() => selected && onOpen(selected, remember)}
            >
              {busy ? "Opening…" : "Open"}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}
