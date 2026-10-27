# Lyre

A cross-platform file manager that looks like Windows 11 File Explorer and opens
directories faster than the one that ships with your OS.

Rust core, Tauri v2 shell, React 19 and TypeScript interface. Runs on Windows,
macOS, and Linux from one codebase.

## Status

This is the first pass: a real, working browser. It reads the actual filesystem
through the Rust core and paints whatever it finds.

Working now:

- Drag and drop: onto a folder, a breadcrumb, a tab, the navigation rail, or
  the folder itself. Within one volume it moves, across volumes it copies,
  Shift inverts whichever applies, and the pointer shows which will happen
- Drops from the operating system, copied into wherever they land
- Rubber-band selection: drag from empty space to select everything the
  rectangle touches, with Shift or Ctrl to add
- An offer, once at startup, to become the desktop's folder handler
- The landing page is "Quick access" on every platform, and "Home" always
  means the home directory
- Safe eject for removable volumes, and favorites and sidebar pins that can be
  added and removed from the context menus
- Volumes appearing or disappearing are noticed while the window is open, so a
  stick plugged in after startup shows up on its own
- This PC lists volumes, not mounts: one card per device, labelled by its
  volume label with the device shown underneath
- Right-click menus: Open, Open with, Cut, Copy, Copy path, Rename, Delete and
  Properties on an item; New folder, Paste, Sort by, View, Refresh and Select
  all on the background
- Copy, cut, paste and delete to the system trash
- Open a file with any application that declares support, and make it the default
- Creating a folder (Ctrl+Shift+N) and renaming in place (F2)
- Properties dialog (Alt+Enter)
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
- Search: instant results for the folder on screen, then matches from every
  subfolder streaming in behind them, ranked by relevance with the matched
  letters emphasised

Not implemented yet, and honest about it in the interface: share, drag and drop,
file contents search, thumbnails and the preview pane.

A paste that would land on an existing name asks first: keep both, which numbers
the incoming item, or replace. Replacing merges a folder rather than deleting it,
so files that exist only in the destination survive and only the files present in
both are written over. Delete moves to the freedesktop trash, which the desktop
can put back.

"Open with" is implemented differently per platform, because the platforms are
different. On Linux the list is read from the desktop's own `.desktop` entries
and the default is changed with `xdg-mime`. On Windows the list lives partly in
package manifests and is activated through COM, and the default is a
`UserChoice` key that Windows hash-validates, so the dialog hands off to
`SHOpenWithDialog`: the same picker Explorer uses for "Choose another app", which
knows every registered app and sets the default properly.

## Running it

```bash
bun install
bun dev            # builds the Rust core and opens the app, with hot reload
```

`bun dev` is the whole thing: it starts Vite on port 1420, compiles the Rust
crate, and opens the window against your real filesystem.

```bash
bun buildit        # build for whichever OS is running it
```

`bun buildit` detects the platform and does the right thing. On Linux and macOS
it installs dependencies and builds; it picks the installer formats the machine
can actually produce (`deb` needs `dpkg-deb`, `rpm` needs `rpmbuild`) and builds
just the executable, saying so, when neither is present.

Flags: `--debug`, `--clean`, `--bundles deb`, `--dry-run` (prints every command
without running it).

On Windows it does more, because a Windows build cannot share state with a Linux
one:

- The source is copied to `%USERPROFILE%\lyre-winbuild` and built there, so the
  two platforms never write object files into the same `target`.
- MSVC is activated with `vcvars64.bat` inside the same shell that runs cargo,
  and the presence of Git's GNU `link.exe` is checked for, since it shadows
  MSVC's and produces link errors that look like source bugs.
- These steps live in a generated `lyre-build.cmd`, so they can be read and
  re-run by hand instead of being buried in a quoted `cmd /c` string.
- Installers are copied back to `build/` on the shared disk.

If a build fails after an edit on the other OS, run `bun buildit --clean`: on a
shared filesystem the timestamps can be coarse enough that cargo reuses a stale
build script.

Requires Rust, Bun, and the Tauri prerequisites for your platform. On Linux
that means `webkit2gtk-4.1`, `gtk3`, and `libsoup3` development packages.

The interface also runs in a plain browser, which is how the layout is developed
without rebuilding Rust:

```bash
bun run dev:web    # http://127.0.0.1:1420, synthetic filesystem
```

In that mode `src/lib/mock.ts` supplies a deterministic filesystem, including a
4,200 entry photo library for exercising the virtualiser.

## Being the default file manager

On Linux, Lyre can register as the handler for `inode/directory`: a `.desktop`
entry in `~/.local/share/applications` declaring that type, set with
`xdg-mime default`. Folders then open here whether they come from the desktop or
from another application. The entry uses `%U`, so the folder arrives as an
argument and the first tab opens it rather than Home.

The window offers this once at startup. "Never show this again" is stored as a
preference, and the More menu can turn the offer back on or act on it directly.

Windows has no equivalent *setting*: Explorer is the shell, and folders are part
of it rather than a file type that can be associated. What third-party file
managers do instead, and what Lyre now does, is override the shell's default verb
for directories and drives. It is a per-user registry change under
`HKCU\Software\Classes`, so it needs no administrator rights, the stock `open`
verb is left untouched, and "Stop opening folders with Lyre" in the More menu
puts it back.

The honest caveats: this is a registry override rather than a supported API, so
anything that launches Explorer directly still gets Explorer, and a Windows
update can reset it. Restarting Explorer, or signing out, may be needed before
the change takes effect.

macOS is the one with no route at all: Finder is the shell and offers nothing
equivalent.

## Memory

Measured on a 20,000 file folder, with the numbers from `/proc/<pid>/smaps_rollup`
rather than a guess:

- The search cache keeps 78 bytes per row instead of 187. It stores what it
  needs to match and rank, and rebuilds the display row only for the rows that
  actually match. A test asserts the rebuilt row is identical to the original.
- The cache is capped by total rows as well as by folder count, so browsing a
  few huge folders cannot pin tens of megabytes while the window sits idle.
- **More → Reduce graphics memory** takes about 45 MB, 15 percent, off an idle
  window by turning off WebKit's GPU buffer sharing. It is off by default
  because that path is the fast one for drawing: the trade is an extra copy per
  frame, which is cheap at 60 Hz and not free at 300 Hz.

What is left is the framework. On this machine an idle window is about 300 MB
PSS, of which roughly 170 MB is the WebKit web process, 90 MB the application
process and 30 MB the network process. Those are WebKitGTK's own costs; the only
lever that moved them materially was the graphics setting above.

## Frame rate

A file manager does not animate continuously, so "it feels like 60 Hz" is
usually about scrolling rather than idle frames. **More → Measure frame rate**
samples the paint loop for 1.2 seconds and reports the median interval, which is
the only honest way to tell a capped webview from a misperception.

On WebKitGTK, a compositor falling back to software is the usual cause of a
locked rate. `WEBKIT_DISABLE_DMABUF_RENDERER=1` is the first thing to try with
NVIDIA drivers; `vblank_mode=0` or `__GL_SYNC_TO_VBLANK=0` affect whether the
compositor is allowed to exceed the monitor rate.

## Drag and drop

Dragging works onto a folder row, the empty space in a folder, any breadcrumb,
any tab, and the navigation rail. The pointer carries a badge saying **Move** or
**Copy**, and the zone under the pointer is outlined in the accent colour.

What it does depends on where it would land, before any key is pressed. Within
one volume a drag moves, because that is the point of dragging something to a
folder beside it. Across volumes it copies, because moving to a USB stick should
not take the file off the disk by accident and would mean a full rewrite anyway.
Shift inverts whichever is the default, and the badge follows as the pointer
crosses between volumes.

Moving an item into the folder it already lives in, or a folder into itself, is
refused rather than quietly turning into a rename.

Internal dragging is driven by pointer events, not HTML5 drag and drop, and that
is deliberate. Tauri's native drag handler is the only way to receive real
filesystem paths for a drop arriving from the operating system, and it replaces
WebView2's HTML5 handler on Windows, which would leave internal dragging working
everywhere except Windows.

**Dragging out of Lyre works.** Keep dragging past the window edge and the drag
becomes a real operating-system drag, so it can be dropped into Discord, a mail
client, LocalSend or another file manager. The copy-or-move mode goes with it, so
the platform shows the same cursor the badge was showing. This uses
`tauri-plugin-drag`, whose Linux implementation is GTK-based and therefore
backend-agnostic; it has been type-checked for Windows but the cross-application
drag itself still wants a real try on each platform.

Drops *from* the OS work and are always copies, since the source may be on
another volume or another machine.

## Keyboard

Explorer's shortcuts, as far as this build can honestly honour them.

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

Deliberately absent, because the feature behind them does not exist yet:
`Ctrl+Z` / `Ctrl+Y` (no undo history), `Ctrl+N` (a second window), `Ctrl+D` /
`Alt+Z` (pinning), `Alt+P` (preview pane), `F6`, and the view sizes we do not
render (`Ctrl+Shift+1/3/4/5/7/8`).

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

## How search stays in milliseconds

`src-tauri/src/search.rs`. Matching is [nucleo](https://github.com/helix-editor/nucleo),
the engine behind Helix: fuzzy subsequence matching that rewards contiguous runs
and word boundaries, and it returns the match positions so results highlight
what matched. Frizbee is faster on some benchmarks but newer, SIMD-dependent and
has no equivalent maturity story.

Four decisions carry the latency:

1. **The folder on screen is already in memory.** Each scan publishes its
   entries to an index keyed by path, so the first phase of a search reads no
   disk at all.
2. **Names are matched, metadata is not.** A recursive search reads directory
   entries and matches file names; only the items that match are stat'ed. The
   walk is breadth-first: depth-first disappears into the first large system
   tree it meets and spends its whole budget there before reaching your own
   folders.
3. **Two phases, one keystroke.** The local phase is emitted before the walk
   starts, so results appear immediately and deeper matches stream in behind
   them. Both are cancelled the moment the query moves.
4. **One sort, at the end.** Results stream in the order they are found, so the
   list never jumps while you type. A single stable sort by relevance runs when
   the walk completes.
5. **The comparator does not allocate.** Ranking 30,000 tied matches ran through
   `char::to_lowercase` iterators per comparison and cost 21 ms of a 23 ms
   search. A byte-wise comparison cut that to 2.7 ms.

Measured on a 6-core Ryzen 5 3600, 30,000 files in one folder, warm index:

| query | matches | time |
| --- | --- | --- |
| `re` | 30,000 | 2.1 ms |
| `report` | 30,000 | 2.6 ms |
| `doc7` | 10,317 | 1.6 ms |
| `dcmnt` (fuzzy) | 30,000 | 3.7 ms |

`cd src-tauri && cargo test --lib bench_packed_folder -- --ignored --nocapture`
reproduces it.

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
