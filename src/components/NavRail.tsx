import { useState } from "react";
import type { Place } from "@/types";
import { Icon } from "./Icons";
import { PlaceGlyph } from "./FileGlyph";

const MIN_WIDTH = 180;
const MAX_WIDTH = 420;

export function NavRail({
  places,
  activePath,
  onNavigate,
  width,
  onWidth,
}: {
  places: Place[];
  activePath: string;
  onNavigate: (path: string) => void;
  width: number;
  onWidth: (next: number) => void;
}) {
  const [foldersOpen, setFoldersOpen] = useState(true);
  const [drivesOpen, setDrivesOpen] = useState(true);

  const folders = places.filter((place) => place.kind === "folder");
  const drives = places.filter((place) => place.kind !== "folder");

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = width;

    const onMove = (move: PointerEvent) => {
      const next = Math.min(
        MAX_WIDTH,
        Math.max(MIN_WIDTH, startWidth + move.clientX - startX),
      );
      onWidth(next);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  return (
    <>
      <nav className="navrail" style={{ width }} aria-label="Folders">
        <div className="nav-section">
          <button
            type="button"
            className="nav-heading"
            data-collapsed={foldersOpen ? "false" : "true"}
            onClick={() => setFoldersOpen((v) => !v)}
            aria-expanded={foldersOpen}
          >
            <Icon
              name="chevronDown"
              size={12}
              strokeWidth={1.8}
              className="nav-heading-chevron"
            />
            Quick access
          </button>
          {foldersOpen
            ? folders.map((place) => (
                <NavItem
                  key={place.path}
                  place={place}
                  active={place.path === activePath}
                  onClick={() => onNavigate(place.path)}
                />
              ))
            : null}
        </div>

        {drives.length > 0 ? (
          <div className="nav-section">
            <button
              type="button"
              className="nav-heading"
              data-collapsed={drivesOpen ? "false" : "true"}
              onClick={() => setDrivesOpen((v) => !v)}
              aria-expanded={drivesOpen}
            >
              <Icon
                name="chevronDown"
                size={12}
                strokeWidth={1.8}
                className="nav-heading-chevron"
              />
              This PC
            </button>
            {drivesOpen
              ? drives.map((place) => (
                  <NavItem
                    key={place.path}
                    place={place}
                    active={place.path === activePath}
                    onClick={() => onNavigate(place.path)}
                  />
                ))
              : null}
          </div>
        ) : null}
      </nav>
      <div
        className="nav-splitter"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize navigation pane"
        onPointerDown={startResize}
      />
    </>
  );
}

function NavItem({
  place,
  active,
  onClick,
}: {
  place: Place;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="nav-item"
      data-active={active ? "true" : "false"}
      data-reveal="true"
      title={place.detail ? `${place.name}\n${place.detail}` : place.name}
      onClick={onClick}
    >
      <span className="nav-item-icon">
        <PlaceGlyph icon={place.icon} size={16} />
      </span>
      <span className="nav-item-label">{place.name}</span>
    </button>
  );
}
