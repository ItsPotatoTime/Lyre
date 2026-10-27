# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(Desktop application: rendered by Tauri v2's system webview, not served on the open web. The schema has no desktop value; every surface is a desktop window.)

## Stack

Confirmed by the user: Rust core + Tauri v2 shell + React 19 + TypeScript + Vite.
Cross-platform target: Windows, macOS, Linux. The user delegates build-tooling details.

## Users

People who use their operating system's file manager every day and are impatient with
it: developers, designers, power users, and anyone moving large numbers of files who
notices that the shipped explorer takes a visible moment to open a folder.

Their job: get to a directory, see what is in it, understand it, and act on it, without
waiting and without hunting for the control they need.

## Product Purpose

Lyre is a file manager that matches Windows 11 File Explorer's design language on every
operating system, and beats the platform explorer on the thing that matters most: it
shows a directory's contents almost instantly, at any size.

Success means a folder with 50,000 entries paints its first screen before the user's
perception of "loading" begins, and every subsequent interaction in it stays responsive.

## Positioning

Speed is treated as a design material, not an optimisation pass. Directory reads are
parallelised, streamed to the interface in batches so the first entries paint
immediately, cached and invalidated precisely, and never blocked by icon resolution or
metadata that the current view does not need. The visual design is a faithful Windows 11
Explorer, so a Windows user is instantly at home on macOS and Linux.

## Operating Context

- Runs as a native desktop window on Windows 11/10, macOS, and Linux (Tauri v2, system
  webview: WebView2, WKWebView, WebKitGTK).
- Used with a mouse and keyboard, on a large monitor or a laptop, alongside terminals,
  editors, and chat windows.
- Directories range from a handful of items to hundreds of thousands.
- Files and folders are local and remote-mounted volumes; listing must never hang the
  interface.
- The user's mental model comes from Explorer: left navigation rail, address bar with
  breadcrumb, command bar above the content, details/list view with sortable columns,
  tabs.

## Capabilities and Constraints

In scope now (first pass, confirmed with the user):
- Real filesystem browsing with a fast Rust core.
- Windows 11 Explorer shell: title bar, tabs, command bar, navigation rail, address bar,
  breadcrumb, search field, details and grid views, sortable columns, status bar.
- Keyboard navigation (arrows, Enter, Backspace, Home/End, type-ahead) and multi-select.
- Light and dark themes following the OS, with a manual override.
- Pinned folders and mounted drives in the navigation rail.

Explicitly deferred: file operations (copy, move, delete, rename, new folder), search
and indexing, drag and drop, thumbnail generation, preview pane, tabs persistence.

Constraints:
- No network calls, no telemetry, no accounts.
- Nothing may block the UI thread; a slow or unavailable volume must degrade, not freeze.
- Cross-platform from one codebase: no Windows-only assumptions in the Rust core.

## Brand Commitments

- Name: Lyre.
- The design language is pinned by the user to Windows 11 File Explorer (Fluent Design
  / WinUI 3): Mica and Acrylic materials, Segoe UI Variable type, 4/8 px radii, the
  reveal highlight, the accent blue. This is binding and is not to be reinterpreted.
- The user's stated reason is that they love the design. Fidelity is the brand.

## Evidence on Hand

No assets, screenshots, or brand files supplied. Icons and illustration are authored in
the project. File-type icons are drawn, not bitmap assets.

## Product Principles

1. The first screen of a directory is immediate; completeness is allowed to arrive a few
   milliseconds later.
2. Faithful to Explorer, so the interface is learned once and used everywhere.
3. The interface never lies about state: loading, empty, error, and partial results are
   each shown as themselves.
4. Do the work once, in Rust, and send the interface only what it renders.
5. Motion confirms an interaction and never delays one.
