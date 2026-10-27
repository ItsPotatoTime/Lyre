import type { Location, LocationKind, PathInfo } from "@/types";
import { folderLabel, LANDING_LABEL } from "./platform";

/** Explorer's Home page is a place, not a directory, so it gets its own kind. */
export const home = (path: string): Location => ({ kind: "home", path });
export const thisPc = (): Location => ({ kind: "thisPc", path: "" });
export const folder = (path: string): Location => ({ kind: "folder", path });
export const gallery = (path: string): Location => ({ kind: "gallery", path });

/** True for locations that have no directory to read. */
export function isVirtual(location: Location): boolean {
  return location.kind === "thisPc" || location.kind === "home";
}

export function locationLabel(location: Location): string {
  switch (location.kind) {
    case "home":
      return LANDING_LABEL;
    case "thisPc":
      return "This PC";
    case "gallery":
      return "Gallery";
    case "folder": {
      const parts = location.path.split(/[\\/]/).filter(Boolean);
      const last = parts[parts.length - 1] ?? location.path;
      return folderLabel(location.path, last);
    }
  }
}

export function locationGlyph(location: Location): string {
  switch (location.kind) {
    case "home":
      return "home";
    case "thisPc":
      return "pc";
    case "gallery":
      return "image";
    case "folder":
      return "folder";
  }
}

export function sameLocation(a: Location, b: Location): boolean {
  return a.kind === b.kind && a.path === b.path;
}

/** The location above this one, if there is one. */
export function parentOf(
  location: Location,
  info: PathInfo | null,
): Location | null {
  if (location.kind === "home") return null;
  if (location.kind === "thisPc") return null;
  if (location.kind === "gallery") {
    return folder(info?.parent ?? location.path);
  }
  if (!info?.parent) return null;
  // A filesystem root's parent is This PC, which is where drives live.
  if (info.parent === location.path) return thisPc();
  return folder(info.parent);
}

export function kindLabel(kind: LocationKind): string {
  switch (kind) {
    case "home":
      return LANDING_LABEL;
    case "thisPc":
      return "This PC";
    case "gallery":
      return "Gallery";
    case "folder":
      return "Folder";
  }
}
