# Behaviour

What the interface does when it is asked to do something, and why it behaves that
way rather than the obvious way.

## Preview pane

The pane shows the focused file: a picture for images, video and PDFs, the first
lines of a text file, and the name, type, size and date for everything.

Thumbnails are made in the main process and handed over as PNG bytes, never as
the original file. That is deliberate: letting the webview decode a 50 megapixel
photograph to show it 300 pixels wide costs a couple of hundred megabytes, which
is the opposite of what the pane should do. Each thumbnail is scaled to 480
pixels, cached on disk under `~/.cache/lyre/thumbnails`, and keyed by the file's
identity, so selecting it again is a file read.

Decoding is delegated to tools a desktop already has: `pdftoppm` for PDFs,
`ffmpegthumbnailer` for video, `ffmpeg` or ImageMagick for images. When none is
installed the pane still shows the file's details rather than failing.

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
client, LocalSend or another file manager. The copy-or-move mode goes with it, so
the platform shows the same cursor the badge was showing. This uses
`tauri-plugin-drag`, whose Linux implementation is GTK-based and therefore
backend-agnostic; it has been type-checked for Windows but the
cross-application drag itself still wants a real try on each platform.

Drops *from* the operating system work and are always copies, since the source may
be on another volume or another machine.

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
