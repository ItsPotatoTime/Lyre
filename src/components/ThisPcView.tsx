import { useState } from "react";
import type { Place } from "@/types";
import { Icon } from "./Icons";
import { PlaceGlyph } from "./FileGlyph";

function gigabytes(bytes: number): string {
  const value = bytes / 1_073_741_824;
  // A 32 MB EFI partition must not read as "0.0 GB free of 0.0 GB".
  if (value < 1) {
    const megabytes = Math.max(1, Math.round(bytes / 1_048_576));
    return `${megabytes} MB`;
  }
  if (value >= 100) return `${Math.round(value)} GB`;
  return `${value.toFixed(1)} GB`;
}

/**
 * Explorer's This PC page: volumes as cards with a capacity bar, not rows.
 * The bar is the accent blue (#0078D4 in the reference capture) on a track of
 * the foreground at low opacity.
 */
export function ThisPcView({
  places,
  selected,
  onSelect,
  onOpen,
}: {
  places: Place[];
  selected: string | null;
  onSelect: (path: string) => void;
  onOpen: (path: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const volumes = places.filter((place) => place.kind !== "folder");

  return (
    <div className="home">
      <button
        type="button"
        className="section-head"
        data-collapsed={open ? "false" : "true"}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon
          name="chevronDown"
          size={12}
          strokeWidth={1.6}
          className="section-chevron"
        />
        Devices and drives
      </button>

      {open ? (
        <div className="drives">
          {volumes.map((volume) => {
            const used = Math.max(0, volume.total - volume.free);
            const ratio = volume.total > 0 ? used / volume.total : 0;
            return (
              <button
                key={volume.path}
                type="button"
                className="drive-card"
                data-drop="folder"
                data-path={volume.path}
                data-selected={selected === volume.path ? "true" : "false"}
                onClick={() => onSelect(volume.path)}
                onDoubleClick={() => onOpen(volume.path)}
              >
                <span className="drive-glyph">
                  <PlaceGlyph icon="drive" size={32} />
                </span>
                <span className="drive-name" title={volume.path}>
                  {volume.name}
                </span>
                {volume.detail ? (
                  <span className="drive-device" title={volume.path}>
                    {volume.detail}
                  </span>
                ) : null}
                <span className="drive-bar">
                  <span
                    className="drive-bar-fill"
                    style={{ width: `${Math.round(ratio * 100)}%` }}
                  />
                </span>
                <span className="drive-detail">
                  {gigabytes(volume.free)} free of {gigabytes(volume.total)}
                </span>
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
