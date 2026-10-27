import { useEffect, useState } from "react";
import type { Tab } from "@/types";
import { windowChrome, type ResizeEdge } from "@/lib/window";
import { Icon } from "./Icons";

function tabName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

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
        {tabs.map((tab) => (
          <div
            key={tab.id}
            className="tab"
            data-active={tab.id === activeId ? "true" : "false"}
            onPointerDown={(event) => {
              // Middle click closes a tab, as in Explorer.
              if (event.button === 1) {
                event.preventDefault();
                onClose(tab.id);
                return;
              }
              if (event.button === 0) onSelect(tab.id);
            }}
          >
            <Icon name="folder" size={14} />
            <button
              type="button"
              className="tab-label"
              title={tab.path}
              onClick={() => onSelect(tab.id)}
            >
              {tabName(tab.path)}
            </button>
            <button
              type="button"
              className="tab-close"
              aria-label={`Close ${tabName(tab.path)}`}
              onClick={(event) => {
                event.stopPropagation();
                onClose(tab.id);
              }}
            >
              <Icon name="close" size={10} />
            </button>
          </div>
        ))}
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
          <Icon name="minimize" size={12} strokeWidth={1} />
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
            size={12}
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
          <Icon name="close" size={12} strokeWidth={1} />
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

/** Invisible hit areas on the window perimeter. Without these an undecorated
 *  window cannot be resized by dragging its edges. */
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
