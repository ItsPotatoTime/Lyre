# Changelog

What changed, in the order it shipped. The release workflow reads the section
for the tag being built and copies it into the draft release, so this file is the
release notes rather than a second copy of them.

## [Unreleased]

### Fixed

- **Rename and new folder refuse names that would leave the folder.** A typed
  name was joined onto the folder as-is, so a name like `../notes.txt` moved
  the item out of its folder, and an absolute name on Windows replaced the
  folder outright. Names carrying a separator, a parent step, or nothing at
  all are refused now. A rename that only changes a name's case also works
  on case-insensitive filesystems, where it used to report that an item with
  that name already exists: the item it meant was itself.

## [0.3.5]

A release for a move that went wrong and took the projects with it: what a paste
did is kept even when it stopped part way, a folder that was made is only removed
again while it is still empty, and replacing merges into a folder instead of
deleting what only the destination had.

### Fixed

- **A move that stops half way can be taken back.** A paste that landed some of
  its items and then failed recorded nothing to undo, so `Ctrl+Z` fell through to
  taking the folder away and deleted the folder those items had just landed in
  with them. What a paste did is now recorded even when it stops, the status bar
  says how many went before it did, and an undo that cannot finish all of it
  keeps what it could not do for the next press instead of reporting it done.

- **A folder that was made is only removed again while it is still empty.** Once
  something is inside it, `Ctrl+Z` says so and leaves it.

- **A selection that names its own destination is refused before anything
  moves**, so a drop onto a folder that is part of the selection no longer
  leaves half of itself done.

- **Replacing keeps what only the destination has.** Choosing Replace deleted the
  destination and copied into a fresh one, so a folder's other files went with
  it, and a copy aimed at the item's own path emptied it. A folder is merged into
  now, a file is written over, and an item is never written over itself.

## [0.3.4]

A release for what the window was keeping from you: a page that overwrote your
sort, a minimise that shrank the window for good, a listing that ran off the
edge, a rubber band that could only start on one side, and a crash that said
nothing. Undo and redo arrive with it, including taking a delete back out of the
bin.

### Added

- **Undo and redo.** `Ctrl+Z` takes back the last thing done to a file, and
  `Ctrl+Y` or `Ctrl+Shift+Z` does it again: a rename, a new folder, a copy, a
  move, or a delete to the trash. A copy is taken back by deleting what it made,
  and a move by putting each item back where it came from, recreating a folder
  that has gone since. A delete comes back out of the bin itself: on Windows the
  bin's own record of where the item came from is what finds it again, and on
  Linux and macOS the trash note Lyre wrote is. Doing something new drops the
  steps that could have been redone, as an editor does, and a permanent delete is
  not undoable, which is what makes it permanent. Both commands are in the More
  menu, dimmed when there is nothing to take back.

- **A crash leaves a note.** A release build is a window, not a console program,
  so a panic closed the window and left nothing to read. The reason and the line
  it happened on now go to `lyre-crash.log` beside the settings, which is
  `%APPDATA%\Lyre` on Windows, so the next "it just closed" can be answered
  instead of guessed at.

### Changed

- **A launch builds one App, not two.** `main` constructed a whole second App
  before the window existed, only to print what it had scanned: one folder read
  and every preference, twice. That count is now printed only when
  `LYRE_REPORT_SCAN` asks for it, and `LYRE_TIMING=1` prints what each step of the
  launch cost. With it, a launch on this machine is 27 ms to load the executable,
  11 ms of the app's own work, and about 1.35 s inside Rune creating the GPU
  device and its render pipelines, which is where the rest of the wait is.

- **The window goes much smaller, and the layout holds.** The floor was 760 by
  480; it is now 400 by 280. Below about 700 points of width the command bar
  keeps its icons and drops their words, below 560 the file-operation buttons
  step aside for the context menu, the rail gives up its width down to 45% of the
  window, the details pane steps aside when there is no room for it beside the
  list, and the Details columns take the shortfall out of all four rather than
  pushing the last two past the edge.

### Fixed

- **A rubber band starts from anywhere in the empty list.** The band's handler
  sat on the rows themselves, so in a folder with a short listing a drag could be
  started to the right of the rows but not below them: that space belongs to the
  scrolling area, which had no handler. It is on the scrolling area now, in all
  four views, and a press on a row still drags the row because the row takes the
  drag first.

- **A minimised window no longer shrinks the window for good.** While minimised,
  the window reports the sliver it collapsed to, 160 by 28, and that was being
  saved as the size to open at. Every launch after a minimise-then-close came back
  at the smallest size allowed, which read as the whole interface falling apart.
  A minimised window now remembers the size it had before, and it draws itself
  again the moment it comes back rather than waiting for a click.

- **No page overwrites the view, the sort or the details pane.** Opening This PC
  put the sort column back to Name and left the direction alone, so a Date
  modified, descending sort came out as Name, descending, and that was saved. It
  was the last of them: laying out a page no longer touches any of the three, and
  a new tab opens the way the others look instead of resetting them.

- **Nothing is drawn past the right edge.** The Quick Access tiles were three to
  a line whatever the width, so the third one hung off a narrow window, and a
  long file name was drawn over the date beside it because its column had no room
  to truncate in. The tiles now wrap to however many fit and take a share of the
  page, and the name, date and type cells truncate inside their own column. The
  file list was asking for the whole window's width beside the rail rather than
  what the rail left, which is why a narrow window's rows ended past the edge.
- **Cut and paste no longer dies on a folder that links back up.** Copying walked
  through links as though they were the folders they name, so a folder holding a
  junction to one of its own parents was copied deeper and deeper until something
  gave: the path ran past what Windows accepts, and with long paths allowed the
  recursion would have run out of stack and taken the whole window with it. A
  link is now copied as a link and never walked through, and both walks stop well
  before either happens. The size the progress bar counts is guarded the same way.
- **A selection a refresh removed cannot take the window down.** The status bar
  worked out its detail from the selected indices alone, so a refresh landing
  between the press and that count could name a row that was no longer there.

## [0.3.3]

A patch release for the things that quietly did not work: Back out of a page,
the mouse's side buttons, a delete that failed, the details pane, and every
dialog's buttons.

### Added

- **Dialogs answer the keyboard.** A dialog draws its buttons from the one list
  the keyboard walks, and it opens with the keyboard on its primary button: press
  `Delete`, then Enter, and the item is deleted. The arrows move between the
  buttons, Enter or Space presses the one they are on, and Escape still closes
  without doing anything. A dialog with a name field leaves every key to that
  field while it has the caret, so typing and Enter there behave as they always
  have. The two small links inside dialogs, "Don't ask again" and "Don't check at
  startup", are still the mouse's.

- **A tab closes with Alt+click or the middle mouse button.** Both are what a
  browser does, and both close the tab under the pointer, its close button
  included. The middle button is not one the toolkit reports, so the window
  catches the press itself and closes the tab it landed on; a press anywhere but
  the strip closes nothing.

### Fixed

- **No page turns the details pane on for you.** Opening This PC switched the
  pane on and nothing switched it back, so one visit changed the setting for
  every folder afterwards, and the change was saved with the rest; the Gallery
  did the same to show a photo's details. Quick Access was quietly switching it
  off on the way in. The pane belongs to the person: This PC, the Gallery, Quick
  Access and a step back onto any of them now leave it exactly as it was. The
  reference scene for This PC sets it for its own capture only.

- **A failed delete says why it failed.** Every finished operation reloads the
  folder on screen, and reloading clears the notice; the outcome was written
  before that reload, so a delete Windows refused reported into the clearing and
  said nothing at all. The message now lands after the reload. Creating a folder
  in a folder you may not write to already said so, and both are covered by tests
  that give a folder and a file a deny entry and expect the message and no crash.

- **Back returns to the page you came from.** History recorded folders, and
  This PC, Quick Access and the Gallery are not folders: each keeps the last real
  folder behind it, and that folder was the whole of what a step recorded. Opening
  This PC and then a folder inside it, Back went to whatever This PC itself had
  been opened from instead of returning to the drive list. A step now names the
  page as well as the folder, so Back, Forward, `Alt+Left`/`Right` and the mouse's
  side buttons all land on the page that was left, and pressing a page you are
  already on is not a step of its own. A folder deleted since, or a drive
  unplugged since, says it is not available there and leaves the window where it
  is rather than drawing an empty folder.
- **The mouse's side buttons act on the press, and the window follows.** The
  press was taken by the window before the toolkit saw it, and a message taken
  that way wakes nothing: an idle window never ran the step that reads it, so
  Back and Forward on the mouse did nothing until the pointer happened to move
  over the window. The press now asks for a paint, and the step it takes is
  reported as a change, so the page it lands on is drawn at once instead of the
  window keeping the old picture. A device that sends the browser buttons as an
  app command rather than as an X button is handled the same way.

## [0.3.2]

A patch release for the rail: its places are found where the system keeps them,
Home is the home folder, and its rows answer a press wherever it lands.

### Changed

- **Quick Access is the landing page, and Home is the home folder.** The page
  with recent files and favourites was called Home, which left the real home
  directory with no row of its own. The page is now Quick Access, and Home is the
  home directory — `/home/<name>`, `C:\Users\<name>` — with its own rail row, its
  own breadcrumbs, and Up to the folder that holds it. A window still opens on
  Quick Access.

- **A search walks the disk once, not once per keystroke.** Every change to the
  search box cancelled the running walk and started a fresh one, so typing six
  letters walked the disk six times and answered once. A query is now walked 180
  ms after the typing stops, the tree is read on every core but one, and the
  names a walk finds are kept for the rest of that search, so the next query in
  the same box matches them without touching the disk. Measured on
  `C:\Windows\System32` in a release build, a query that matches nothing went
  from 185,000 entries a second to about 520,000, so a five-million-entry disk
  takes seconds rather than half a minute. The kept names are dropped when the
  search box is cleared or the folder changes, so an answer is never older than
  the search it belongs to.

### Fixed

- **Shortcuts to folders open as folders.** A folder reached through a link —
  the `My Videos`, `My Music` and `My Pictures` junctions in Documents, or a
  `.lnk` someone made — was described by the standard library as a file, so it
  sorted with the files, drew a file's blank icon, and was handed to the shell to
  open. The shell asked Lyre, because Lyre is what opens folders, and a second
  window appeared on the folder instead. A link is now listed as the folder it
  is, with the folder's icon, and opens in the window it was pressed in. A
  shortcut to a file is still the shell's, so it keeps its arguments, its working
  directory and its own icon, and one whose target has gone draws its type's mark
  rather than nothing. Where Windows denies reading through a link, as it does
  for those three junctions, the folder the link names is what opens rather than
  an empty one.
- **Desktop, Documents, Pictures and the rest are found where the system keeps
  them.** The rail, the Quick Access tiles and the Gallery joined the English
  name onto the home folder, but that is only where these folders start: a person
  can move any of them, and OneDrive's folder backup moves Desktop, Documents and
  Pictures without asking. On a machine set up that way the row for Desktop and
  the Gallery both answered "not available here". Each one is now resolved from
  the same place Explorer reads (`User Shell Folders`), from `user-dirs.dirs` on
  Linux, and from the home folder on macOS, and a folder that is not there is not
  offered rather than drawn as a dead row.
- **A press on a rail row opens it wherever it lands.** The press was on the
  row's inner content, so its padding, the spacer beside the label and the place's
  own icon took the press and did nothing: the row looked clickable and part of it
  was not, which read as needing several presses.
- **The tab strip names the folder on screen.** A tab's label and icon were read
  from the snapshot taken when it was last left, so a tab that navigated kept
  saying Quick Access until another tab was opened or switched to.
- **Only the place on screen is selected.** The rail lit whichever row was last
  pressed, so opening a pinned folder left This PC selected beside it, and two
  pins of the same folder could both look selected. Selection comes from the
  folder the window is actually in, so exactly one row can be lit, and a pin
  saved twice (with a trailing separator, or in another case) is folded into one.
- **Up no longer leaves a place.** Quick Access and This PC are places rather
  than folders, but Up read the parent of the folder they had replaced and
  offered `C:\Users\<name>` from This PC. Up is disabled where there is nothing
  above, and closes with a notice if it is pressed anyway.
- **A tab no longer carries its search or its drag into the next one.** Switching
  tabs kept the leaving tab's search running, so the arriving tab drew the other
  folder's results, and a drag in progress survived the switch.
- **The Gallery survives being switched away from and back.** Restoring a
  Gallery tab, or a write anywhere under Pictures while it was open, replaced the
  date-grouped photo grid with a flat listing of the folder's top level.
- **Favorites and pins come over from 0.2.** They were not lost by a broken save:
  0.2 kept them in the webview's local storage and 0.3 keeps its settings in
  `prefs.json`, so the first 0.3 launch found nothing to read. The old lists are
  now imported once, at the first launch of this build.
- **The update signature is no longer encoded twice.** The release script encoded
  each signature one level deeper than the manifest carries, so a client refused
  it with "Invalid encoding in minisign data". New releases store the signature
  file exactly as `tauri signer sign` writes it, and the client decodes a second
  time when the first does not parse, so manifests published the old way keep
  verifying.

## [0.3.1]

A patch release for what the rewrite got wrong.

### Fixed

- **The keyboard shortcuts are back.** The rewrite kept only a handful, so
  `Alt+Left`/`Right`/`Up` for history, `F5` refresh, `F4`, `F3` and `Ctrl+F`/`E`,
  `Ctrl+B`, `Ctrl+1` to `Ctrl+9`, `Ctrl+Shift+N`, `Alt+Enter`, `Ctrl+Shift+C`,
  `Ctrl+Shift+2`/`6`, `PageUp`/`PageDown`, `Space`, `Escape` clearing the
  selection, and typing to jump to a name all work again. So do the two side
  buttons on a mouse, which walk the same history as `Alt+Left` and `Alt+Right`;
  Windows delivers those to the window directly, since the toolkit has no notion
  of them. `F11` and `Shift+F10` are still missing: the first needs full-screen
  support the toolkit does not expose, and the second needs a pointer position to
  anchor the menu at.
- **Sorting by Date modified compared the date's text, not the date.**
  "4/30/2026" and "10/20/2026" were ordered as strings, so November came before
  February and the list looked unsorted. The timestamp that sits beside that text
  is what the sort uses now, here and in Group by date.
- **The rail chevrons on This PC and Network did nothing.** They had no state
  behind them, and the row carried the click. Opening This PC now lists the drives
  read at that moment, and Network says browsing is not implemented rather than
  offering an arrow that opens nothing.
- **Drives in the rail were named by their letter and could not be right-clicked.**
  An unlabelled system drive read "C:". They use the same name the This PC cards
  do, "Local Disk (C:)", and open the same menu, so an inserted stick can be
  ejected from the rail.
- **The window forgot its size.** It opens at the size it was left at, clamped to
  the screen it opens on and raised to the window's minimum where it is smaller.
  Reading the size back is Windows-only; elsewhere the window opens at its default
  and nothing is saved.
- **The chosen sort and view were lost at every launch.** They were being saved,
  and then startup replaced them: opening on Home loads a fixture that resets the
  mode, the sort and the preview pane, and that reset was written back as though it
  had been chosen.

## [0.3.0]

This release is the interface rewritten. Lyre's window is no longer a web page:
it is drawn by Rune's own GPU renderer from Rust, with no webview, no React and no
JavaScript anywhere in the shipped application. One native binary per platform now
carries the whole file manager.

### Why it was rewritten

- **The web layer is gone.** The old application was a Rust core behind a Tauri
  webview and a React interface, which meant shipping a browser engine inside a
  file manager and paying for a DOM node per row. The window is now a single Rust
  crate drawing its own rows, so a folder of twenty thousand entries costs the
  scan and nothing else.
- **The chrome belongs to the application.** The titlebar, tabs, address row,
  command bar, resize edges, minimize and maximize are drawn and handled by Lyre
  rather than borrowed from a native frame. The Windows 11 Explorer layout is the
  layout on every platform instead of an approximation of one.
- **The Tauri application is deleted.** `src/` and `src-tauri/`, the React
  components, the Vite toolchain and the vendored wry and drag patches are gone
  from the tree. The repository is one Rust crate, its packaging and its docs; the
  old application remains in git history.

### What the window does

- **The Explorer layout**: tabs, history, breadcrumbs, the navigation rail, and
  details, grid, list and tiles, with Home for favourites and recents, This PC
  with live volumes, and a preview pane beside the list.
- **Folders refresh themselves**, and one that is slow to read says so. The scan
  runs off the UI thread, the status bar counts items as they arrive, and Escape
  stops it.
- **File operations with real progress**: copy, move, rename, delete to the trash,
  conflict prompts, and a transfer that reports bytes while it runs.
- **Dragging both ways, natively.** Files come in from Explorer, 7-Zip or a
  browser and go back out to those same targets, implemented against the shell's
  own data object and drop target rather than through a patched webview. An
  archiver that deletes its temporary folder the instant the drag returns no
  longer loses the files.
- **Other programs' menu items**, read from Windows: registry verbs including
  cascaded submenus, and the extensions whose items exist only because their DLL
  builds them at runtime. "Customize this menu" groups everything by the program
  that provides it, opens a group on hover, and loads a third-party extension only
  when its switch is turned on. Registrations that cannot be read are named with
  the reason instead of quietly missing.
- **Open with**, on every platform: the applications that declare they can open
  the file's type, with the current default marked and preselected, and the choice
  can be made permanent where the platform allows it.
- **Properties**, with what a file can say for itself: an image's dimensions and
  aspect ratio, a text file's lines and words, a folder's contents, and the size
  and timestamps of anything at all. The preview pane shows the same facts beside
  the list.
- **Appearance** follows the desktop or is held to light or dark, and each
  platform is asked in its own place: the registry on Windows, `defaults` on
  macOS, `gsettings` on freedesktop.

### Notes for this version

- Icons come from the packaging that already existed: a resource compiled into the
  Windows binary, the `.app` bundle on macOS, the `.desktop` entry on Linux.
  Release builds are window-subsystem binaries, so no console appears behind the
  window.
- Updates are signed and checked against the key compiled into the application. A
  **0.2.0 AppImage on Linux cannot update itself to this version** and needs one
  manual install: writing over a running AppImage is refused by the kernel, and
  that fix only exists in 0.3.0's own updater. Windows updates from 0.2.0
  normally; on macOS the archive is still opened for you and has to be moved into
  place by hand.
- Windows 11's modern context menu items are still not read. They are activated
  through COM and draw their own menu, so nothing about them is reachable from the
  registry, and the customize list says so rather than leaving a gap.

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
