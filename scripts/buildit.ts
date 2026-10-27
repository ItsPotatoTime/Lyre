#!/usr/bin/env bun
/**
 * `bun buildit` — build Lyre for whichever operating system is running it.
 *
 * Linux and macOS build in place. Windows is different enough to deserve its
 * own path, and the reasons are worth recording because each one cost someone
 * an afternoon:
 *
 * 1. **A Windows build must not share `src-tauri/target` with Linux.** The two
 *    produce different object files for the same paths; mixing them produces
 *    link errors that look like source bugs. On Windows the source is copied to
 *    a folder on the VM's own drive and built there.
 * 2. **MSVC must be activated in the same shell as cargo.** `vcvars64.bat` sets
 *    INCLUDE, LIB and PATH; spawning cargo from a shell that never ran it fails
 *    with "link.exe not found" or, worse, finds Git's GNU `link.exe` first.
 * 3. **A stale build script survives on a shared disk.** File timestamps there
 *    are coarse enough that cargo's freshness check can miss an edit, leaving a
 *    `build-script-build.exe` from a previous tree. `--clean` removes it.
 *
 * Usage:
 *   bun buildit                build for this OS, release
 *   bun buildit --debug        debug build (faster, for testing the pipeline)
 *   bun buildit --clean        throw away the build cache first
 *   bun buildit --dry-run      print the commands without running them
 *   bun buildit --bundles nsis,msi   override the installer formats
 */

import { existsSync } from "node:fs";
import { mkdirSync, copyFileSync, readdirSync, rmSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve, basename } from "node:path";

type Platform = "windows" | "linux" | "macos";

const args = process.argv.slice(2);
const flag = (name: string) => args.includes(`--${name}`);
const option = (name: string) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : undefined;
};

const dryRun = flag("dry-run");
const clean = flag("clean");
const debug = flag("debug");
const root = resolve(import.meta.dir, "..");

const detected: Platform =
  process.platform === "win32"
    ? "windows"
    : process.platform === "darwin"
      ? "macos"
      : "linux";

// A dry run may pretend to be another platform, so the Windows path can be
// reviewed from Linux without building anything. Real builds always use the
// platform they are running on.
const forced = args.includes("--dry-run") ? process.env.LYRE_BUILD_PLATFORM : undefined;
const platform: Platform = (forced as Platform | undefined) ?? detected;

/**
 * Installer formats that exist on this machine, not on this platform.
 *
 * Tauri can bundle deb, rpm and AppImage on Linux, but each needs a tool that
 * may not be installed: `dpkg-deb`, `rpmbuild`, `appimagetool`. Asking for a
 * format whose tool is missing fails at the very end of a long build, so the
 * list is chosen from what is actually present. With none of them the binary is
 * still built, and the script says where it is.
 */
function linuxBundles(): string {
  const available: string[] = [];
  if (toolExists("dpkg-deb")) available.push("deb");
  if (toolExists("rpmbuild")) available.push("rpm");
  if (available.length === 0) return "";
  return available.join(",");
}

function toolExists(command: string): boolean {
  const probe = Bun.spawnSync({
    cmd: [process.platform === "win32" ? "where" : "which", command],
    stdout: "ignore",
    stderr: "ignore",
  });
  return probe.exitCode === 0;
}

const bundles =
  option("bundles") ??
  (platform === "windows"
    ? "nsis,msi"
    : platform === "macos"
      ? "dmg,app"
      : linuxBundles());

const colour = {
  dim: (text: string) => `\x1b[2m${text}\x1b[0m`,
  bold: (text: string) => `\x1b[1m${text}\x1b[0m`,
  green: (text: string) => `\x1b[32m${text}\x1b[0m`,
  red: (text: string) => `\x1b[31m${text}\x1b[0m`,
  yellow: (text: string) => `\x1b[33m${text}\x1b[0m`,
};

function stage(number: number, text: string) {
  console.log(`\n${colour.bold(`[${number}]`)} ${text}`);
}

function fail(message: string): never {
  console.error(`\n${colour.red("failed:")} ${message}`);
  process.exit(1);
}

/**
 * Run a command in the foreground, inheriting stdio.
 *
 * `okUpTo` exists for robocopy, which reports *how much it did* through its
 * exit code: 1 means files were copied, and anything below 8 is success. Only
 * 8 and above are real failures.
 */
function run(
  command: string,
  commandArgs: string[],
  options: { cwd?: string; env?: Record<string, string>; okUpTo?: number } = {},
) {
  const line = [command, ...commandArgs].join(" ");
  if (dryRun) {
    console.log(colour.dim(`  would run: ${line}`));
    return 0;
  }
  console.log(colour.dim(`  $ ${line}`));
  const result = Bun.spawnSync({
    cmd: [command, ...commandArgs],
    cwd: options.cwd ?? root,
    env: { ...process.env, ...options.env },
    stdout: "inherit",
    stderr: "inherit",
  });
  const code = result.exitCode ?? 0;
  const limit = options.okUpTo ?? 0;
  if (code > limit) {
    fail(
      options.okUpTo !== undefined
        ? `${line} exited with ${code}, above the ${limit} that counts as success`
        : `${line} exited with ${code}`,
    );
  }
  return code;
}

/** Run a command and capture its trimmed stdout. */
function capture(command: string, commandArgs: string[]): string {
  if (dryRun) return "";
  try {
    const result = Bun.spawnSync({
      cmd: [command, ...commandArgs],
      stdout: "pipe",
      stderr: "ignore",
    });
    return result.stdout.toString().trim();
  } catch {
    return "";
  }
}

function has(command: string): boolean {
  if (dryRun) return true;
  return capture(platform === "windows" ? "where" : "which", [command]).length > 0;
}

function requireTool(command: string, hint: string) {
  if (!has(command)) fail(`${command} is not on PATH. ${hint}`);
}

// ---------------------------------------------------------------------------
// Shared checks
// ---------------------------------------------------------------------------
stage(1, `Building Lyre for ${platform}${debug ? " (debug)" : " (release)"}`);

if (!existsSync(join(root, "package.json")) || !existsSync(join(root, "src-tauri", "Cargo.toml"))) {
  fail("run this from the project root, or the checkout is incomplete.");
}
requireTool("cargo", "Install Rust from https://rustup.rs.");
requireTool("bun", "Install Bun from https://bun.sh.");

// Dependencies are platform specific: a node_modules installed on Linux holds
// binaries a Windows run cannot use, and the other way round.
const stampPath = join(root, ".bun-platform");
const previous = existsSync(stampPath) ? readFileSync(stampPath, "utf8").trim() : null;
const mismatch = previous !== null && previous !== platform;
if (mismatch) {
  console.log(
    colour.yellow(
      `\n  dependencies were last installed for ${previous}; reinstalling for ${platform}`,
    ),
  );
}

// ---------------------------------------------------------------------------
// Windows: local copy, MSVC, then build
// ---------------------------------------------------------------------------
if (platform === "windows") {
  const localRoot = dryRun && forced
    ? "%USERPROFILE%\\lyre-winbuild"
    : join(process.env.USERPROFILE ?? root, "lyre-winbuild");
  const outDir = join(root, "build");

  stage(2, `Copying the source to ${localRoot}`);
  console.log(colour.dim("  A Windows build must not share target/ with Linux."));
  // 0 to 7 all mean success for robocopy: 1 is "files were copied", which is
  // the normal case and not an error.
  const copyCode = run(
    "robocopy",
    [
      root,
      localRoot,
      "/MIR",
      "/NFL",
      "/NDL",
      "/NJH",
      "/NJS",
      "/XD",
      "node_modules",
      "target",
      ".git",
      ".cairn",
      "dist",
      "build",
      ".reference",
    ],
    { okUpTo: 7 },
  );
  if (!dryRun) {
    if (copyCode >= 8) fail(`robocopy failed with ${copyCode}`);
    if (!existsSync(join(localRoot, "package.json"))) {
      fail(`the copy is missing package.json; is ${root} readable?`);
    }
  }

  stage(3, "Locating Visual Studio and activating MSVC");
  const vswhere = [
    join(process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)", "Microsoft Visual Studio/Installer/vswhere.exe"),
    join(process.env.ProgramFiles ?? "C:\\Program Files", "Microsoft Visual Studio/Installer/vswhere.exe"),
  ].find((candidate) => existsSync(candidate));
  if (!vswhere && !dryRun) {
    fail(
      "vswhere.exe was not found. Install Visual Studio Build Tools with the\n" +
        "  'Desktop development with C++' workload.",
    );
  }
  const vsInstall = capture(vswhere, [
    "-latest",
    "-products",
    "*",
    "-requires",
    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    "-property",
    "installationPath",
  ]);
  if (!dryRun && !vsInstall) {
    fail("no Visual Studio install has the C++ build tools.");
  }
  const vcvars = join(vsInstall || "C:\\VS", "VC", "Auxiliary", "Build", "vcvars64.bat");
  if (!dryRun && !existsSync(vcvars)) fail(`vcvars64.bat is missing at ${vcvars}`);

  stage(4, "Installing dependencies in the Windows copy");
  // --force because the local node_modules is reused between runs while the
  // lockfile keeps arriving from the other OS.
  run("bun", ["install", "--force"], { cwd: localRoot });

  stage(5, "Building the Rust crate and the installers");
  if (clean) run("cargo", ["clean"], { cwd: join(localRoot, "src-tauri") });

  // vcvars64 only affects the shell that runs it, so the build has to happen in
  // a cmd session that called it. That session is a generated .cmd file rather
  // than a nested-quoted one-liner: cmd.exe's quoting is the usual source of
  // mysterious failures, and a file can be read and rerun by hand.
  const tauriArgs = [
    "tauri",
    "build",
    ...(bundles ? ["--bundles", bundles] : ["--no-bundle"]),
    ...(debug ? ["--debug"] : []),
  ];
  const scriptPath = join(localRoot, "lyre-build.cmd");
  const script = [
    "@echo off",
    "setlocal",
    `call "${vcvars}"`,
    "if errorlevel 1 exit /b 2",
    // Git for Windows puts a GNU link.exe on PATH; if it wins, rustc fails with
    // GNU-style link errors that look like source problems.
    "where link | findstr /i \"\\Git\\\" >nul",
    "if errorlevel 1 goto build",
    "echo [error] Git's link.exe is shadowing MSVC's. Check PATH order.",
    "exit /b 3",
    ":build",
    `bun run ${tauriArgs.join(" ")}`,
    "exit /b %errorlevel%",
    "",
  ].join("\r\n");

  if (dryRun) {
    console.log(colour.dim(`  would write ${scriptPath}:`));
    for (const line of script.split("\r\n")) {
      if (line) console.log(colour.dim(`    | ${line}`));
    }
    console.log(colour.dim(`  would run: cmd /d /s /c "${scriptPath}"`));
  } else {
    writeFileSync(scriptPath, script);
    run("cmd", ["/d", "/s", "/c", scriptPath], { cwd: localRoot });
  }

  stage(6, `Collecting installers into ${outDir}`);
  const bundleRoot = join(localRoot, "src-tauri", "target", debug ? "debug" : "release", "bundle");
  if (dryRun) {
    console.log(colour.dim(`  would copy ${bundleRoot}/**/*.{exe,msi} to ${outDir}`));
  } else {
    mkdirSync(outDir, { recursive: true });
    let copied = 0;
    for (const kind of ["nsis", "msi"]) {
      const dir = join(bundleRoot, kind);
      if (!existsSync(dir)) continue;
      for (const name of readdirSync(dir)) {
        if (!/\.(exe|msi)$/i.test(name)) continue;
        copyFileSync(join(dir, name), join(outDir, name));
        copied += 1;
        console.log(`  ${colour.green("+")} build/${name}`);
      }
    }
    if (copied === 0) {
      fail(`no installers were produced in ${bundleRoot}.`);
    }
  }
} else {
  // -------------------------------------------------------------------------
  // Linux and macOS: build in place
  // -------------------------------------------------------------------------
  stage(2, "Installing dependencies");
  run("bun", ["install", ...(clean ? ["--force"] : [])]);

  if (platform === "linux") {
    const sysDeps = capture("pkg-config", ["--exists", "webkit2gtk-4.1"]);
    if (!dryRun && sysDeps === "" && !has("pkg-config")) {
      console.log(
        colour.yellow(
          "  pkg-config is missing; the build needs webkit2gtk-4.1, gtk3 and libsoup3 development packages.",
        ),
      );
    }
  }

  stage(3, "Building the Rust crate and the installers");
  if (clean) run("cargo", ["clean"], { cwd: join(root, "src-tauri") });
  run("bun", [
    "tauri",
    "build",
    ...(bundles ? ["--bundles", bundles] : ["--no-bundle"]),
    ...(debug ? ["--debug"] : []),
  ]);

  stage(4, bundles ? "Installers" : "Executable");
  if (!bundles && !dryRun) {
    console.log(
      colour.yellow(
        "  no packaging tool found (dpkg-deb, rpmbuild).\n" +
          "  Install one to get an installer, or use the binary below.",
      ),
    );
    const binary = join(root, "src-tauri", "target", debug ? "debug" : "release", "lyre");
    if (existsSync(binary)) console.log(`  ${colour.green("+")} src-tauri/target/${debug ? "debug" : "release"}/lyre`);
  }
  const bundleRoot = join(root, "src-tauri", "target", debug ? "debug" : "release", "bundle");
  if (dryRun) {
    console.log(colour.dim(`  would list ${bundleRoot}/*`));
  } else if (existsSync(bundleRoot)) {
    for (const kind of readdirSync(bundleRoot)) {
      const dir = join(bundleRoot, kind);
      let names: string[] = [];
      try {
        names = readdirSync(dir);
      } catch {
        continue;
      }
      for (const name of names) {
        console.log(`  ${colour.green("+")} ${join("src-tauri/target/bundle", kind, name)}`);
      }
    }
  }
}

if (!dryRun) {
  writeFileSync(stampPath, platform);
  console.log(
    `\n${colour.green("done")} ${basename(root)} · ${platform} · ${bundles || "no installer"}`,
  );
}
