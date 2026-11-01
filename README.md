# Lyre

A fast, cross-platform file manager in the Windows 11 Explorer design language.

Lyre reads a folder through a parallel Rust scanner and paints the first rows
while the rest are still arriving, so a folder of 20,000 entries is on screen in
under 20 ms. Windows, macOS and Linux from one codebase: Rust core, Tauri v2
shell, React 19 interface.

![Linux](https://img.shields.io/badge/Linux-tested-2ea44f?style=flat-square)
![Windows](https://img.shields.io/badge/Windows-tested%20in%20a%20VM-2ea44f?style=flat-square)
![macOS](https://img.shields.io/badge/macOS-untested-d29922?style=flat-square)

> [!WARNING]
> **macOS is untested.** The macOS branches are written, but no one has run this
> window on a Mac. Treat it as experimental: it cannot be the default folder
> handler there (Finder is the shell) and the "open with" list comes back empty.
> Linux and Windows are the platforms this is used on.

[Install](#install) • [Benchmarks](#benchmarks) • [Build and run](#build-and-run) • [Keyboard](#keyboard) • [Status](#status) • [How it stays fast](docs/performance.md) • [Platform notes](docs/platforms.md)

## Features

- **Folders open now.** The directory scanner streams batches to the interface as
  it reads, so the first screen paints before the rest of the folder is known,
  and the list is virtualised: 50,000 entries cost the same DOM as 30.
- **Search that answers while you type.** The folder on screen is answered from
  memory in single-digit milliseconds, then a breadth-first walk covers
  everything below it and streams deeper matches in behind. Folders lead the
  results, and accents do not have to be typed: `tari` finds `Țări`.
- **Explorer's shell, everywhere.** Tabs, command bar, address bar with a folding
  breadcrumb, navigation rail, details and large-icon views, sortable columns,
  group by date, compact density.
- **Real file operations.** Copy, cut and paste with a conflict prompt, delete to
  the trash, rename in place, new folder, properties, and open with any
  registered application.
- **Preview pane.** Image, video and PDF thumbnails, the first lines of a text
  file, and the details of whatever is focused. Pixels are decoded once, at a
  size worth showing, and cached by file identity; the webview never sees a 50
  megapixel original. [How it works](docs/behaviour.md#preview-pane)
- **Drag and drop that knows its own mind.** Onto a folder, a breadcrumb, a tab
  or the rail: a move within one volume, a copy across volumes, Shift inverts it,
  and the badge says which. Dragging past the window edge becomes a real
  operating-system drag, so files can be dropped into other applications.
  [How it works](docs/behaviour.md#drag-and-drop)
- **Volumes, pins and safe eject.** This PC lists devices rather than mounts, a
  removable volume can be ejected without unplugging it, and folders can be
  pinned to Quick access. A stick plugged in while the window is open appears on
  its own.
- **Hidden and protected items.** Two switches, because Explorer has two: hidden
  entries are drawn dimmed so you can tell what they are, and the system's own
  entries stay out of the listing until asked for.
- **Light, dark or the system's choice**, Explorer's keyboard shortcuts,
  type-ahead, and rubber-band selection.

## Install

Releases carry an installer for every platform, built by
[the release workflow](.github/workflows/release.yml) on GitHub's runners:

| Platform | File |
| --- | --- |
| Windows | `Lyre_<version>_x64-setup.exe` for one user, or the `.msi` for all |
| macOS | `Lyre_<version>_universal.dmg`, one app for Apple silicon and Intel |
| Linux | `AppImage`, `deb`, `rpm` |

Arch users can build from the PKGBUILDs in
[packaging/aur](packaging/aur) until the packages are in the AUR, after which it
is `paru -S lyre-bin` (the prebuilt release) or `paru -S lyre` (from source). A
Flatpak manifest lives in [flatpak](flatpak/app.lyre.files.yml) but is not
published yet.

Nothing is signed, so Windows SmartScreen and macOS Gatekeeper each ask once.
Per-platform detail, the AUR submission steps and the release procedure are in
[docs/install.md](docs/install.md).

## Benchmarks

> [!NOTE]
> These are development-build numbers on one machine (`opt-level = 1`): a Ryzen
> 5 3600, NVMe, btrfs for real trees and tmpfs for the synthetic one, warm page
> cache. A release build is faster. What matters here is the order of magnitude,
> and that every table names the command that reproduces it.

### Listing a folder

| Directory | Entries | Scanner | Including the first batch |
| --- | --- | --- | --- |
| 20,000 files, btrfs | 20,000 | 16 ms | 18 ms |
| 20,000 files, tmpfs | 20,000 | 13 ms | 15 ms |

```bash
cd src-tauri && cargo test --lib bench_twenty_thousand -- --ignored --nocapture
```

The second column is what the window gets first, because the scan streams: rows
paint before the total is known. The status bar shows both numbers as they
happen.

### Searching

A recursive search, warm, one query at a time:

| Tree | Entries examined | `libc` (17,739 hits) | `zzzzzzz` (0 hits) | `cargo` (4,317 hits) |
| --- | --- | --- | --- | --- |
| `/usr` | 639,755 | 110 ms | 98 ms | 115 ms |
| `/home` | 929,929 | 226 ms | 226 ms | 213 ms |

```bash
cd src-tauri && cargo test --lib bench_real_tree -- --ignored --nocapture
```

The cost is the walk, not the number of matches: seventeen thousand hits and no
hits cost the same. The folder on screen is answered before the walk starts,
from the index the last scan left behind.

The same search restricted to one folder of 30,000 files, which is what typing
in a large directory feels like:

| Query | Matches | Time |
| --- | --- | --- |
| `re` | 30,000 | 2.9 ms |
| `report` | 30,000 | 2.7 ms |
| `doc7` | 10,317 | 2.5 ms |
| `dcmnt` (fuzzy) | 30,000 | 4.8 ms |

```bash
cd src-tauri && cargo test --lib bench_packed_folder -- --ignored --nocapture
```

### Memory

- The search index keeps 78 bytes per row instead of 175: it stores what it needs
  to match and rank, and rebuilds a display row only for entries that match. A
  test asserts the rebuilt row is identical to the original.
- An idle window is about 300 MB PSS, of which roughly 170 MB is the web
  process, 90 MB the application and 30 MB the network process: those are
  WebKitGTK's own costs. The one lever that moved them, at 45 MB per window, is
  **More → Reduce graphics memory**.

### Against Windows File Explorer

Not measured yet. Explorer has to be timed on Windows, and the number worth
comparing is the one a person feels: from double-clicking a folder to the first
row being on screen, on the same folder, on the same disk. Until that is
recorded here, this README claims nothing about it.

## Build and run

Needs Rust, Bun, and Tauri's prerequisites for the platform. On Linux that is the
development packages for `webkit2gtk-4.1`, `gtk3` and `libsoup3`.

> [!TIP]
> Two commands get you a window: `bun install`, then `bun dev`.

```bash
bun install
bun dev        # compiles the Rust core and opens the window on your filesystem
bun buildit    # builds for whichever OS is running it
```

`bun dev` starts Vite on port 1420, or on the next free port if that one is taken,
and tells Tauri the same number, so the window and the dev server cannot
disagree. `bun buildit` detects the platform: it picks the installer formats the
machine can actually produce, and on Windows builds in a separate copy of the
tree so the two operating systems never share object files. Flags, port
handling and the Windows build steps are in [docs/building.md](docs/building.md).

The interface also runs in a plain browser against a synthetic filesystem, which
is how the layout is developed without rebuilding Rust:

```bash
bun run dev:web
```

## Keyboard

| Keys | Action |
| --- | --- |
| `Alt+Left` / `Alt+Right` / `Alt+Up` | Back, forward, up |
| `Backspace` | Up one level |
| `F5` | Refresh |
| `Ctrl+T` / `Ctrl+W` | New tab, close tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next, previous tab |
| `Ctrl+1` … `Ctrl+9` | Go to tab |
| `Ctrl+B` | Navigation pane |
| `Alt+D`, `Ctrl+L`, `F4` | Edit the address bar |
| `F3`, `Ctrl+F`, `Ctrl+E` | Focus search |
| `F11` | Full screen |
| `Enter` | Open |
| `F2` | Rename |
| `Ctrl+Shift+N` | New folder |
| `Alt+Enter` | Properties |
| `Ctrl+Shift+C` | Copy path |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy, cut, paste |
| `Delete` | Move to the trash |
| `Shift+F10` | Context menu at the selection |
| `Ctrl+Shift+2` / `Ctrl+Shift+6` | Large icons, details |
| `Ctrl+A`, `Shift+arrows`, `Ctrl+click`, `Shift+click` | Selection |
| `Home` / `End` / `PageUp` / `PageDown` | Move through the list |
| `Space` | Toggle the focused item |
| `Escape` | Clear selection, close a menu |
| typing | Jump to a name |

The two side buttons on a mouse are back and forward, walking the same per-tab
history as `Alt+Left` and `Alt+Right`.

`Ctrl+Z`, `Ctrl+Y`, `Ctrl+N`, `Ctrl+D`, `Alt+Z`, `Alt+P` and `F6` do nothing on
purpose: there is no undo history, second window or pin gesture behind them yet,
and a shortcut that silently does the wrong thing is worse than one that is
absent.

## Status

Everything under [Features](#features) works and is covered by the checks in
[Verification](#verification). Not implemented, and said so in the interface
rather than failing silently: sharing, and searching inside file contents rather
than by name.

Platform differences are real and are described in
[docs/platforms.md](docs/platforms.md):

- Being the default folder handler exists on Linux (a `.desktop` entry claiming
  `inode/directory`) and on Windows (a per-user override of the shell's folder
  verb, no administrator rights). macOS has no route: Finder is the shell.
- Open with hands off to the system picker on Windows, and reads `.desktop`
  entries on Linux, where the default is set through `xdg-mime`.
- On Linux the webview itself tops out near 60 fps whatever the display does. The
  measurements are in [docs/performance.md](docs/performance.md).

## Verification

```bash
bun run typecheck
bun run build
cd src-tauri && cargo test
cd src-tauri && cargo clippy
```

## Project layout

```
src/
  App.tsx               window state, tabs, history, shortcuts
  components/           chrome, navigation, list, menus, icons
  lib/ipc.ts            the only place that talks to Rust
  lib/useDirectory.ts   streaming scan into a sorted, render-ready array
  lib/sort.ts           one comparator, ICU collation
  lib/mock.ts           synthetic filesystem for browser development
  styles/               tokens, base, chrome, content
src-tauri/
  src/scan.rs           the scanner
  src/search.rs         matching, ranking, the walk
  src/places.rs         known folders, volumes, breadcrumbs
  src/lib.rs            commands, scan registry
docs/                   performance, platforms, building, behaviour
PRODUCT.md              who this is for and what it is
DESIGN.md               the visual system, as built
```

## License

No license has been chosen yet. Third-party marks are absent: the design
language follows Windows 11 Explorer, but no Microsoft asset is redistributed
here. Icons are drawn in this repository.
