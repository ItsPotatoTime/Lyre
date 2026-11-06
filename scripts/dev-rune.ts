#!/usr/bin/env bun
/**
 * `bun dev:rune` — open the native Rune window.
 *
 * `bun dev` starts the Tauri/React interface. This command starts the
 * experimental native crate on `feat/rune-native-ui` instead. Arguments after
 * the options are forwarded to the Rune binary, so headless verification works
 * through the same command:
 *
 *   bun dev:rune
 *   bun dev:rune --release
 *   bun dev:rune -- --snap out.png --scene downloads --size 1076x600 --scale 1
 *   LYRE_RUNE_PATH=... bun dev:rune
 */

import { runeCargo } from "./rune-toolchain.ts";

const args = process.argv.slice(2);
const release = args.includes("--release");
const appArgs = args.filter((arg) => arg !== "--release");
const cargoArgs = release ? ["run", "--release"] : ["run"];

console.log("lyre: starting the native Rune window");
process.exit(runeCargo(cargoArgs, appArgs));
