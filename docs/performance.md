# Performance

Three things are measured here: how long a folder takes to appear, how long a
search takes to answer, and what the window costs while it sits there. Each
section names the command that reproduces its numbers.

The machine these were taken on: Ryzen 5 3600 (6 cores, 12 threads), NVMe, btrfs
on `/home`, tmpfs on `/tmp`, a development build (`opt-level = 1`). A release
build is faster; these numbers are the pessimistic ones.

## Listing a folder

`src-tauri/src/scan.rs`. A folder with 100,000 entries must paint its first
screen within a frame or two of the click, which rules out the obvious
implementation (collect everything, sort, return) because it defers all feedback
until the slowest `stat` in the directory finishes. Three decisions carry it:

1. **Enumerate names, then stat in parallel.** `read_dir` gives names and file
   types without a syscall per entry. The names are cut into 512-entry chunks and
   stat'ed by a fixed pool of workers, two per core.
2. **Stream, do not wait.** Each finished chunk goes to the webview over a Tauri
   IPC channel the moment it exists. The first rows paint while the rest of the
   directory is still being read, and the status bar count climbs.
3. **The interface owns the comparator.** Rows arrive unsorted and are merged
   into an already-sorted array one batch at a time, one linear pass per batch.
   Sorting happens in Rust only where it is free (nothing per entry), and the
   only full sort is when the user clicks a column.

| Directory | Entries | Scanner | Including the first batch |
| --- | --- | --- | --- |
| 20,000 files, btrfs + NVMe | 20,000 | 16 ms | 18 ms |
| 20,000 files, tmpfs | 20,000 | 13 ms | 15 ms |

```bash
cd src-tauri && cargo test --lib bench_twenty_thousand -- --ignored --nocapture
```

The second column is what the window sees first: rows are on screen before the
total is known.

Rendering is windowed too: a 4,200 entry folder keeps about 30 rows in the DOM,
and the row arrays live in refs rather than React state, so a fast scan does not
push 50,000 element array copies through the reconciler.

## Searching

`src-tauri/src/search.rs`. Matching is [nucleo](https://github.com/helix-editor/nucleo),
the engine behind Helix: fuzzy subsequence matching that rewards contiguous runs
and word boundaries, and it returns the match positions so results highlight what
matched. Frizbee is faster on some benchmarks but newer, SIMD-dependent and has
no equivalent maturity story.

Five decisions carry the latency:

1. **The folder on screen is already in memory.** Each scan publishes its entries
   to an index keyed by path, so the first phase of a search reads no disk at
   all.
2. **Names are matched, metadata is not.** A recursive search reads directory
   entries and matches the name, of a file or a folder; only the items that match
   are stat'ed. The walk is breadth-first: depth-first disappears into the first
   large system tree it meets and spends its whole budget there before reaching
   your own folders.
3. **Two phases, one keystroke.** The local phase is emitted before the walk
   starts, so results appear immediately and deeper matches stream in behind
   them. Both are cancelled the moment the query moves.
4. **One sort, at the end.** Results stream in the order they are found, so the
   list never jumps while you type. A single stable sort runs when the walk
   completes: folders first, then the files, each by relevance.
5. **The comparator does not allocate.** Ranking 30,000 tied matches ran through
   `char::to_lowercase` iterators per comparison and cost 21 ms of a 23 ms
   search. A byte-wise comparison cut that to 2.7 ms.

### Numbers

A recursive search over real trees, warm page cache:

| Tree | Entries examined | `libc` (17,739 hits) | `zzzzzzz` (0 hits) | `cargo` (4,317 hits) |
| --- | --- | --- | --- | --- |
| `/usr` | 639,755 | 110 ms | 98 ms | 115 ms |
| `/home` | 929,929 | 226 ms | 226 ms | 213 ms |

```bash
cd src-tauri && cargo test --lib bench_real_tree -- --ignored --nocapture
```

The cost is the walk, not the number of matches: seventeen thousand hits and no
hits cost the same. The same tree with a cold page cache took 931 ms and 1,696 ms
respectively, which is the disk rather than the search.

One folder of 30,000 files, warm index: what typing in a large directory costs.

| Query | Matches | Time |
| --- | --- | --- |
| `re` | 30,000 | 2.9 ms |
| `report` | 30,000 | 2.7 ms |
| `doc7` | 10,317 | 2.5 ms |
| `dcmnt` (fuzzy) | 30,000 | 4.8 ms |

```bash
cd src-tauri && cargo test --lib bench_packed_folder -- --ignored --nocapture
```

### Accents

They do not have to be typed. `tari` finds `Țări`, `lodz` finds `Łódź`, `tieng`
finds `Tiếng`, `елка` finds `Ёлка`, and the accented spelling works too. Names and
queries are both folded through Unicode decomposition, so this covers every script
that composes its diacritics rather than a list of languages, plus a small table
for letters that are a single glyph such as `ł` and `ø`. Folding never changes the
number of characters, which is what keeps the highlight on the letters the person
can see.

### One ceiling

A walk examines at most five million entries. It used to stop after three seconds
as well, which meant the same query returned different answers on different
machines, because a slow disk examines fewer entries in a fixed time. A count is a
property of the tree rather than of the hardware, so what one person finds is what
another finds. The cap is high enough that only a genuinely enormous tree meets
it, and the status bar says so when it does.

## Memory

Measured on a 20,000 file folder, with the numbers from `/proc/<pid>/smaps_rollup`
rather than a guess:

- The search cache keeps **78 bytes per row instead of 175** (55 percent less). It
  stores what it needs to match and rank, and rebuilds a display row only for
  entries that match. A test asserts the rebuilt row is identical to the original.
  ```bash
  cd src-tauri && cargo test --lib the_compact_form_costs_far_less_per_row -- --nocapture
  ```
- The cache is capped by total rows as well as by folder count, so browsing a few
  huge folders cannot pin tens of megabytes while the window sits idle.
- **More → Reduce graphics memory** takes about 45 MB, 15 percent, off an idle
  window by turning off WebKit's GPU buffer sharing. It is off by default because
  that path is the fast one for drawing: the trade is an extra copy per frame,
  which is cheap at 60 Hz and not free at 300 Hz.

What is left is the framework. An idle window is about **300 MB PSS**, of which
roughly 170 MB is the web process, 90 MB the application process and 30 MB the
network process. Those are WebKitGTK's own costs; the only lever that moved them
materially was the graphics setting above.

## Frame rate

A file manager does not animate continuously, so "it feels like 60 Hz" is usually
about scrolling rather than idle frames. **More → Show frame rate** puts the paint
loop in the status bar: it samples continuously in one second windows and reports
the median rate, so the number is live rather than a single reading that has
already gone by the time you look at it. A rate that does not move while you
scroll a large folder is a cap, and the number names it.

The meter keeps one pixel changing every frame, so it measures a window that is
actually being drawn rather than one being spared work. That is also why it is a
switch you turn on for a moment and not a setting: while it is on, the compositor
is working as hard as it can.

### The ceiling on Linux

The webview has a ceiling of its own. Measured on this machine (WebKitGTK 2.54.2,
Hyprland 0.56.2, a 1920x1080 panel at 300 Hz, Radeon 6700 XT on Mesa), the same
second of `requestAnimationFrame` gave:

- GTK's own frame clock: **294 to 297 Hz** with the window redrawn every frame, and
  60 Hz when nothing asks for a frame.
- WebKitGTK: **59 Hz**, for a 1x1 transparent probe, for a 1x1 repaint, and for a
  full-window repaint every frame. The same number with the dmabuf renderer
  disabled, with compositing disabled, and under XWayland.
- WebKitGTK with `WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS` set to 300 and to 0: still
  59 Hz.

So the monitor is not the limit, the compositor is not the limit, and the meter is
not wrong. The build contains WebKit's own preference for this,
`PreferPageRenderingUpdatesNear60FPS` ("Prefer page rendering updates near 60
frames per second rather than using the display's refresh rate"), and 2.54 exposes
no property or environment variable that turns it off, so nothing the application
sets moves the number. On Linux the window is a 60 fps window because its renderer
is one.

Where the reading is far below 60, a compositor falling back to software is the
usual cause: a desktop in a VM measured 31 Hz. On WebKitGTK,
`WEBKIT_DISABLE_DMABUF_RENDERER=1` is the first thing to try with NVIDIA drivers;
`vblank_mode=0` or `__GL_SYNC_TO_VBLANK=0` affect whether the compositor is
allowed to exceed the monitor rate.

On Windows there is no equivalent engine ceiling to look for: Chromium's paint loop
follows the display, so a rate near 60 on a faster panel means the webview has lost
its hardware compositing path. Check the panel's own refresh rate in Windows
Settings first.
