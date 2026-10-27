import { useState } from "react";
import type { Location, Place } from "@/types";
import { folder, gallery as galleryLocation, home, thisPc } from "@/lib/location";
import { Icon } from "./Icons";
import { PlaceGlyph } from "./FileGlyph";

const MIN_WIDTH = 140;
const MAX_WIDTH = 360;

/**
 * Explorer's navigation pane: a short list of starting points, the pinned
 * known folders, then the expandable machines. Selecting an item is a fill,
 * not an accent bar.
 */
export function NavRail({
  places,
  location,
  pictures,
  onNavigate,
  width,
  onWidth,
  onUnavailable,
}: {
  places: Place[];
  location: Location;
  pictures: string;
  onNavigate: (location: Location) => void;
  width: number;
  onWidth: (next: number) => void;
  onUnavailable: (message: string) => void;
}) {
  const [thisPcOpen, setThisPcOpen] = useState(false);
  const [networkOpen, setNetworkOpen] = useState(false);

  const folders = places.filter((place) => place.kind === "folder");
  const volumes = places.filter((place) => place.kind !== "folder");

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = width;
    const onMove = (move: PointerEvent) => {
      onWidth(
        Math.min(
          MAX_WIDTH,
          Math.max(MIN_WIDTH, startWidth + move.clientX - startX),
        ),
      );
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
          <NavRow
            glyph={<PlaceGlyph icon="home" size={16} />}
            label="Home"
            active={location.kind === "home"}
            onClick={() => onNavigate(home(folders[0]?.path ?? pictures))}
          />
          <NavRow
            glyph={<PlaceGlyph icon="image" size={16} />}
            label="Gallery"
            active={location.kind === "gallery"}
            onClick={() => onNavigate(galleryLocation(pictures))}
          />
        </div>

        <div className="nav-sep" />

        <div className="nav-section">
          {folders.slice(1).map((place) => (
            <NavRow
              key={place.path}
              glyph={<PlaceGlyph icon={place.icon} size={16} />}
              label={place.name}
              active={
                location.kind === "folder" && location.path === place.path
              }
              pinned
              onClick={() => onNavigate(folder(place.path))}
            />
          ))}
        </div>

        <div className="nav-sep" />

        <div className="nav-section">
          <NavRow
            glyph={<PlaceGlyph icon="pc" size={16} />}
            label="This PC"
            active={location.kind === "thisPc"}
            twisty={thisPcOpen}
            onTwisty={() => setThisPcOpen((value) => !value)}
            onClick={() => onNavigate(thisPc())}
          />
          {thisPcOpen
            ? volumes.map((volume) => (
                <NavRow
                  key={volume.path}
                  child
                  glyph={<PlaceGlyph icon="drive" size={16} />}
                  label={volume.name}
                  active={
                    location.kind === "folder" && location.path === volume.path
                  }
                  onClick={() => onNavigate(folder(volume.path))}
                />
              ))
            : null}
          <NavRow
            glyph={<PlaceGlyph icon="network" size={16} />}
            label="Network"
            active={false}
            twisty={networkOpen}
            onTwisty={() => setNetworkOpen((value) => !value)}
            onClick={() =>
              onUnavailable("Network locations arrive with a network provider.")
            }
          />
        </div>
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

function NavRow({
  glyph,
  label,
  active,
  pinned = false,
  child = false,
  twisty,
  onTwisty,
  onClick,
}: {
  glyph: React.ReactNode;
  label: string;
  active: boolean;
  pinned?: boolean;
  child?: boolean;
  twisty?: boolean;
  onTwisty?: () => void;
  onClick: () => void;
}) {
  return (
    <div
      className={`nav-item${child ? " nav-child" : ""}`}
      data-active={active ? "true" : "false"}
      data-open={twisty ? "true" : "false"}
    >
      {onTwisty ? (
        <button
          type="button"
          className="nav-item-glyph"
          aria-label={twisty ? `Collapse ${label}` : `Expand ${label}`}
          onClick={onTwisty}
          style={{ background: "none", border: "none", padding: 0 }}
        >
          <Icon
            name="chevronRight"
            size={12}
            strokeWidth={1.6}
            className="nav-item-chevron"
          />
        </button>
      ) : null}
      <button
        type="button"
        onClick={onClick}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          flex: 1,
          minWidth: 0,
          height: "100%",
          color: "inherit",
          textAlign: "left",
        }}
      >
        {!onTwisty ? <span className="nav-item-glyph">{glyph}</span> : null}
        {onTwisty ? glyph : null}
        <span className="nav-item-label">{label}</span>
      </button>
      {pinned ? (
        <span className="nav-pin" title="Pinned to Quick access">
          <Icon name="pin" size={12} />
        </span>
      ) : null}
    </div>
  );
}
