# Design

Lyre's visual world is Windows 11 File Explorer, pinned by the user. This file
records what the build actually does, so later work extends the system instead
of re-deciding it.

Values marked **measured** were sampled from real Explorer captures rather than
recalled: a light-theme tab strip and a dark-theme window, both read pixel by
pixel. Where a measurement and a remembered Fluent token disagreed, the
measurement won.

## Materials

Explorer is not one surface. It is three tones stacked, with no dividers between
them, which is the detail that makes a copy read as correct or not.

| Surface | Light | Dark | Used for |
| --- | --- | --- | --- |
| Chrome | `#f3f3f3` | `#202020` | Tab strip, command bar, address bar, nav rail, status bar |
| Content | `#ffffff` **measured** | `#272727` | The file list and its column header |
| Raised | `#ffffff` | `#2c2c2c` | Address field, search field, flyouts, menus |

The chrome carries the desktop's wallpaper tint on Windows. Lyre cannot sample
it on Linux or macOS, so it fakes the material with a 2 to 3 percent accent wash
(`--surface-chrome-tint`) laid over the chrome tone. Nothing else in the
interface is tinted.

The content area is pure white in light mode. Grey content with a white toolbar
is the common wrong answer; Explorer's list is the brightest surface in the
window.

## Type

Segoe UI Variable is the voice, with Selawik as the metric-compatible open
fallback for platforms that lack it, then the platform UI face.

| Register | Size / line | Weight | Used for |
| --- | --- | --- | --- |
| Caption | 12 / 16 | 400 | Command labels, column headers, breadcrumbs, tabs, status bar |
| Row | 13 / 18 | 400 | File names and file metadata |
| Body | 14 / 20 | 400 | Navigation items, menu items |
| Body strong | 14 / 20 | 600 | Selection counts |
| Subtitle | 20 / 28 | 600 | Reserved for dialogs |

Numerals in the Size and Date columns are `tabular-nums` so the columns form a
grid the eye can run down.

## Metrics

The chrome is 128 px tall in total: tab strip 40, command bar 44, address bar 44,
then a 1 px divider. Status bar 26. Navigation rail 240, resizable 180 to 420.

Details rows are 30 px with a 32 px sticky column header; compact view is 24 and
28. Grid tiles are 104x112, compact 88x96, icon 48.

Columns: name flexible with a 200 px floor, then 184, 150, and 104 px. Size is
right-aligned; everything else is left-aligned.

Radii: 4 px for controls, 8 px for surfaces, tabs and flyouts. Nothing else
rounds.

## Colour

One accent does all interactive work: `#005fb8` light, `#60cdff` dark (measured
in the dark capture). Everything else is neutral. Selection is the accent at 11
percent light and 18 percent dark, never a solid fill; the focused row adds a
1 px accent inset ring so keyboard position is always visible.

File-type colours are the one place with a palette, because Explorer's icons are
full colour and that colour is what makes a folder scannable: amber folders,
green images, violet video, pink audio, red PDF, blue documents, green
spreadsheets, orange presentations, indigo code.

Contrast: every text pairing measures at or above 5.0:1, most above 7:1.

## Iconography

One authored set, 20x20 grid, 1.5 px stroke, round joins, drawn as SVG in
`src/components/Icons.tsx`. No icon font, no emoji. File glyphs are the
exception: filled page silhouettes in the family colour with a white mark
knocked out, drawn in `FileGlyph.tsx`.

## Motion

Fluent's curves, and very little of them.

- Standard `cubic-bezier(0.8, 0, 0.2, 1)`, decelerate `cubic-bezier(0.1, 0.9,
  0.2, 1)`, durations 83, 167, 250, 333 ms.
- **Rows, tiles, and list hovers never animate.** The pointer crosses a list far
  too fast for a transition to read as anything but lag, and Explorer does not
  ease them either.
- Keyboard-driven actions never animate. Arrow navigation, type-ahead, Enter,
  and Backspace are instantaneous.
- What does animate: menus scale in from their trigger over 167 ms, the status
  bar notice fades up, the skeleton breathes. All under 300 ms, all ease-out.
- Every pressable control scales to 0.97 on press, 0.92 for icon-only buttons.

`prefers-reduced-motion` collapses every transition and animation to 1 ms and
stops the looping ones.

## Signature

The reveal highlight: a soft radial wash that follows the pointer across command
bar buttons and navigation items. Delegated from one `pointermove` listener, and
the custom properties are written only on the hovered element, so the style
recalculation never touches its siblings. It is deliberately absent from list
rows, where the cost would scale with the directory.

## States

Every region has all five: rest, hover, pressed, selected, focused.

- Selection: accent tint plus, when focused, a 1 px accent ring.
- Empty folder: "This folder is empty", with the hidden-items hint.
- Empty search: "No items match your search", naming the cause rather than
  pretending the folder is empty.
- Loading: skeleton rows, but only after 140 ms, so a folder that reads in 6 ms
  never shows a loading state at all.
- Error: what failed and a retry action that re-reads the folder.
- Unimplemented operations say so in the status bar instead of failing silently.

## Browser surfaces

Themed rather than left at browser defaults: selection colour, caret, focus rings
(double ring on the surface colour, not a blue halo), scrollbars (thin, thumb at
28 percent, expanding on hover), and tabular numerals.

## What is deliberately not here

No gradients, no glass as decoration, no shadows except on flyouts, no cards, no
icon-plus-heading-plus-text blocks, no coloured left borders, no rounded chunk
standing in for content. Explorer has none of them and neither does this.

## Search

Results replace the listing in place: the same rows, the same columns, the same
selection behaviour. Three details carry it.

Matched characters are emphasised with weight rather than colour, so a match
stays legible on top of a selection fill or a hover wash. Runs of matched
characters collapse into one element, and offsets are character offsets, so a
name with an emoji highlights correctly.

Column headers stop being sortable while a search is live. Results are ordered
by relevance, and a sort arrow above them would describe an order that is not
being shown.

The status bar switches to what the user is actually asking: how many results,
how much of the tree was examined, and how long it took. A capped result set
says so ("showing the best") instead of implying the list is complete.
