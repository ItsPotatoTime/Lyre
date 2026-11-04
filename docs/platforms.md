# Platform notes

The three platforms answer "open this folder" and "open this file" differently
enough that the differences are not cosmetic. What follows is what Lyre does on
each, and where a platform makes something impossible it says so rather than
pretending.

**macOS is the untested one.** Linux and Windows are used; the macOS branches are
written but have not been run on a Mac, so read them as intent rather than
experience.

## Being the default folder handler

### Linux

Lyre registers as the handler for `inode/directory`: a `.desktop` entry in
`~/.local/share/applications` declaring that type, set with `xdg-mime default`.
Folders then open here whether they come from the desktop or from another
application. The entry uses `%U`, so the folder arrives as an argument and the
first tab opens it rather than Home.

The window offers this once at startup. "Never show this again" is stored as a
preference, and the More menu can turn the offer back on or act on it directly.

### Windows

Windows has no equivalent *setting*: Explorer is the shell, and folders are part
of it rather than a file type that can be associated. What third-party file
managers do instead, and what Lyre does, is override the command the shell runs
for the classes a folder open passes through. It is a per-user registry change
under `HKCU\Software\Classes`, so it needs no administrator rights:

- `Directory\shell\open` and `explore`, which is the class a folder path
  resolves to.
- `Folder\shell\open` and `explore`, for shell items that are not paths.
- `Drive\shell\open` and `explore`, for a volume opened from This PC.
- `CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow`, the File
  Explorer folder's own new-window verb, for anything that invokes it directly.

The machine's own `Folder` and CLSID commands carry a `DelegateExecute` handler
that runs inside Explorer, so that value is cleared alongside each command;
without it the inherited delegate wins and the command is never run.

Every value that is replaced is recorded first, under
`%LOCALAPPDATA%\Lyre\folder-handler.json`, and "Stop opening folders with Lyre"
puts back what was there rather than guessing at a default. A value another
application has taken over since is left alone rather than clobbered.

The honest caveats: this is a registry override rather than a supported API.
Anything that launches Explorer by full path still gets Explorer, which includes
"Show in folder" and "Open file location" (they call `explorer.exe /select,`) and
a second double-click inside an Explorer window, because that window navigates
itself. A launch the window cannot serve is handed to `explorer.exe` instead of
being dropped: a `/select`, a `shell:` namespace, a `::{CLSID}`, or one of
Explorer's own switches. A file argument opens the folder that holds it.

### Win+E on Windows

Win+E is not one of those verbs. Measured on Windows 11 25H2, the key spawns a
new process, `"C:\Windows\explorer.exe" ::{52205FD8-5DFB-447D-801A-D0B52F2E83E1}`.
That is a full path, so it consults neither the CLSID verb nor `App Paths`, and
no registry key can intercept it. What can be intercepted is the keystroke, so
while Lyre is the folder handler it installs a low level keyboard hook, takes
Win+E, and opens a window itself.

Two limits come with that. The hook lives in the running process, so Win+E goes
back to Explorer once every Lyre window is closed. And because each press opens a
new process rather than reusing the running window, every instance hooks the key
and a short lived named mutex decides which one opens the window for a given
press. An earlier version let only one instance hook, which meant closing that
window quietly returned Win+E to Explorer for every other window. Win+Shift+E,
Win+Ctrl+E and Win+Alt+E are left alone.

The hook runs on a thread of its own, not the one that draws the window: a low
level hook is delivered through the installing thread's message queue and the
system stops consulting a thread that answers too slowly, so a busy frame could
lose the key. The Windows key is tracked from the key stream rather than read when
the callback runs, for the same reason. Each press writes one line to
`%LOCALAPPDATA%\Lyre\hotkey.log` saying whether the key arrived, what the modifier
state was, and whether it was taken, which is what makes "Win+E still opens
Explorer" answerable without a debugger.

### Win+E with Lyre closed

The hook lives in a process, so with every window closed there is nothing to take
the key. "Take Win+E even with Lyre closed" in the More menu registers `LyreWinE`
under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which starts
`lyre.exe --win-e-agent` with the session. That is the same helper with one
difference: it does not stop when the last window closes, and it watches the
logon entry so turning the switch off ends it within a few seconds. It costs
about 9 MB and has no window.

The alternative would be an Image File Execution Options shim on `explorer.exe`.
It catches more, because Win+E, the taskbar button, the Start menu entry and even
`explorer.exe /select,` all launch that image, and it needs no resident process.
It also intercepts the shell's own startup, where the shim has to relaunch a
renamed copy of the real Explorer, so a stale copy or one wrong switch means no
desktop at logon. That is not a risk worth taking for one hotkey.

### macOS

No route at all: Finder is the shell and offers nothing equivalent.

## Open with

**Linux** is open. The MIME type comes from `xdg-mime`, the candidates come from
the `.desktop` entries the desktop itself reads, opening goes through
`gio launch`, and the default is a single file rewritten by `xdg-mime`.
Everything can be listed and changed from inside the window.

**Windows** is closed in two specific ways. The list of applications for a type
lives partly in AppX manifests that are activated through COM rather than shell
commands, so a from-scratch enumeration returns ProgIDs that cannot be launched
without `IApplicationActivationManager`. And the default is a `UserChoice` key
carrying a hash Windows validates, so writing it is tampering that the OS
reverts. Explorer itself answers "Choose another app" by opening
`SHOpenWithDialog`, which is what Lyre does: the operating system's own picker,
which knows every registered app and sets the default through the sanctioned
path.

**macOS** has no application list of its own here yet: the Unix implementation
looks for `.desktop` entries, which macOS does not have, so the list comes back
empty and opening a file goes to the system's own association through `open`.

## Context menu items from other programs

The "Other programs" section of a right-click menu is a platform feature, not a
portable one. Each desktop keeps its own registry for these items, and Lyre can
only read the Windows one today.

**Windows** reads two kinds of registration. A *verb* is a key under
`HKCR\<class>\shell` holding a command to run; a *handler* is a COM object under
`HKCR\<class>\shellex\ContextMenuHandlers` that is asked, through
`IContextMenu::QueryContextMenu`, what it would draw. Both are read off the
thread that draws the window, and the answer is cached per file type for five
minutes. A handler is third-party code loaded into this process, exactly as it is
into Explorer: a broken one can take the window down. There is no way to ask a
DLL what it would draw without loading it.

None of these items are shown on first install. Every one starts off and is
switched on individually in "Customize this menu", because installing a program
should not change what a right-click does. Lyre's own items are the ones that
start on. Rows that repeat something Lyre already offers under the same name —
Windows' "Add to Favorites" against Lyre's "Add to favorites", for example — are
left out of the list rather than shown twice.

A handler is told what is selected through one of `IShellExtInit`,
`IInitializeWithItem` or `IInitializeWithFile`, and Lyre tries all three. Every
registration that exists but could not be read is listed under "Customize this
menu" as a warning naming the program, the class id and the reason. That list is
where to look when an expected item is missing.
`cargo test print_what_the_installed_handlers_offer -- --ignored --nocapture`
prints the same information for the whole machine.

**Windows 11's modern menu is a third kind, and is not supported yet.** An app
can declare `windows.fileExplorerContextMenus` in a sparse MSIX package and
implement `IExplorerCommand`. Bitdefender's "Scan with Bitdefender" is exactly
this: `BDContextualPack.msix` registers CLSID
`325C7BE3-E00C-4943-ADC1-2387FD06E08E` for `*`, `Directory` and `Drive`. It
creates no verb key and no `ContextMenuHandlers` key, so the classic walk sees
nothing. Packaged COM classes are indexed under `HKCR\PackagedCom\ClassIndex`,
but the index does not say which are menu commands or which item types they apply
to — that lives in the package manifest. The shell's own `IContextMenu` does not
expose `IExplorerCommandProvider` either, so there is no generic enumeration a
separate application can borrow. Lyre can create such a command and read its
title, but discovering them all needs package-manifest reading and a full
`IExplorerCommand`/`IShellItemArray` implementation. The customise list states
this gap rather than leaving someone hunting for an item that cannot appear.
Cascading `SubCommands` verbs are also named as unsupported rather than dropped.

`cargo test print_shell_verb_registrations -- --ignored --nocapture` prints the
classic verb keys and the installed packaged-COM packages side by side.

**Linux** and **macOS** report that the section is unsupported, give the reason
in that platform's own terms, and show only Lyre's own items. On Linux the
equivalent registrations are `[Desktop Action ...]` blocks in `.desktop` files;
reading them is a separate feature, not a fallback that can be improvised. On
macOS, Finder keeps its menu extensions to itself and does not let another
application enumerate them. Neither platform calls a Windows API, shows a
Windows-shaped message, or offers an item that cannot run.

## Hidden and protected items

Two different things keep an entry out of a listing, and they are two switches
because Explorer treats them as two.

An entry is **hidden** when its name begins with a dot, or when it carries the
hidden attribute on Windows. "Show hidden items" in the View menu brings those
back, drawn dimmed so a list that is showing them still says which ones they
are: the name takes the dim tone (`--text-hidden`, 5.1:1 light and 5.3:1 dark)
and the picture drops to 55 percent, and both return to full strength once the
row is selected or focused.

An entry is **protected** when the system marks it as its own, which on Windows
means `FILE_ATTRIBUTE_SYSTEM`: `pagefile.sys`, `hiberfil.sys`, `bootmgr`,
`$Recycle.Bin`, `System Volume Information`, `Config.Msi`, the `DumpStack` logs.
Explorer keeps those out of the list even with hidden files turned on, and that
was the listing difference worth fixing: with hidden items shown, Lyre was
offering files Explorer would not. "Show protected operating system items" is off
by default, so the default listing is the one Explorer shows, and switching it on
lists everything the volume holds. No other platform marks entries this way, so
there the switch has nothing to reveal.

The reverse difference stays, and is worth knowing: a leading dot hides a name on
every platform here, because that is what a dot means everywhere except Windows,
where Explorer has no such rule and lists `.gitignore` normally. Explorer's own
rule is the file attribute, which is read as well.

Both filters are applied in the scanner and again in every path through search,
including the folder walk, the descent into subfolders and the rows answered from
a cached index, so a count can never disagree with the list.

## Watching a folder for changes

The window tells the Rust core which single folder is on screen, and the core
answers with `folder-changed` when its contents move. The interface then re-runs
the same streaming scan a manual F5 would. No new dependency is involved.

**Windows** uses `ReadDirectoryChangesW` with one outstanding overlapped read. It
fires on names, attributes, size, creation and last write, so an edit in place is
seen as well as an arrival. The read is issued with `FILE_FLAG_OVERLAPPED` and
collected with `GetOverlappedResult`; on the way out it is cancelled before the
buffer is dropped, which is the part that is easy to get wrong. Stopping is
checked on a 200 ms wait rather than interrupting a blocked call.

**Linux, macOS and any volume that refuses a notification** — network shares are
the usual case — fall back to comparing a fingerprint of the entries: names,
lengths, modified times and whether each row is a folder, hashed in sorted
order. That is plain `std`, so it is the same code on every platform and is
covered by unit tests. The cost is one metadata read per entry each time it is
checked, so the interval grows from 500 ms for a small folder to 3 s at twenty
thousand rows. Native watchers on Linux and macOS (inotify and FSEvents) would
remove that periodic work; they are not implemented yet.

One thread watches one folder and is replaced whenever the folder changes. It
never walks into subfolders: Lyre lists immediate children, and a change deep in
a tree is not a change to the listing on screen. Search results are not re-run
automatically, because repeating a recursive walk on every change is far more
expensive than the folder listing it would refresh.

## Dragging out of an archive

An archiver's drag is not a file drag. 7-Zip and WinRAR extract the selected
members into a throwaway folder (`%TEMP%\7zE9A2F\`, `%TEMP%\Rar$EXa0.123\`),
describe them with `CF_HDROP`, and delete that folder as soon as `DoDragDrop`
returns. Two things then go wrong for a webview-based file manager:

- The drag can name the *parent* extraction folder instead of its members, so a
  twenty-file extraction arrives as one folder.
- wry reads `CF_HDROP` inside `IDropTarget::Drop`, but Tauri queues the event to
  the app with `PostMessageW`. By the time the interface asks to paste, the
  source has often removed every path it just named.

Lyre snapshots the paths on the first `DragEnter`, which carries the same
`CF_HDROP` paths but arrives while the pointer is still over the window and the
archiver is still holding its files. `Drop` is a second chance for a platform
that sends no `Enter` paths, and the window stages what it can when it collects
the drop if both hooks are missed. A drag described by one extraction folder is
expanded into its members, and each member is held in a `lyre-drop-*` folder with
a hard link, so the bytes survive the archiver's cleanup without a second copy.
A rename and then a copy cover volumes that cannot link. The interface pastes
from those stable paths, and the staging folder is removed when the paste
finishes. A drag that leaves without a drop is released after a short grace
period, in case `leave` was sent just after `drop`.

**All platforms.** Detection is by path name and the system temp directory, not a
Windows API, and the snapshot is `std::fs` throughout. Linux and macOS archivers
that extract to a temp folder with one of the recognised names get the same
treatment; a normal file drag is never copied or moved through staging. Folders
left behind by a crashed run, or by a paste dismissed at the conflict prompt,
are swept at the next startup once they are six hours old. A folder already
handed to a paste is never reclaimed by a later drop, however long the copy
takes.

**Known limit.** An archiver using an unrecognised temp-folder name, or one that
deletes its files before the event loop receives `DragEnter`, still sees the old
behaviour. Closing the gap entirely means replacing wry's `IDropTarget`, which
would also replace every other native drop; that trade is not worth making.
