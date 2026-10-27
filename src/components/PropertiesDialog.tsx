import { useEffect, useRef } from "react";
import type { PathProperties } from "@/lib/ipc";
import { formatDate, formatSize } from "@/lib/format";
import { Icon } from "./Icons";
import { FileGlyph } from "./FileGlyph";

/**
 * Properties, as a small dialog rather than a flyout: it is a thing you read,
 * not a command you issue, and the reference's own Properties window is modal.
 *
 * Folder size is deliberately absent. Totalling a tree is a walk of its own,
 * and showing an instant number that turns out to be wrong would be worse than
 * showing none; the immediate item count is reported instead.
 */
export function PropertiesDialog({
  properties,
  onClose,
}: {
  properties: PathProperties;
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

  const rows: [string, string][] = [
    ["Type", properties.isDir ? "File folder" : "File"],
    [
      "Location",
      properties.parent ?? "—",
    ],
    ...(properties.isDir
      ? ([
          [
            "Contains",
            properties.items === null
              ? "—"
              : `${properties.items} ${properties.items === 1 ? "item" : "items"}`,
          ],
        ] as [string, string][])
      : ([
          ["Size", formatSize(properties.size ?? 0, false) || "0 bytes"],
        ] as [string, string][])),
    ["Modified", formatDate(properties.modifiedMs) || "—"],
    ["Attributes", properties.readonly ? "Read-only" : "Read and write"],
  ];

  return (
    <div
      className="dialog-scrim"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-label={`Properties for ${properties.name}`}
        tabIndex={-1}
        ref={ref}
      >
        <header className="dialog-head">
          <span className="dialog-glyph">
            <FileGlyph
              kind={properties.isDir ? "folder" : "file"}
              size={28}
            />
          </span>
          <span className="dialog-title" title={properties.name}>
            {properties.name}
          </span>
          <button
            type="button"
            className="dialog-close"
            aria-label="Close"
            onClick={onClose}
          >
            <Icon name="close" size={10} strokeWidth={1.6} />
          </button>
        </header>

        <dl className="dialog-rows">
          {rows.map(([label, value]) => (
            <div className="dialog-row" key={label}>
              <dt>{label}</dt>
              <dd title={value}>{value}</dd>
            </div>
          ))}
        </dl>

        <footer className="dialog-foot">
          <button type="button" className="dialog-action" onClick={onClose}>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}
