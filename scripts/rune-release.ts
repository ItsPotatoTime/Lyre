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

import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, basename } from "node:path";

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
 * The signer decodes the key file as base64 to get the key text, then decodes
 * the key line inside that text. A key line that has been through anything
 * URL-safe carries `_` or `-`, which standard base64 does not, and the second
 * decode is what fails: "Invalid symbol 95" is the underscore. Both markers are
 * unambiguous, so translating them is safe, and padding is restored per line
 * because base64url drops it. A key that needs nothing is returned untouched.
 */
function normaliseKey(secret: string): string {
  const trimmed = secret.trim();
  const outer = Buffer.from(trimmed, "base64");
  const text = outer.toString("utf8");
  if (!text || !/[-_]/.test(text)) return trimmed;

  const fixed = text
    .split("\n")
    .map((line) => {
      // The comment line is not base64 and is not part of the key.
      if (line.startsWith("untrusted comment") || !/[-_]/.test(line)) return line;
      const translated = line.replace(/-/g, "+").replace(/_/g, "/");
      return translated + "=".repeat((4 - (translated.length % 4)) % 4);
    })
    .join("\n");
  console.log("rune-release: key line was base64url; translated to standard base64");
  return Buffer.from(fixed, "utf8").toString("base64");
}

function sign(artifact: string, key: string): void {
  const secret = process.env.TAURI_SIGNING_PRIVATE_KEY;
  if (!secret) fail("TAURI_SIGNING_PRIVATE_KEY is not set; refusing to sign");
  const password = process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "";

  // A scratch file keeps the key off the command line (and out of the logs).
  const scratch = mkdtempSync(join(tmpdir(), "lyre-sign-"));
  const keyPath = join(scratch, "lyre.key");
  try {
    writeFileSync(keyPath, normaliseKey(secret), { mode: 0o600 });
    const cmd = ["run", "tauri", "signer", "sign", "--private-key", keyPath];
    if (password) cmd.push("--password", password);
    cmd.push(artifact);
    const result = Bun.spawnSync({ cmd: ["bun", ...cmd], stdout: "inherit", stderr: "inherit" });
    if ((result.exitCode ?? 1) !== 0) fail(`tauri signer sign exited with ${result.exitCode}`);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }

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