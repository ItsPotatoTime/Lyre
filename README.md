# Lyre

A cross-platform file manager that looks like Windows 11 File Explorer and opens
directories faster than the one that ships with your OS.

Rust core, Tauri v2 shell, React 19 and TypeScript interface. Runs on Windows,
macOS, and Linux from one codebase.

## Status

This is the first pass: a real, working browser. It reads the actual filesystem
through the Rust core and paints whatever it finds.

Working now:

- Directory listing with a parallel, streamed Rust scanner
- Details and large-icon views, both virtualised
- Tabs, navigation history, breadcrumb with folding, editable address field
- Navigation rail with known folders and mounted volumes, resizable
- Sortable columns, name/date/type/size, ascending and descending
- Selection (click, ctrl-click, shift-range, keyboard), type-ahead, Home/End,
  PageUp/PageDown, Enter, Backspace
- In-folder filter from the search field
- Explorer and compact density, hidden items toggle
- Light, dark, and system themes
- Window chrome: tabs in a custom title bar, caption buttons, edge resizing
- Opening a file in whatever the desktop associates with its type

Not implemented yet, and honest about it in the interface: copy, cut, paste,
rename, delete, share, new folder, drag and drop, search indexing, thumbnails,
context menus and the preview pane.

## Running it

```bash
bun install
bun dev            # builds the Rust core and opens the app, with hot reload
```

`bun dev` is the whole thing: it starts Vite on port 1420, compiles the Rust
crate, and opens the window against your real filesystem.

```bash
bun run app:build  # production bundles for the host OS
```

Requires Rust, Bun, and the Tauri prerequisites for your platform. On Linux
that means `webkit2gtk-4.1`, `gtk3`, and `libsoup3` development packages.

The interface also runs in a plain browser, which is how the layout is developed
without rebuilding Rust:

```bash
bun run dev:web    # http://127.0.0.1:1420, synthetic filesystem
```

In that mode `src/lib/mock.ts` supplies a deterministic filesystem, including a
4,200 entry photo library for exercising the virtualiser.

## How it is fast

The scanner is in `src-tauri/src/scan.rs`. Three decisions carry it:

1. **Enumerate names, then stat in parallel.** `read_dir` gives names and file
   types without a syscall per entry. The names are cut into 512-entry chunks
   and stat'ed by a fixed pool of workers, two per core.
2. **Stream, do not wait.** Each finished chunk goes to the webview over a Tauri
   IPC channel the moment it exists. The first rows paint while the rest of the
   directory is still being read, and the status bar count climbs.
3. **The interface owns the comparator.** Rows arrive unsorted and are merged
   into an already-sorted array one batch at a time, one linear pass per batch.
   Sorting happens in Rust only where it is free (nothing per entry), and the
   only full sort is when the user clicks a column.

Measured on this machine, in a debug build, on a synthetic directory of 20,000
files: **16 ms** from call to final total. `cargo test --lib bench -- --ignored
--nocapture` reproduces it.

Rendering is windowed: a 4,200 entry folder keeps about 30 rows in the DOM, and
the row arrays live in refs rather than React state so a fast scan does not push
50,000 element array copies through the reconciler.

## Layout

```
src/
  App.tsx               window state, tabs, history, shortcuts
  components/           chrome, navigation, list, menus, icons
  lib/scan (ipc.ts)     the only place that talks to Rust
  lib/useDirectory.ts   streaming scan into a sorted, render-ready array
  lib/sort.ts           one comparator, ICU collation
  lib/mock.ts           synthetic filesystem for browser development
  styles/               tokens, base, chrome, content
src-tauri/
  src/scan.rs           the scanner
  src/places.rs         known folders, volumes, breadcrumbs
  src/lib.rs            commands, scan registry
```

## Verification

```bash
bun run typecheck
bun run build
cd src-tauri && cargo test
cd src-tauri && cargo clippy
```

The Rust tests cover the scanner's contract: totals match what was sent, hidden
entries are opt-in, a large directory streams in more than one batch without
dropping or duplicating an entry, cancellation does not claim completion, and a
missing directory reports something a human can read.

## Licensing

No license has been chosen yet. Third-party marks are absent: the design
language follows Windows 11 Explorer, but no Microsoft asset is redistributed
here. Icons are drawn in this repository.
