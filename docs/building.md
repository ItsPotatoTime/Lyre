# Building

## Requirements

Rust and [Bun](https://bun.sh), plus Tauri's prerequisites for the platform. On
Linux that means the development packages for `webkit2gtk-4.1`, `gtk3` and
`libsoup3`.

## Development

```bash
bun install
bun dev
```

`bun dev` is the whole thing: it starts Vite, compiles the Rust crate, and opens
the window against your real filesystem, with hot reload for the interface.

Vite starts on port 1420. If that port is taken, by a leftover dev server or a
second checkout, `bun dev` moves up to the next free one instead of failing, and
tells Tauri where it went: the port is chosen once, in `scripts/dev.ts`, and
handed to Vite through `LYRE_DEV_PORT` and to Tauri through `--config`, which
merges over `build.devUrl`. The window and the dev server therefore cannot
disagree.

```bash
bun dev --port 1500    # start somewhere specific
bun dev --dry-run      # print the port and the command without running it
```

**Settings live per origin.** The interface keeps its preferences, pins and theme
in the webview's own local storage, which belongs to the URL it was loaded from.
A window opened on port 1421 is a different origin from one on port 1420, so it
starts from the defaults and anything you changed elsewhere appears to be gone.
It is not: stop whatever is holding 1420 and start again, and the old settings
come back.

## Building a release

```bash
bun buildit
```

It detects the platform and does the right thing.

**Linux and macOS.** It installs dependencies and builds, and it picks the
installer formats the machine can actually produce: `deb` needs `dpkg-deb`,
`rpm` needs `rpmbuild`, and with neither present it builds just the executable
and says so.

**Windows** does more, because a Windows build cannot share state with a Linux
one:

- The source is copied to `%USERPROFILE%\lyre-winbuild` and built there, so the
  two platforms never write object files into the same `target`.
- MSVC is activated with `vcvars64.bat` inside the same shell that runs cargo,
  and the presence of Git's GNU `link.exe` is checked for, since it shadows
  MSVC's and produces link errors that look like source bugs.
- Those steps live in a generated `lyre-build.cmd`, so they can be read and
  re-run by hand instead of being buried in a quoted `cmd /c` string.
- Installers are copied back to `build/` on the shared disk.

Flags: `--debug`, `--clean`, `--bundles deb`, `--dry-run` (prints every command
without running it).

If a build fails after an edit on the other operating system, run
`bun buildit --clean`: on a shared filesystem the timestamps can be coarse enough
that cargo reuses a stale build script.

## Browser development

The interface also runs in a plain browser, which is how the layout is developed
without rebuilding Rust:

```bash
bun run dev:web    # http://127.0.0.1:1420, synthetic filesystem
```

In that mode `src/lib/mock.ts` supplies a deterministic filesystem, including a
4,200 entry photo library for exercising the virtualiser.

## Verification

Before a change is considered done:

```bash
bun run typecheck
bun run build
cd src-tauri && cargo test
cd src-tauri && cargo clippy
```

The Rust tests cover the scanner's contract: totals match what was sent, hidden
and protected entries are opt-in by name and by attribute, a large directory
streams in more than one batch without dropping or duplicating an entry,
cancellation does not claim completion, and a missing directory reports something
a human can read. The search tests cover ranking, both phases, the row cap, the
folded-accent path and the index round trip.
