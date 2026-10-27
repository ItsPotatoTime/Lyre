import { useEffect, useState } from "react";
import type { Tab } from "@/types";
import { windowChrome, type ResizeEdge } from "@/lib/window";
import { locationGlyph, locationLabel } from "@/lib/location";
import { Icon } from "./Icons";
import { PlaceGlyph } from "./FileGlyph";

/**
 * The title strip carries the tabs, and nothing else. It is the darkest tone
 * in the window (#0A0A0A in the reference) so the active tab reads as a raised
 * card sitting on it.
 */
export function TitleBar({
  tabs,
  activeId,
  onSelect,
  onClose,
  onNewTab,
}: {
  tabs: Tab[];
  activeId: string;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNewTab: () => void;
}) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    void windowChrome.isMaximized().then(setMaximized);
    return windowChrome.onMaximizeChange(setMaximized);
  }, []);

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="tabstrip" data-tauri-drag-region>
        {tabs.map((tab) => {
          const label = locationLabel(tab.location);
          return (
            <div
              key={tab.id}
              className="tab"
              data-active={tab.id === activeId ? "true" : "false"}
              onPointerDown={(event) => {
                if (event.button === 1) {
                  event.preventDefault();
                  onClose(tab.id);
                  return;
                }
                if (event.button === 0) onSelect(tab.id);
              }}
            >
              <span className="tab-glyph">
                <PlaceGlyph icon={locationGlyph(tab.location)} size={14} />
              </span>
              <button
                type="button"
                className="tab-label"
                title={tab.location.path || label}
                onClick={() => onSelect(tab.id)}
              >
                {label}
              </button>
              <button
                type="button"
                className="tab-close"
                aria-label={`Close ${label}`}
                onClick={(event) => {
                  event.stopPropagation();
                  onClose(tab.id);
                }}
              >
                <Icon name="close" size={9} strokeWidth={1.6} />
              </button>
            </div>
          );
        })}
        <button
          type="button"
          className="tab-new"
          aria-label="New tab"
          title="New tab (Ctrl+T)"
          onClick={onNewTab}
        >
          <Icon name="plus" size={14} />
        </button>
      </div>

      <div className="window-controls">
        <button
          type="button"
          className="caption-btn"
          data-kind="minimize"
          aria-label="Minimize"
          onClick={() => windowChrome.minimize()}
        >
          <Icon name="minimize" size={10} strokeWidth={1} />
        </button>
        <button
          type="button"
          className="caption-btn"
          data-kind="maximize"
          aria-label={maximized ? "Restore" : "Maximize"}
          onClick={() => {
            void windowChrome.toggleMaximize().then(setMaximized);
          }}
        >
          <Icon
            name={maximized ? "restore" : "maximize"}
            size={10}
            strokeWidth={1}
          />
        </button>
        <button
          type="button"
          className="caption-btn"
          data-kind="close"
          aria-label="Close"
          onClick={() => windowChrome.close()}
        >
          <Icon name="close" size={10} strokeWidth={1} />
        </button>
      </div>
    </header>
  );
}

const EDGES: { className: string; edge: ResizeEdge }[] = [
  { className: "grip-n", edge: "North" },
  { className: "grip-s", edge: "South" },
  { className: "grip-e", edge: "East" },
  { className: "grip-w", edge: "West" },
  { className: "grip-ne", edge: "NorthEast" },
  { className: "grip-nw", edge: "NorthWest" },
  { className: "grip-se", edge: "SouthEast" },
  { className: "grip-sw", edge: "SouthWest" },
];

/** Invisible hit areas on the perimeter. Without these an undecorated window
 *  cannot be resized by dragging its edges. */
export function ResizeEdges() {
  return (
    <>
      {EDGES.map(({ className, edge }) => (
        <div
          key={className}
          className={`grip ${className}`}
          onPointerDown={(event) => {
            event.preventDefault();
            windowChrome.startResize(edge);
          }}
        />
      ))}
    </>
  );
}
