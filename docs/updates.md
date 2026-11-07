# Updates

Lyre updates from its own GitHub releases. **More → Check for updates** asks
`https://github.com/ItsPotatoTime/Lyre/releases/latest/download/latest.json`
whether there is a newer build, and if there is, offers it: the version, the
notes from that release, and an Install button that downloads it and hands it to
the platform's installer. The same check runs quietly once at launch — off the UI
thread, so the window opens first — and only interrupts when there is something
newer. **More → Check for updates at startup** turns that off (and back on), and
the update dialog has a **Don't check at startup** link; the choice is remembered.

## The signature is not optional

Every update is signed, and the build will not install one whose signature does
not match the public key compiled into it. That is what makes an update link safe
to follow: a build served from anywhere other than the project's releases fails
the check and is refused.

The key pair is a minisign key. It is generated once, for the life of the
project:

```bash
bunx tauri signer generate -w ~/.tauri/lyre.key
```

That writes the private key to `~/.tauri/lyre.key` and prints the public key. Two
things follow from it:

1. **The public key is compiled into the app** — `PUBKEY` in
   [rune-ui/src/updates.rs](../rune-ui/src/updates.rs), the same key as
   `plugins.updater.pubkey` in [tauri.conf.json](../src-tauri/tauri.conf.json). A
   build with an empty key can check and download but refuses to install, because
   an empty key matches nothing.
2. **The private key goes in the repository's secrets**, as
   `TAURI_SIGNING_PRIVATE_KEY` (the file's contents) and, if it was generated
   with a password, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Losing the private key means no existing installation can ever be updated again,
because they all trust the public key compiled into them. Keep it somewhere that
is backed up, and never commit it.

Until both secrets exist, the release workflow stops at the signing step. That is
deliberate: a release without signatures would be published and then silently
rejected by every installed copy.

## Making a release

The [release workflow](../.github/workflows/release.yml) ships the native crate
(`rune-ui/`): each platform builds `lyre-rune`, packages the format the updater
installs (NSIS setup on Windows, AppImage on Linux, a universal `.app.tar.gz` on
macOS), signs it with `tauri signer sign` using the key above, and uploads a
manifest fragment. A publish job merges the fragments into `latest.json` and
attaches everything to a draft release.

1. Bump the version in `rune-ui/Cargo.toml`. The updater compares the announced
   version against `CARGO_PKG_VERSION` compiled into the running build, so it must
   be higher than what people have for an offer to appear.
2. Add a `## [0.3.0]` section to `CHANGELOG.md`; it becomes the release notes.
3. Commit, then tag: `git tag v0.3.0 && git push origin v0.3.0`.
4. **Publish the draft.** `releases/latest/download/latest.json` only resolves to
   published releases, so an unpublished draft is invisible to every installed
   copy. This is the step that ships the update.

## What each platform installs

The updater hands the download to the platform's own installer, so the format
matches the update artifact for that platform: the NSIS setup on Windows, the
`.app.tar.gz` on macOS, the AppImage on Linux. A `.deb` or `.rpm` installation is
not updated this way: those follow the distribution's package manager, and the
app refuses with that sentence rather than replacing files a package manager
owns.

## When it cannot check

A build with no public key configured, or one whose connection to the release
page fails, reports the reason in the dialog instead of failing quietly. The
error for a missing key points back here.
