# Changelog

What changed, in the order it shipped. The release workflow reads the section
for the tag being built and copies it into the draft release, so this file is the
release notes rather than a second copy of them.

## [Unreleased]

### Added

- **Folders refresh themselves.** Files another program saves, edits, renames or
  deletes appear without pressing F5. Windows uses `ReadDirectoryChangesW`; Linux,
  macOS and volumes that refuse a notification compare a fingerprint of names,
  sizes and modified times on an interval that grows with the folder, from 500 ms
  to 3 s. Bursts are collapsed into one re-read, an inline rename keeps its row
  until the editor closes, and search results still refresh on the next query.
- **The system's own context menu items, on Windows.** What other programs
  register now appears in Lyre's menus: "Open in Windows Terminal", 7-Zip's
  archive commands, and the rest. Verbs are read from the registry; a menu handler
  is asked what it would draw, which is the same call the shell makes; and choosing
  one runs it through the program that registered it. A new program is picked up
  when its registration is there, with no list to maintain. Windows 11
  `IExplorerCommand` handlers and cascading `SubCommands` verbs are not runnable
  yet and are named as such rather than left out quietly.
- **Customize this menu**, at the bottom of every right-click menu and in the More
  menu. It lists everything that can appear in that menu, Lyre's own items and the
  ones other programs added, with a tick per item and a count of how many are
  shown. Turning something off is remembered. It also lists every registration
  that exists but could not be read, with the reason.

### Changed

- **What other programs add to the menu is off until it is asked for.** Every
  system verb, handler and background item now starts disabled, so installing an
  application cannot change what a right-click does. They stay listed in
  "Customize this menu" and are switched on one at a time. Lyre's own items are
  unchanged.
- **The unused directory-mtime stamp is gone.** Editing a file in place leaves
  the parent folder's timestamp alone, so it could never have detected the very
  change this feature is for. The watcher compares names, sizes and modified
  times instead.

### Fixed

- **"Add to favorites" appeared twice in Customize this menu.** Windows registers
  an "Add to Favorites" verb that duplicates Lyre's own row. A system entry whose
  name matches one of Lyre's items in that menu is now left out, and each submenu
  deduplicates against itself. A group left with no children is dropped rather
  than becoming a clickable row, and an "Open" inside a program's own submenu is
  left alone.
- **The missing "Scan with Bitdefender" is explained rather than silent.** It is
  a Windows 11 modern context menu command declared in a sparse MSIX package
  (`desktop4:FileExplorerContextMenus`), not a verb or a `ContextMenuHandlers`
  entry, so the classic walk cannot see it. The customise list now states that
  gap, and `print_shell_verb_registrations` prints classic verb keys and
  packaged-COM packages side by side.
- **A registration that failed to load no longer looks like one that does not
  exist.** Loading a menu handler means loading someone else's DLL. A missing DLL,
  a refused selection or an unsupported initialiser were all swallowed, so the
  item simply was not there. "Customize this menu" now names the program, the
  class id and the reason; the ignored test
  `print_what_the_installed_handlers_offer` prints the same for a whole machine.
- **Handlers written against the newer initialisers are read.** Only
  `IShellExtInit` was asked for, so a handler implementing
  `IInitializeWithItem` or `IInitializeWithFile` instead never drew anything.
  All three are tried, in that order.
- **Background menu items are no longer all credited to Windows.** The folder's
  own menu arrives whole, so there is no honest way to say which program owns a
  row. Lyre now says nothing rather than filing a third party's item as a system
  one.
- **Linux and macOS no longer show a message about the Windows registry.** Each
  says why other programs' items are unavailable in its own terms, and both show
  only Lyre's own items. A menu id can no longer name a registry key outside the
  one it was read from.
- **PRODUCT.md described the first-pass plan as shipped state.** File operations,
  search, drag and drop, thumbnails and the preview pane had all shipped, and the
  signed update check is the one network request.
- **`bun buildit` clears older installers from `build/`.** A build from a previous
  version used to leave its `.exe`/`.msi` beside the new ones. `.workbuddy-ai/` is
  now ignored by git and left out of the Windows build copy.

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
