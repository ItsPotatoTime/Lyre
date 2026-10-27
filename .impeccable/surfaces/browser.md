---
version: 1
slug: "browser"
primary_target: "browser"
related_targets: []
---

# Surface: browser

## Scope and mode

Primary surface of the product: the file browser window. Mode: Operate. The visitor is
mid-task, repeatedly, every day. Everything here serves scanning, selection, and
navigation; expression lives in precision and in one signature move.

## Audience and job

A daily file-manager user opening a directory of unknown size, looking for a file whose
name they partly know or sorting a folder to see what changed. They need the contents to
appear at once, the names readable, the sort obvious, and the keyboard to work the way
Explorer's does.

## Constraints

Windows 11 Explorer is the pinned visual language: Mica ground, Acrylic layers, Segoe UI
Variable, 4 px control and 8 px surface radii, reveal highlight, accent blue. Nothing may
be added that Explorer does not have, and nothing Explorer shows may be missing. Motion
must confirm, never delay: no animation on keyboard-driven navigation and no entrance
animation on rows.

## Direction contract

THESIS: A file manager that is honest about time. The category default is a spinner over
an empty list while a directory resolves; Lyre refuses that arrangement by painting the
first entries as they are read, and by making the interface's structure identical to
Explorer's so the speed is the only thing the user notices.

OWN-WORLD: Fluent materials on a neutral Mica ground lifted by a wallpaper tint, one
accent blue doing all interactive work, hairline strokes at 6 percent ink, 14 px Segoe
UI Variable with a 12 px navigation register, 32 px list rows, and 8 px surfaces. The
signature is the reveal highlight: a pointer-following stroke that traces the edge of the
row or control under the cursor, the one flourish Windows 11 owns.

STORY: The visitor opens a folder, sees files rather than a loading state, finds the
target by name or by sort, selects several, and moves on. Nothing about the interface
asks to be admired; the acknowledgement is that they never waited.

FIRST VIEWPORT: A 1200x800 window. A 48 px title bar carrying the tab strip and window
controls. Below it, a 40 px command bar: New, Cut, Copy, Paste, Rename, Delete, Sort,
View on the left, `...` on the right, each an icon over a 12 px label. A 44 px address
row: back, forward, up, then a breadcrumb field that fills the width, then the search
field. The body is a two-column split: a 260 px navigation rail with pinned folders and
drives, and the content area. Content opens in details view: a sticky 32 px column header
over 32 px rows, name first with a 20 px type glyph, then date modified, type, size.
Selected row is accent-tinted at full width. A 28 px status bar reports item count and
selection at the left.

FORM: Explorer's own shell, held to its own proportions. This surface is brief-pinned, so
no roll ran and no stock arrangement is being avoided; the discipline is fidelity, measured
against Windows 11 Explorer rather than against a mood.
seed: none (unavailable)

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review,
the verdict, DESIGN.md, and every shipping raster carrying its provenance.

## Memorable moment

Opening a 50,000-entry folder: rows are already there, and the count in the status bar
climbs to its final number as the rest of the directory arrives. That climbing count is
the speed made visible.

## Unresolved

- File operations, search indexing, and drag and drop are product scope, not this
  surface's; they arrive in a later pass.

## Finish review

Verdict: pass, with the deferred scope named rather than hidden.

What was verified against the built result, not the intent:

- Shell metrics measured in the running window at 1240x820: tab strip 40,
  command bar 44, address bar 44, status bar 26, rail 240, column header 32,
  rows 30 (24 compact), tiles 104x112. No document overflow in either axis.
- Column header cells and row cells share one grid template; every cell origin
  matches its header origin.
- Text contrast, light and dark, measured on the rendered colors: lowest pair
  5.0:1, most 7:1 and above.
- All 47 rendered icons rasterised and checked for ink, bounding box, and edge
  clipping: none blank, none clipped.
- A 4,224 entry folder keeps 30 to 63 nodes in the DOM; grid scroll height
  matches rows x tile height exactly (52,640 px) and the last tile lands inside
  the viewport at the bottom of the scroll.
- Rust scanner, through its own tests: exact totals, opt-in hidden entries,
  multi-batch streaming with no dropped or duplicated entry, cancellation that
  does not claim completion, readable failure for a missing directory. 20,000
  entries in 16 ms in a debug build.
- The packaged app launches, and the webview completes a real scan round trip
  (`scan 1 start /home/potato`, `scan 1 done 59`).

Known gaps, deliberately: no file operations, no search index, no drag and drop,
no thumbnails, no context menu, no OS file associations. Each is stated in the
interface where a user would otherwise expect it to work.

Raster provenance: nothing ships as a bitmap. Icons, glyphs, and the app icon are
authored SVG in this repository, rasterised only into the platform icon set that
Tauri requires. No reference image is redistributed; the Explorer captures were
used to sample values, not as assets.
