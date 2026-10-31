# Platform notes

The three platforms answer "open this folder" and "open this file" differently
enough that the differences are not cosmetic. What follows is what Lyre does on
each, and where a platform makes something impossible it says so rather than
pretending.

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
managers do instead, and what Lyre does, is override the shell's default verb
for directories and drives. It is a per-user registry change under
`HKCU\Software\Classes`, so it needs no administrator rights, the stock `open`
verb is left untouched, and "Stop opening folders with Lyre" in the More menu
puts it back.

The honest caveats: this is a registry override rather than a supported API, so
anything that launches Explorer directly still gets Explorer, and a Windows
update can reset it. Restarting Explorer, or signing out, may be needed before
the change takes effect.

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
