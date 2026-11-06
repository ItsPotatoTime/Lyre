#!/usr/bin/env bun
/**
 * Shared toolchain setup for the native Rune crate (`rune-ui/`).
 *
 * On Windows, Git for Windows puts a GNU coreutils `link` on PATH. Rust's
 * msvc toolchain invokes `link.exe` by name, so a build started from a plain
 * shell either fails with "link: extra operand" or links with the wrong tool.
 * `vcvars64.bat` only affects the shell that called it, and going through
 * cmd.exe from a script is fragile. Setting the three variables directly is
 * reproducible and works from bun on every platform.
 *
 * The MSVC and SDK versions are discovered rather than pinned: choco installs
 * Build Tools with a versioned toolset directory that changes over time.
 */

import { existsSync, readdirSync } from "node:fs";
import { join, resolve, basename } from "node:path";

export const root = resolve(import.meta.dir, "..");
export const runeManifest = join(root, "rune-ui", "Cargo.toml");

/** Highest-named version directory under `dir`, or undefined when absent. */
function latest(dir: string): string | undefined {
  if (!existsSync(dir)) return undefined;
  const versions = readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort()
    .reverse();
  return versions[0] ? join(dir, versions[0]) : undefined;
}

export function runeEnv(): Record<string, string> {
  const env: Record<string, string> = { ...process.env } as Record<string, string>;
  if (process.platform !== "win32") return env;

  const programFilesX86 = process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)";
  const vsRoot = join(programFilesX86, "Microsoft Visual Studio", "2022", "BuildTools");
  const msvc = latest(join(vsRoot, "VC", "Tools", "MSVC"));
  const sdkRoot = join(programFilesX86, "Windows Kits", "10");
  const sdkLib = latest(join(sdkRoot, "Lib"));
  if (!msvc || !sdkLib) {
    throw new Error(
      "MSVC or the Windows SDK was not found. Install Visual Studio 2022 Build " +
        "Tools with the 'Desktop development with C++' workload:\n" +
        "  choco install visualstudio2022buildtools -y\n" +
        "  choco install visualstudio2022-workload-vctools -y",
    );
  }

  const version = basename(sdkLib);
  env.PATH = [
    join(msvc, "bin", "Hostx64", "x64"),
    join(sdkRoot, "bin", version, "x64"),
    env.PATH ?? "",
  ].join(";");
  env.LIB = [
    join(msvc, "lib", "x64"),
    join(sdkRoot, "Lib", version, "um", "x64"),
    join(sdkRoot, "Lib", version, "ucrt", "x64"),
  ].join(";");
  env.INCLUDE = [
    join(msvc, "include"),
    join(sdkRoot, "Include", version, "um"),
    join(sdkRoot, "Include", version, "shared"),
    join(sdkRoot, "Include", version, "ucrt"),
    join(sdkRoot, "Include", version, "winrt"),
  ].join(";");
  return env;
}

/**
 * Run cargo against `rune-ui`. `cargoArgs` are cargo options (they go before
 * `--manifest-path`); `appArgs` are passed to the binary after `--`.
 */
export function runeCargo(cargoArgs: string[], appArgs: string[] = []): number {
  const cmd = ["cargo", ...cargoArgs, "--manifest-path", runeManifest];
  if (appArgs.length > 0) cmd.push("--", ...appArgs);
  const result = Bun.spawnSync({
    cmd,
    cwd: root,
    env: runeEnv(),
    stdout: "inherit",
    stderr: "inherit",
  });
  return result.exitCode ?? 1;
}
