#!/usr/bin/env bun
/**
 * Shared toolchain setup for the native Rune crate (`rune-ui/`).
 *
 * Two Windows traps, both learned the hard way:
 *
 * 1. A terminal opened before Rust was installed keeps its old PATH, so
 *    `cargo` is missing even though rustup put the shim in `~/.cargo/bin`.
 *    Resolve that shim explicitly instead of asking the shell to find it.
 * 2. Git for Windows puts a GNU coreutils `link` on PATH. Rust's msvc
 *    toolchain invokes `link.exe` by name, so a build started from a plain
 *    shell fails with "link: extra operand" or links with the wrong tool.
 *    `vcvars64.bat` only affects the shell that called it; setting PATH, LIB
 *    and INCLUDE directly is reproducible and needs no cmd.exe round trip.
 *
 * MSVC and SDK versions are discovered rather than pinned: choco installs
 * Build Tools into a versioned toolset directory that changes over time.
 */

import { existsSync, readdirSync } from "node:fs";
import { join, resolve, basename, dirname, delimiter } from "node:path";

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

function home(): string {
  return process.env.USERPROFILE ?? process.env.HOME ?? "";
}

/**
 * Absolute path to cargo when it can be found, otherwise the bare command as a
 * last resort. `CARGO` wins so a non-standard toolchain is still usable.
 */
export function cargoBin(): string {
  const explicit = process.env.CARGO;
  if (explicit && existsSync(explicit)) return explicit;
  const cargoHome = process.env.CARGO_HOME ?? (home() ? join(home(), ".cargo") : "");
  if (cargoHome) {
    const exe = process.platform === "win32" ? "cargo.exe" : "cargo";
    const candidate = join(cargoHome, "bin", exe);
    if (existsSync(candidate)) return candidate;
  }
  return "cargo";
}

function cargoDir(): string {
  const bin = cargoBin();
  return bin === "cargo" ? "" : dirname(bin);
}

/**
 * Windows keeps the original casing of environment variable names. Setting
 * `PATH` when `process.env` already has `Path` would hand the child process two
 * competing variables, so keep exactly one key for each variable we own.
 */
function envKey(name: string): string {
  const lower = name.toLowerCase();
  return Object.keys(process.env).find((key) => key.toLowerCase() === lower) ?? name;
}

export function runeEnv(): Record<string, string> {
  const isWindows = process.platform === "win32";
  const pathKey = envKey("PATH");
  const base = process.env[pathKey] ?? "";
  const owned = new Set(isWindows ? ["path", "lib", "include"] : ["path"]);
  const env: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value === undefined || owned.has(name.toLowerCase())) continue;
    env[name] = value;
  }

  if (!isWindows) {
    env[pathKey] = [cargoDir(), base].filter(Boolean).join(delimiter);
    return env;
  }

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
  env[pathKey] = [
    join(msvc, "bin", "Hostx64", "x64"),
    join(sdkRoot, "bin", version, "x64"),
    cargoDir(),
    base,
  ]
    .filter(Boolean)
    .join(delimiter);
  env[envKey("LIB")] = [
    join(msvc, "lib", "x64"),
    join(sdkRoot, "Lib", version, "um", "x64"),
    join(sdkRoot, "Lib", version, "ucrt", "x64"),
  ].join(delimiter);
  env[envKey("INCLUDE")] = [
    join(msvc, "include"),
    join(sdkRoot, "Include", version, "um"),
    join(sdkRoot, "Include", version, "shared"),
    join(sdkRoot, "Include", version, "ucrt"),
    join(sdkRoot, "Include", version, "winrt"),
  ].join(delimiter);
  return env;
}

/**
 * Run cargo against `rune-ui`. `cargoArgs` are cargo options (they go before
 * `--manifest-path`); `appArgs` are passed to the binary after `--`.
 */
export function runeCargo(cargoArgs: string[], appArgs: string[] = []): number {
  const cmd = [cargoBin(), ...cargoArgs, "--manifest-path", runeManifest];
  if (appArgs.length > 0) cmd.push("--", ...appArgs);
  try {
    const result = Bun.spawnSync({
      cmd,
      cwd: root,
      env: runeEnv(),
      stdout: "inherit",
      stderr: "inherit",
    });
    return result.exitCode ?? 1;
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    console.error(`failed: could not start cargo (${detail})`);
    console.error(
      "  Install Rust from https://rustup.rs and open a new terminal, or set\n" +
        "  CARGO to the full path of cargo.exe.",
    );
    return 1;
  }
}
