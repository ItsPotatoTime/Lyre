# Changelog

What changed, in the order it shipped. The release workflow reads the section
for the tag being built and copies it into the draft release, so this file is the
release notes rather than a second copy of them.

## [0.2.0]

### Added

- **Lyre can be the folder handler on Windows.** Every shell verb a folder open
  goes through is written per user under `HKCU`, with `DelegateExecute` cleared
  so the inherited handler cannot win, and every value that is replaced is
  recorded before it is replaced so "Stop opening folders with Lyre" puts back
  what was there rather than guessing.
- **Win+E opens Lyre** while it is the folder handler, including when no window
  is open, through a small logon agent. A switch in the More menu turns that on
  and off, and handing folders back to Explorer hands the key back too.
- **Progress for copy, move and delete.** The status bar shows the item, the
  bytes, a bar and a percentage, and a Cancel that stops at the next chunk and
  removes the half written file.
- **Delete works on Windows.** It goes through the shell's own Recycle Bin call;
  before this it refused on anything that was not Unix.
- **Picture previews on a clean install.** Images are decoded in process, so a
  machine with no ffmpeg and no ImageMagick still shows photographs. Video and
  PDF thumbnails come from the system's own thumbnail providers, which is the
  same code Explorer uses.
- **The preview pane is a switch**: a cross in its header, a toggle in the command
  bar, and the choice remembered with the rest of the view.
- **Updates from the project's own releases**, signed, with the download visible
  and the signature checked before anything is installed.
- The mouse's side buttons walk the tab history.

### Changed

- The frame-rate meter caps its measurement at thirty frames a second and reads
  the rate from the span between frames rather than the median interval.
- `rust-version` is 1.90, which is what Tauri 2.12 requires; the old 1.77 had not
  been buildable for a while.

### Fixed

- **Windows: the folder-handler registration was a no-op that reported success.**
  It named a verb that was never created, so nothing changed and the interface
  said it had.
- **Windows: Win+E fell back to Explorer** after a reinstall, or after the window
  that owned the hook was closed. The hook now lives in a helper process, one
  press opens one window, and holding the key no longer sprays windows.
- **Windows: windows could start minimised**, because the launcher's show state
  was inherited. Lyre now asks the shell for a normal window.
- **The preview cache had no location on Windows**, so no thumbnail could be made
  there at all, whatever decoders the machine had.
- **`convert.exe` on Windows is the NTFS tool, not ImageMagick.** It is no longer
  called there, and `magick` is looked for first everywhere.
- **A cancelled paste left the bar frozen** at whatever percentage it had
  reached, and left a partially written file behind. It now hears the operation
  end and removes what it wrote.
- A move that copied everything and then could not remove the original is
  reported as that, rather than as an item that was skipped.
- The test fixtures build their paths the way production does, which is what made
  the suite compile and pass on Windows.

## [0.1.0]

First release: tabs, navigation rail, details and large-icon views, search,
previews, drag and drop, volumes and eject, and the default-file-manager switch
on Linux.

[0.2.0]: https://github.com/ItsPotatoTime/Lyre/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ItsPotatoTime/Lyre/releases/tag/v0.1.0
