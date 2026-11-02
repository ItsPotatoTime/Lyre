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
new process rather than reusing the running window, only the first instance hooks
the key, which it claims with a named mutex; otherwise each press would multiply
the number of windows. Win+Shift+E, Win+Ctrl+E and Win+Alt+E are left alone.

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
