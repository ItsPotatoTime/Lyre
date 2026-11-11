#!/usr/bin/env bun
/**
 * Release helpers for the native Rune crate (`rune-ui/`).
 *
 *   bun scripts/rune-release.ts sign <artifact> <platform-key>
 *   bun scripts/rune-release.ts manifest <version> <notes-file> <out> <fragment.json>...
 *
 * `sign` signs one installer with the project's minisign key and writes a
 * fragment `{ key, url, signature }` beside it. The key is the same one the old
 * Tauri release used, so installed copies keep trusting it: it is read from
 * TAURI_SIGNING_PRIVATE_KEY (and TAURI_SIGNING_PRIVATE_KEY_PASSWORD), written to
 * a scratch file so the secret never appears on a command line, and handed to
 * `tauri signer sign`. That writes `<artifact>.sig`; the manifest signature is
 * the base64 of that file's text, which is exactly what the updater's
 * minisign-verify step decodes.
 *
 * `manifest` merges the fragments into a Tauri-shaped `latest.json`
 * (`version`, `notes`, `pub_date`, `platforms{ key: { url, signature } }`). The
 * base platform keys (`windows-x86_64`, `linux-x86_64`, `darwin-*`) are the ones
 * the updater asks for; `windows-x86_64` points at the NSIS setup, not an msi,
 * because the client runs the download as an executable.
 *
 * Nothing here publishes: the workflow uploads the artifacts and the manifest to
 * a draft release, and a person publishes it.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { basename } from "node:path";

const args = process.argv.slice(2);
const command = args[0];

function fail(message: string): never {
  console.error(`rune-release: ${message}`);
  process.exit(1);
}

/** The release asset URL for a file, from the repository and tag in the env. */
function assetUrl(filename: string): string {
  const repo = process.env.GITHUB_REPOSITORY ?? "ItsPotatoTime/Lyre";
  const tag = process.env.TAG ?? process.env.GITHUB_REF_NAME;
  if (!tag) fail("no tag: set TAG or GITHUB_REF_NAME");
  return `https://github.com/${repo}/releases/download/${tag}/${filename}`;
}

/**
 * The shapes the stored key could be in, most likely first.
 *
 * Tauri's signer base64-decodes the key file to get the key text, then decodes
 * the key inside that text, so a key can be one encoding or two deep and either
 * layer can have come through something URL-safe. Rather than guess which, every
 * plausible reading is offered and the signer is left to accept one. The label is
 * logged, so the working one is known afterwards and the rest can go.
 */
function keyCandidates(secret: string): { label: string; content: string }[] {
  const trimmed = secret.trim();
  const candidates: { label: string; content: string }[] = [
    { label: "as stored", content: trimmed },
  ];

  // A key line that came through anything URL-safe carries `_` or `-`, which
  // standard base64 does not; both markers are unambiguous, so translating them
  // and restoring the padding base64url drops is safe.
  const translate = (text: string): string =>
    text
      .split("\n")
      .map((line) => {
        if (line.startsWith("untrusted comment") || !/[-_]/.test(line)) return line;
        const swapped = line.replace(/-/g, "+").replace(/_/g, "/");
        return swapped + "=".repeat((4 - (swapped.length % 4)) % 4);
      })
      .join("\n");
  const rewrap = (text: string): string =>
    Buffer.from(text, "utf8").toString("base64");

  candidates.push({ label: "stored key translated", content: translate(trimmed) });

  // The stored value may itself be base64 of the key text, one or two layers
  // deep. A decode that yields nothing printable is not a reading worth trying.
  const decoded = Buffer.from(trimmed, "base64").toString("utf8");
  if (/^[\x20-\x7e\r\n]+$/.test(decoded) && decoded.trim().length > 0) {
    candidates.push({ label: "decoded once", content: rewrap(decoded) });
    candidates.push({ label: "decoded once, translated", content: rewrap(translate(decoded)) });
  }
  return candidates;
}

function sign(artifact: string, key: string): void {
  const secret = process.env.TAURI_SIGNING_PRIVATE_KEY;
  if (!secret) fail("TAURI_SIGNING_PRIVATE_KEY is not set; refusing to sign");
  const password = process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "";

  // The key goes through the environment, never `--private-key`. In this CLI that
  // flag is decoded as the key itself rather than read as a file, which is why
  // passing a path produced "Invalid symbol 95, offset 29": the underscore in the
  // macOS temp path, and "symbol 58, offset 1" for the colon in the Windows one.
  let accepted = false;
  for (const candidate of keyCandidates(secret)) {
    const env: Record<string, string> = {};
    for (const [name, value] of Object.entries(process.env)) {
      if (value !== undefined) env[name] = value;
    }
    env.TAURI_SIGNING_PRIVATE_KEY = candidate.content;
    const cmd = ["run", "tauri", "signer", "sign"];
    if (password) cmd.push("--password", password);
    cmd.push(artifact);
    const result = Bun.spawnSync({
      cmd: ["bun", ...cmd],
      stdout: "inherit",
      stderr: "inherit",
      env,
    });
    if ((result.exitCode ?? 1) === 0) {
      console.log(`rune-release: the stored key is accepted ${candidate.label}`);
      accepted = true;
      break;
    }
  }
  if (!accepted) fail("no reading of TAURI_SIGNING_PRIVATE_KEY was accepted");

  const sigPath = `${artifact}.sig`;
  const signature = Buffer.from(readFileSync(sigPath)).toString("base64");
  const fragment = { key, url: assetUrl(basename(artifact)), signature };
  // The key is in the name so one artifact (a universal macOS build) can carry
  // several platform entries without the fragments overwriting each other.
  writeFileSync(`${artifact}.${key}.json`, JSON.stringify(fragment, null, 2) + "\n");
  console.log(`rune-release: signed ${basename(artifact)} as ${key}`);
}

function manifest(version: string, notesFile: string, out: string, fragments: string[]): void {
  const notes = readFileSync(notesFile, "utf8").trim();
  const platforms: Record<string, { url: string; signature: string }> = {};
  for (const path of fragments) {
    const fragment = JSON.parse(readFileSync(path, "utf8")) as {
      key: string;
      url: string;
      signature: string;
    };
    if (!fragment.key || !fragment.url || !fragment.signature) {
      fail(`fragment ${path} is missing key/url/signature`);
    }
    platforms[fragment.key] = { url: fragment.url, signature: fragment.signature };
  }
  if (Object.keys(platforms).length === 0) fail("no platform fragments to merge");
  const document = {
    version,
    notes,
    pub_date: new Date().toISOString(),
    platforms,
  };
  writeFileSync(out, JSON.stringify(document, null, 2) + "\n");
  console.log(`rune-release: wrote ${out} with ${Object.keys(platforms).length} platform(s)`);
}

switch (command) {
  case "sign":
    if (args.length !== 3) fail("usage: sign <artifact> <platform-key>");
    sign(args[1], args[2]);
    break;
  case "manifest":
    if (args.length < 5) fail("usage: manifest <version> <notes-file> <out> <fragment.json>...");
    manifest(args[1], args[2], args[3], args.slice(4));
    break;
  default:
    fail(`unknown command ${command ?? "(none)"}; expected sign or manifest`);
}