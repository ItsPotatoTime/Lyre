# Installing

## From a release

Every release carries one file per platform. Take the one for your system, they
all run the same program.

| Platform | File | What it is |
| --- | --- | --- |
| Linux | `Lyre_<version>_amd64.AppImage` | one file, no install, needs `webkit2gtk-4.1` |
| Linux | `Lyre_<version>_amd64.deb` | Debian, Ubuntu and derivatives |
| Linux | `Lyre-<version>-1.x86_64.rpm` | Fedora, openSUSE and derivatives |
| Windows | `Lyre_<version>_x64-setup.exe` | per-user installer (NSIS), no administrator needed |
| Windows | `Lyre_<version>_x64_en-US.msi` | system-wide installer (WiX) |
| macOS | `Lyre_<version>_universal.dmg` | one app for Apple silicon and Intel |

Nothing is code-signed yet, so both Windows and macOS will warn the first time.
That is expected, and the way past it is below.

## Linux

### AppImage

```bash
chmod +x Lyre_*.AppImage
./Lyre_*.AppImage
```

No installation and no root. It carries its own WebKit, which is why the file is
around 80 MB: the system's libraries are not needed. It needs glibc 2.35 or
newer, because it is built on Ubuntu 22.04, so a distribution from after early
2022 is fine.

### Debian, Ubuntu

```bash
sudo apt install ./Lyre_<version>_amd64.deb
```

`apt` resolves the dependency and registers the desktop entry, so folders can be
opened with Lyre from the desktop after that.

### Fedora, openSUSE

```bash
sudo dnf install ./Lyre-<version>-1.x86_64.rpm
```

### Arch, and anything with `paru` or `yay`

```bash
paru -S lyre-bin    # the release .deb, unpacked: no compiling
paru -S lyre        # built from the release tarball on your machine
```

`yay -S lyre-bin` and `yay -S lyre` are the same packages. The PKGBUILDs are in
[packaging/aur](../packaging/aur); see [publishing to the AUR](#publishing-to-the-aur)
for what has to happen before they exist there.

### Flatpak

Not published yet, because it has not been built and installed once. The manifest
is in [flatpak/app.lyre.files.yml](../flatpak/app.lyre.files.yml) and can be
built locally, which needs `flatpak-builder` and about a gigabyte of runtime:

```bash
flatpak install flathub org.gnome.Platform//44 org.gnome.Sdk//44 \
    org.freedesktop.Sdk.Extension.rust-stable//44
flatpak-builder --user --install --force-clean build-dir flatpak/app.lyre.files.yml
flatpak run app.lyre.files
```

## Windows

The `-setup.exe` installs under your user profile and needs no administrator
rights; the `.msi` installs for every user. Either way Windows SmartScreen will
stop the first launch because the binary is unsigned: **More info**, then **Run
anyway**.

## macOS

Open the `.dmg` and drag Lyre to Applications. It is unsigned and not notarised,
so the first launch needs either a right-click on the app icon and **Open**, or:

```bash
xattr -d com.apple.quarantine /Applications/Lyre.app
```

The app is universal, so it is native on both Apple silicon and Intel. macOS is
the least tested platform in this project; see the note at the top of the
[README](../README.md).

## From source

[building.md](building.md) covers the toolchain, `bun dev` for development and
`bun buildit` for a local release build.

## Cutting a release

1. Bump the version in three places, which must agree: `package.json`,
   `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`.
2. Commit, then tag and push the tag:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. [`.github/workflows/release.yml`](../.github/workflows/release.yml) builds
   Linux, Windows and macOS on GitHub's runners and attaches every installer to a
   **draft** release for that tag. The build decides nothing about publishing.
4. Read the draft, fix the notes, then publish it by hand.

Nothing is signed, so no secrets are needed. If signing is added later (an
Authenticode certificate for Windows, an Apple Developer ID and notarisation for
macOS), those become repository secrets and steps in the same workflow.

## Publishing to the AUR

An AUR package is a git repository on `aur.archlinux.org` holding a `PKGBUILD` and
a `.SRCINFO`. You push it once and update it as versions change. Two steps need
doing before the ones below: the GitHub repository has to be **public**, and the
release it points at has to exist.

1. Create an account at <https://aur.archlinux.org> and add your SSH public key
   under *My Account*.
2. Fill in the checksums, because the AUR does not accept `SKIP`:

   ```bash
   cd packaging/aur/lyre-bin && makepkg -g >> PKGBUILD   # or edit the line in
   cd ../lyre && makepkg -g >> PKGBUILD
   ```

3. Clone the (still empty) package repository and push the files:

   ```bash
   git clone ssh://aur@aur.archlinux.org/lyre-bin.git
   cd lyre-bin
   cp ../Lyre/packaging/aur/lyre-bin/PKGBUILD .
   makepkg --printsrcinfo > .SRCINFO
   git add PKGBUILD .SRCINFO
   git commit -m "Initial import"
   git push
   ```

   The first push creates the package. Repeat in a second directory for `lyre`.

4. After that, `paru -S lyre-bin` and `yay -S lyre-bin` work for everyone, and
   every release needs only a `pkgver` bump, a re-run of `makepkg -g` and a push.

A Flathub submission is a different kind of work: it builds from source with no
network, which means vendoring the Rust crates and building the interface with
the node SDK extension rather than bun. That is a task of its own.
