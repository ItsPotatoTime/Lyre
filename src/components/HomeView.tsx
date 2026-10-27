import { useState } from "react";
import type { Place } from "@/types";
import { Icon } from "./Icons";
import { PlaceGlyph } from "./FileGlyph";

/** Explorer's empty-favourites illustration: two folders and a star. */
function FavoritesArt() {
  return (
    <svg width="148" height="86" viewBox="0 0 148 86" aria-hidden="true">
      <path
        d="M74 12h18l6 8h36a4 4 0 0 1 4 4v38a4 4 0 0 1-4 4H58a4 4 0 0 1-4-4V16a4 4 0 0 1 4-4z"
        fill="currentColor"
        opacity=".38"
      />
      <path
        d="M18 30h22l7 9h45a5 5 0 0 1 5 5v26a5 5 0 0 1-5 5H18a5 5 0 0 1-5-5V35a5 5 0 0 1 5-5z"
        fill="currentColor"
      />
      <path
        d="m61 44 3.6 7.4 8.2 1.2-5.9 5.7 1.4 8.1-7.3-3.9-7.3 3.9 1.4-8.1-5.9-5.7 8.2-1.2z"
        fill="#ffffff"
      />
    </svg>
  );
}

/**
 * Explorer's Home page. It is not a directory listing: it is the six places
 * and the favourites shelf below them.
 */
export function HomeView({
  places,
  onNavigate,
  onUnavailable,
}: {
  places: Place[];
  onNavigate: (path: string) => void;
  onUnavailable: (message: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const folders = places.filter((place) => place.kind === "folder");

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
        Quick access
      </button>

      {open ? (
        <div className="places">
          {folders.map((place) => (
            <button
              key={place.path}
              type="button"
              className="place"
              onClick={() => onNavigate(place.path)}
            >
              <span className="place-glyph">
                <PlaceGlyph icon={place.icon} size={32} />
              </span>
              <span className="place-text">
                <span className="place-name">{place.name}</span>
              </span>
            </button>
          ))}
        </div>
      ) : null}

      <button
        type="button"
        className="pill"
        title="Add a file to Favorites"
        onClick={() =>
          onUnavailable(
            "Favourites arrive with file operations: select a file, then add it.",
          )
        }
      >
        <Icon name="star" size={14} />
        Favorites
      </button>

      <div className="favorites-empty">
        <span className="favorites-art">
          <FavoritesArt />
        </span>
        <span className="favorites-title">Your favorites will appear here</span>
        <span className="favorites-body">
          Select a file, then select Add to Favorites to access it here.
        </span>
      </div>
    </div>
  );
}
