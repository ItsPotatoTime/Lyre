# Updates

Lyre updates from its own GitHub releases. **More → Check for updates** asks
`https://github.com/ItsPotatoTime/Lyre/releases/latest/download/latest.json`
whether there is a newer build, and if there is, offers it: the version, the
notes from that release, and an Install button that downloads it and hands it to
the platform's installer. Nothing happens at startup, because a file manager that
reaches the network before it shows a folder is slower than it looks.

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

1. **The public key goes in [tauri.conf.json](../src-tauri/tauri.conf.json)**,
   under `plugins.updater.pubkey`. Right now it is an empty string, which is why
   this build can check for an update and download it but will refuse to install
   one: an empty key matches nothing.
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

1. Bump the version in three places: `src-tauri/tauri.conf.json`,
   `src-tauri/Cargo.toml` and `package.json`. They have to agree, because the
   updater compares the announced version against the one compiled into the
   running build, and a mismatched bundle name is how a release fails to install.
2. Commit, then tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. The [release workflow](../.github/workflows/release.yml) builds every
   platform, signs the artifacts, and attaches them plus `latest.json` to a draft
   release.
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
