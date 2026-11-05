/**
 * Regression checks for the content-area drop zone.
 *
 * Run with `bun run test:dropzone`. Plain assertions, deliberately: the project
 * has no JavaScript test runner, and adding one is not worth a new dependency
 * for four checks. This file is not imported by the app, so Vite never bundles
 * it and `tsc --noEmit` still type-checks it.
 *
 * The bug these guard: dragging files from an archiver onto a folder *row*
 * worked, because the row has `data-drop="folder"`, but opening that folder and
 * dropping on its background did nothing. An empty folder renders a status
 * panel instead of the scroller that carried `data-drop="background"`, and the
 * column header and status bar are siblings of that scroller. None of them had
 * a drop-zone ancestor, so `targetAt()` returned null and the drop was lost.
 */
import {
  backgroundDropZone,
  zoneTarget,
  type BackgroundDropZone,
} from "./dragAndDrop";

function check(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

function same(actual: BackgroundDropZone, expected: BackgroundDropZone): boolean {
  return (
    actual["data-drop"] === expected["data-drop"] &&
    actual["data-path"] === expected["data-path"]
  );
}

/** JSX `data-*` attributes arrive in the DOM as camel-less dataset keys. */
function asDataset(zone: BackgroundDropZone): DOMStringMap {
  return { drop: zone["data-drop"], path: zone["data-path"] };
}

const folder = backgroundDropZone("C:/Users/me/Documents", true);
check(
  same(folder, {
    "data-drop": "background",
    "data-path": "C:/Users/me/Documents",
  }),
  "an open folder's whole content area must be a drop target",
);

const home = backgroundDropZone("Home", false);
check(same(home, {}), "Home has no directory, so it must not accept a paste");

const thisPc = backgroundDropZone("This PC", false);
check(same(thisPc, {}), "This PC has no directory either");

const noPath = backgroundDropZone("", true);
check(same(noPath, {}), "a location with no path must not be a destination");

// The attributes above are only useful if the hit test reads them. A point on
// the empty-folder panel now resolves to the content area, and that zone must
// produce a background target rather than being discarded.
const resolved = zoneTarget({ dataset: asDataset(folder) });
check(
  resolved?.kind === "background" && resolved.path === "C:/Users/me/Documents",
  "the content area must resolve to the folder on screen",
);
check(
  zoneTarget({ dataset: asDataset(home) }) === null,
  "Home's empty space must not resolve to a paste destination",
);

console.log("backgroundDropZone: 6 checks passed");
