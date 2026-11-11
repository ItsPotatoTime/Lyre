//! Compiles the application icon into the executable.
//!
//! Windows takes a file's icon from a resource linked into it, so an icon that
//! only exists as a loose file gets no icon at all: Explorer shows the generic
//! one, and there is no handle for the window to put in its titlebar. Embedding
//! it at build time solves both, and the resource id is what `frame::install`
//! then loads.
//!
//! Only Windows has this step. macOS reads its icon from the bundle's
//! `Info.plist` and `Resources/icon.icns`, and Linux from the `.desktop` entry's
//! `Icon=`, both of which the packaging already writes.

fn main() {
    #[cfg(windows)]
    {
        // The `.rc` is beside this crate's manifest, so its icon path climbs one
        // level to the icon set the packaging uses.
        println!("cargo:rerun-if-changed=lyre.rc");
        println!("cargo:rerun-if-changed=../src-tauri/icons/icon.ico");
        // The result names the compiled resource; nothing needs it afterwards.
        let _ = embed_resource::compile("lyre.rc", embed_resource::NONE);
    }
}
