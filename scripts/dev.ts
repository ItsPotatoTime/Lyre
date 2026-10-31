#!/usr/bin/env bun
/**
 * `bun dev` — run the app against a development server on a free port.
 *
 * Vite and Tauri's development URL both used to be a hard-wired 1420, so
 * anything else on that port (a leftover server, a second checkout, another
 * app) failed the whole command with "Port 1420 is already in use" and left the
 * window closed. The port is chosen here instead, once, and both halves are
 * told the same number: Vite through LYRE_DEV_PORT, Tauri through --config,
 * which merges over `build.devUrl` in tauri.conf.json. Picking it in one place
 * is the point: a Vite that quietly moved on its own would leave the window
 * pointed at a server that is not there.
 *
 * Usage:
 *   bun dev                 start on 1420, or the next free port above it
 *   bun dev --port 1500     start on a given port, still moving up if taken
 *   bun dev --dry-run       print the port and the command, without running it
 */

import { createServer } from "node:net";
import { resolve } from "node:path";

/// The port Tauri's configuration default is written against.
const FIRST_PORT = 1420;
/// How far up to look before giving up rather than binding something surprising.
const ATTEMPTS = 20;

type Colour = (text: string) => string;
const dim: Colour = (text) => `\x1b[2m${text}\x1b[0m`;
const green: Colour = (text) => `\x1b[32m${text}\x1b[0m`;
const yellow: Colour = (text) => `\x1b[33m${text}\x1b[0m`;
const red: Colour = (text) => `\x1b[31m${text}\x1b[0m`;

const args = process.argv.slice(2);
const flag = (name: string) => args.includes(`--${name}`);
const option = (name: string) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : undefined;
};

function fail(message: string): never {
  console.error(`\n${red("failed:")} ${message}`);
  process.exit(1);
}

const dryRun = flag("dry-run");
const root = resolve(import.meta.dir, "..");
const requested = Number(option("port") ?? process.env.LYRE_DEV_PORT ?? FIRST_PORT);
if (!Number.isInteger(requested) || requested <= 0 || requested > 65535) {
  fail(`${option("port") ?? process.env.LYRE_DEV_PORT} is not a port number.`);
}

/** True when nothing is listening on `port` on the loopback interface. */
function free(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const probe = createServer();
    probe.unref();
    probe.once("error", () => resolve(false));
    probe.once("listening", () => probe.close(() => resolve(true)));
    probe.listen(port, "127.0.0.1");
  });
}

async function pickPort(): Promise<number> {
  for (let port = requested; port < requested + ATTEMPTS; port += 1) {
    if (await free(port)) return port;
  }
  return fail(
    `ports ${requested} to ${requested + ATTEMPTS - 1} are all in use. ` +
      "Close whatever is listening there, or pass --port.",
  );
}

const port = await pickPort();
if (port !== requested) {
  console.log(
    yellow(`port ${requested} is in use; using ${port}`),
  );
}
console.log(dim(`  ${green(`http://127.0.0.1:${port}`)}`));

// Tauri merges this over tauri.conf.json, so the window and the dev server
// cannot disagree, whatever the port turned out to be.
const command = ["bun", "tauri", "dev", "--config", JSON.stringify({ build: { devUrl: `http://127.0.0.1:${port}` } })];
const env = { ...process.env, LYRE_DEV_PORT: String(port) };

if (dryRun) {
  console.log(dim(`  would run: ${command.join(" ")}`));
  console.log(dim(`  with LYRE_DEV_PORT=${port}`));
  process.exit(0);
}

const dev = Bun.spawn({ cmd: command, cwd: root, env, stdout: "inherit", stderr: "inherit" });

// Ctrl+C reaches Tauri, which stops the dev server it started; this just makes
// sure the port probe and this process leave with the same code.
const code = await dev.exited;
process.exit(code);
