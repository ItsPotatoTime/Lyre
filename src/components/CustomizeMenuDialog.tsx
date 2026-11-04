import { useEffect, useMemo, useRef } from "react";
import { Icon } from "./Icons";
import {
  menuItemShown,
  menuSpecs,
  uniqueShellEntries,
  type MenuScope,
  type MenuSpec,
} from "@/lib/menuItems";
import type { ShellMenuCatalog, ShellMenuEntry } from "@/lib/ipc";

const SCOPES: Record<MenuScope, { detail: string; note: string | null }> = {
  item: {
    detail: "Files and folders",
    note: null,
  },
  background: {
    detail: "Folder background",
    note: "The menu a right-click opens on the empty space in a folder.",
  },
  place: {
    detail: "Places",
    note: "The menu a right-click opens on a sidebar row, a tile, or a drive.",
  },
};

/** The program that added an item, unless the label already says it. */
function shellDetail(entry: ShellMenuEntry): string | null {
  const parts: string[] = [];
  if (entry.children.length > 0) {
    parts.push(`${entry.children.length} items`);
  }
  const source = entry.source.trim();
  if (
    source &&
    source !== "Windows" &&
    !entry.label.toLowerCase().includes(source.toLowerCase())
  ) {
    parts.push(source);
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

/**
 * The list behind "Customize this menu".
 *
 * Two inventories in one place, because they are the same decision: Lyre's own
 * items, and the ones other programs registered with the system. A tick means
 * the item appears. Nothing is saved with a button: the menu is rebuilt every
 * time it opens, so a change shows the next time it is opened and there is no
 * half-applied state to explain.
 *
 * The system's list is read for the item that was right-clicked, so it names
 * what that file type or folder actually offers rather than every registration
 * on the machine.
 */
export function CustomizeMenuDialog({
  scope,
  shell,
  hidden,
  shown,
  onToggle,
  onReset,
  onClose,
}: {
  scope: MenuScope;
  /** `null` while the system is still being read. */
  shell: ShellMenuCatalog | null;
  hidden: string[];
  shown: string[];
  onToggle: (id: string, defaultOn: boolean, next: boolean) => void;
  onReset: () => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    ref.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [onClose]);

  const hiddenSet = useMemo(() => new Set(hidden), [hidden]);
  const shownSet = useMemo(() => new Set(shown), [shown]);
  const isShown = (id: string, defaultOn: boolean) =>
    menuItemShown(id, defaultOn, hiddenSet, shownSet);

  const specs = menuSpecs(scope);
  const shellRows = shell?.supported ? uniqueShellEntries(shell.entries, scope) : [];
  // A catalogue crosses an IPC boundary, so the list is read with a default
  // even though the interface requires it.
  const shellWarnings = shell?.warnings ?? [];
  const total = specs.length + shellRows.length;
  const visible =
    specs.filter((spec) => isShown(spec.id, true)).length +
    shellRows.filter((entry) => isShown(entry.id, entry.defaultOn)).length;
  const customized = hidden.length > 0 || shown.length > 0;

  return (
    <div
      className="dialog-scrim"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="dialog dialog-menu"
        role="dialog"
        aria-modal="true"
        aria-labelledby="customize-menu-title"
        tabIndex={-1}
        ref={ref}
      >
        <header className="dialog-head">
          <span className="dialog-glyph">
            <Icon name="sliders" size={20} />
          </span>
          <div className="dialog-titles">
            <span className="dialog-title" id="customize-menu-title">
              Customize context menu
            </span>
            <span className="dialog-subtitle">{SCOPES[scope].detail}</span>
          </div>
          <button
            type="button"
            className="dialog-close"
            aria-label="Close"
            onClick={onClose}
          >
            <Icon name="close" size={10} strokeWidth={1.6} />
          </button>
        </header>

        {SCOPES[scope].note ? (
          <p className="dialog-note">{SCOPES[scope].note}</p>
        ) : null}

        <div className="menu-settings">
          <Section title="Lyre">
            <SpecRows specs={specs} isShown={isShown} onToggle={onToggle} />
          </Section>

          {scope === "place" ? null : (
            <Section title="Other programs">
              {shell === null ? (
                <p className="menu-settings-empty">
                  Reading what other programs offer…
                </p>
              ) : !shell.supported ? (
                <p className="menu-settings-empty">{shell.note}</p>
              ) : (
                <>
                  {shellRows.length === 0 && shellWarnings.length === 0 ? (
                    <p className="menu-settings-empty">
                      No other program has added anything here.
                    </p>
                  ) : null}
                  {shellRows.map((entry) => (
                    <Row
                      key={entry.id}
                      label={entry.label || entry.source}
                      detail={shellDetail(entry)}
                      checked={isShown(entry.id, entry.defaultOn)}
                      onChange={(next) => onToggle(entry.id, entry.defaultOn, next)}
                    />
                  ))}
                  {shellWarnings.length > 0 ? (
                    <div className="menu-settings-warnings">
                      <div className="menu-settings-warnings-head">
                        {shellWarnings.length} registration
                        {shellWarnings.length === 1 ? "" : "s"} could not be read
                      </div>
                      {shellWarnings.map((warning, index) => (
                        <p className="menu-settings-warning" key={index}>
                          {warning}
                        </p>
                      ))}
                    </div>
                  ) : null}
                  {/* A platform can support the menu and still be missing a
                      whole class of item. Say which one. */}
                  {shell.note ? (
                    <p className="menu-settings-empty">{shell.note}</p>
                  ) : null}
                </>
              )}
            </Section>
          )}
        </div>

        <footer className="dialog-foot dialog-foot-split">
          <button
            type="button"
            className="dialog-link"
            disabled={!customized}
            onClick={onReset}
          >
            Reset
          </button>
          <span className="dialog-count">
            {visible} of {total} shown
          </span>
          <span className="dialog-gap" />
          <button
            type="button"
            className="dialog-action dialog-action-primary"
            onClick={onClose}
          >
            Done
          </button>
        </footer>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="menu-settings-section">
      <header className="menu-settings-head">
        <span>{title}</span>
        <span className="menu-settings-rule" aria-hidden="true" />
      </header>
      {children}
    </section>
  );
}

/**
 * Lyre's items, with a heading wherever the menu itself draws a division. A
 * group of one gets no heading: a title above a single row is noise.
 */
function SpecRows({
  specs,
  isShown,
  onToggle,
}: {
  specs: MenuSpec[];
  isShown: (id: string, defaultOn: boolean) => boolean;
  onToggle: (id: string, defaultOn: boolean, next: boolean) => void;
}) {
  const groups: { title: string; items: MenuSpec[] }[] = [];
  for (const spec of specs) {
    const last = groups[groups.length - 1];
    if (last && last.title === spec.group) last.items.push(spec);
    else groups.push({ title: spec.group, items: [spec] });
  }

  return (
    <>
      {groups.map((group) => (
        <div className="menu-settings-group" key={group.title}>
          {group.items.length > 1 ? (
            <div className="menu-settings-group-title">{group.title}</div>
          ) : null}
          {group.items.map((spec) => (
            <Row
              key={spec.id}
              label={spec.label}
              detail={spec.detail ?? null}
              checked={isShown(spec.id, true)}
              onChange={(next) => onToggle(spec.id, true, next)}
            />
          ))}
        </div>
      ))}
    </>
  );
}

function Row({
  label,
  detail,
  checked,
  onChange,
}: {
  label: string;
  detail: string | null;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <label className="menu-row">
      <input
        type="checkbox"
        className="menu-row-input"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="menu-row-box" aria-hidden="true">
        <Icon name="check" size={11} strokeWidth={2.2} />
      </span>
      <span className="menu-row-label">{label}</span>
      {detail ? <span className="menu-row-detail">{detail}</span> : null}
    </label>
  );
}
