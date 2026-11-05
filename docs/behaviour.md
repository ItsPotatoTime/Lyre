# Behaviour

What the interface does when it is asked to do something, and why it behaves that
way rather than the obvious way.

## Preview pane

The pane shows the focused file: a picture for images, video and PDFs, the first
lines of a text file, and the name, type, size and date for everything.

It is a switch, not a fixture: the cross in its own header hides it, and the
preview button in the command bar, which stays on screen, brings it back. The
View menu carries the same item for anyone who looks there instead, and the
choice is remembered with the rest of the view. Hiding it gives its column back
to the list rather than leaving a gap.

Thumbnails are made in the main process and handed over as PNG bytes, never as
the original file. That is deliberate: letting the webview decode a 50 megapixel
photograph to show it 300 pixels wide costs a couple of hundred megabytes, which
is the opposite of what the pane should do. Each thumbnail is scaled to 480
pixels, cached on disk under the platform's cache directory
(`%LOCALAPPDATA%\lyre\thumbnails` on Windows, `~/.cache/lyre/thumbnails`
elsewhere), and keyed by the file's identity, so selecting it again is a file
read.

Pictures are decoded in process, so a machine nobody has prepared still shows
photographs. That is a deliberate exception to handing work to the desktop: a
fresh Windows account has neither `ffmpeg` nor ImageMagick, and its `convert.exe`
is the utility that turns a volume into NTFS, so the delegated path could never
have worked there. The formats are JPEG, PNG, GIF, WebP, BMP and TIFF; SVG, ICO,
AVIF and HEIC have no decoder here and fall through to the tools.

Video and PDF are asked of the system instead, through the thumbnail provider
Windows registers for the type, which is the one Explorer uses. That is why the
list of formats is as long as the machine's rather than as long as ours: a codec
installed for Explorer is a codec available here, and a machine without one shows
nothing in either. The request is made in two steps because the pane has to feel
instant. It first asks only for a thumbnail Windows already holds, which costs a
few milliseconds and no decoding at all, and that is the common case for anything
Explorer has shown. Only a miss goes to a worker thread, one at a time, with a
three second budget, and a refusal is remembered for the session so scrolling
past a folder of films does not pay for the same failed extraction on every row.
Elsewhere the old tools still serve: `ffmpegthumbnailer` and `pdftoppm`, and a
file nothing can be made for shows its details rather than failing.

Rows and tiles show the file's own picture too, made and cached the same way,
except that a file over 24 MB keeps its glyph in a row: decoding that much to
show it 16 pixels wide is not a trade worth making, and the pane still shows it at
full size. A row asks for a picture only once it has stayed on screen for a
moment, at most three are made at a time, and the finished picture is handed to
every row waiting for it. That last part is what makes the pictures appear in
place: storing one in the cache is invisible to a row that has already drawn
itself, so before that a row only ever saw pictures that were cached before it
mounted.

The pane is 300 px on a large window but gives way with the window down to 220 px,
and the details columns narrow to fit whatever the list has left. In a half-sized
window a long date ellipsizes rather than the Type and Size columns sliding under
the pane, where they could not be read or scrolled to. Dragging a divider trades
width with the other columns, so it still moves.

## Drag and drop

Dragging works onto a folder row, the empty space in a folder, any breadcrumb, any
tab, and the navigation rail. The pointer carries a badge saying **Move** or
**Copy**, and the zone under the pointer is outlined in the accent colour.

What it does depends on where it would land, before any key is pressed. Within one
volume a drag moves, because that is the point of dragging something to a folder
beside it. Across volumes it copies, because moving to a USB stick should not take
the file off the disk by accident and would mean a full rewrite anyway. Shift
inverts whichever is the default, and the badge follows as the pointer crosses
between volumes.

Moving an item into the folder it already lives in, or a folder into itself, is
refused rather than quietly turning into a rename.

Internal dragging is driven by pointer events, not HTML5 drag and drop, and that
is deliberate. Tauri's native drag handler is the only way to receive real
filesystem paths for a drop arriving from the operating system, and it replaces
WebView2's HTML5 handler on Windows, which would leave internal dragging working
everywhere except Windows.

**Dragging out of Lyre works.** Keep dragging past the window edge and the drag
becomes a real operating-system drag, so it can be dropped into Discord, a mail
client, LocalSend, Firefox or another file manager. The drag advertises both copy
and move, so a browser that can only copy and Explorer that can move each accept
it; the target decides. This uses `tauri-plugin-drag`, whose Linux implementation
is GTK-based and therefore backend-agnostic. The plugin module and the drag icon
are loaded when the window opens, because `DoDragDrop` has to start while the
button is still down; a release that beats the handoff abandons the drag instead
of dropping the files on the wrong window. The cross-application drag itself
still wants a real try on each platform.

Drops *from* the operating system work and are always copies, since the source may
be on another volume or another machine.

**Dragging out of an archive works.** 7-Zip and WinRAR do not hand over an archive:
they extract the selected members into a folder such as `%TEMP%\7zE9A2F`, name
those paths in `CF_HDROP`, and delete the folder as soon as the drag ends. 7-Zip
names only that empty folder until the button is released, then extracts inside
`IDropTarget::Drop`. By the time Tauri's queued event reaches the webview the
files are often gone, which is why twenty members sometimes arrived as one folder
and sometimes not at all. A patched wry calls back into Rust synchronously at
that exact moment, and every path under the system temp directory is held by
hard link in a folder Lyre owns. A lone `7zE…` or `Rar$…` container is expanded
into its members; the paste reads the stable paths and the staging folder is
released afterwards. The rule is generic, so a browser attachment, a mail client
or an archiver nobody has heard of gets the same treatment. Ordinary drops from a
file manager are left exactly as they were, and a source file is never renamed
away. [Platform detail](platforms.md#dragging-out-of-an-archive)

## Paste conflicts

A paste that would land on an existing name asks first: keep both, which numbers
the incoming item (`a.txt`, `a (2).txt`), or replace. Replacing merges a folder
rather than deleting it, so files that exist only in the destination survive and
only the files present in both are written over.

Delete moves to the freedesktop trash, which the desktop itself can put back.

## Being the first to know

Volumes appearing or disappearing are noticed while the window is open, so a stick
plugged in after startup shows up on its own. This PC lists volumes rather than
mounts: one card per device, labelled by its volume label with the device shown
underneath.

The folder on screen is watched too. A file saved, edited, renamed or deleted by
another program is on screen without pressing F5. One folder is watched at a
time — the one behind the current listing — and switching tabs or navigating
moves the watcher with it. Home and This PC have no single folder behind them,
so nothing is watched there; volumes have their own watcher above.

A name, a size and a modified time decide whether anything changed, so editing a
file in place counts even though the folder's own timestamp does not move.
Notifications arrive in bursts during a copy, so the window waits for a quiet
moment and re-reads once, with a cap so a long copy never postpones the refresh
forever. An inline rename keeps its row until the editor closes; a change that
arrives during it is applied afterwards. The folder's own operations still
refresh as before, and a manual F5 is still there.

Search results are not re-run automatically: a recursive walk is far too
expensive to repeat on every change. Starting a new query picks up the current
state of the disk. [Platform detail](platforms.md#watching-a-folder-for-changes)
