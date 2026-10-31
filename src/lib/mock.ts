/**
 * A synthetic filesystem for design work and for running the interface in a
 * plain browser, where there is no Rust core behind it. Content is
 * deterministic and deliberately realistic: the layouts have to be judged
 * against names, date spread, and file sizes that actually occur.
 *
 * This module is loaded lazily, so it costs nothing in the packaged app.
 */

import type { Entry, EntryKind, Hit, PathInfo, Place } from "@/types";
import type { ScanHandlers, ScanHandle, SearchHandlers } from "./ipc";

const HOME = "/home/you";

export const mockHome = () => HOME;

/**
 * A stand-in thumbnail for the browser.
 *
 * The real one is a PNG made by the desktop app, and the browser cannot run
 * ffmpeg or pdftoppm. Without something here, the row and tile thumbnails exist
 * in the code and never in the page, so how they look and when they appear could
 * only be judged in the packaged build. This paints a small landscape in the
 * file's own colour instead, derived from its path so a picture looks the same
 * on every reload.
 */
export function mockThumbnail(path: string): string {
  let hash = 0;
  for (let i = 0; i < path.length; i += 1) {
    hash = (hash * 31 + path.charCodeAt(i)) | 0;
  }
  const hue = Math.abs(hash) % 360;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">` +
    `<rect width="64" height="64" fill="hsl(${hue} 42% 54%)"/>` +
    `<path d="M0 64 L18 32 L32 48 L45 37 L64 56 L64 64 Z" fill="hsl(${hue} 38% 33%)"/>` +
    `</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export function mockPlaces(): Place[] {
  const folders: [string, string, string][] = [
    ["Desktop", "desktop", "folder"],
    ["Downloads", "download", "folder"],
    ["Documents", "document", "folder"],
    ["Pictures", "image", "folder"],
    ["Music", "audio", "folder"],
    ["Videos", "video", "folder"],
  ];
  return [
    {
      name: "you",
      path: HOME,
      kind: "folder" as const,
      icon: "home",
      detail: null,
      total: 0,
      free: 0,
    },
    ...folders.map(([name, icon, kind]) => ({
      name,
      path: `${HOME}/${name}`,
      kind: kind as Place["kind"],
      icon,
      detail: null,
      total: 0,
      free: 0,
    })),
    {
      name: "Local Disk",
      path: "/",
      kind: "drive" as const,
      icon: "drive",
      detail: "C:",
      total: 476_900_000_000,
      free: 182_400_000_000,
    },
    {
      name: "Seagate Backup",
      path: "/media/you/Seagate Backup",
      kind: "removable" as const,
      icon: "drive",
      detail: "/dev/sdb1",
      total: 1_800_000_000_000,
      free: 1_200_000_000_000,
    },
  ];
}

export function mockPathInfo(path: string): PathInfo {
  const segments: { name: string; path: string }[] = [{ name: "/", path: "/" }];
  let built = "";
  for (const part of path.split("/").filter(Boolean)) {
    built += `/${part}`;
    segments.push({ name: part, path: built });
  }
  const name = segments[segments.length - 1]?.name ?? "/";
  const parent =
    segments.length > 1 ? segments[segments.length - 2]!.path : null;
  return {
    path,
    name,
    parent,
    segments,
    exists: true,
    isDir: true,
  };
}

export async function mockScan(
  path: string,
  showHidden: boolean,
  showProtected: boolean,
  handlers: ScanHandlers,
): Promise<ScanHandle> {
  const started = performance.now();
  const all = listing(path);
  const entries = all.filter((e) => visible(e, showHidden, showProtected));
  let cancelled = false;

  // Stream the way the real core does: a few hundred rows per frame so the
  // first paint and the climbing count can both be seen.
  const batchSize = 320;
  let sent = 0;
  const tick = () => {
    if (cancelled) return;
    const next = entries.slice(sent, sent + batchSize);
    sent += next.length;
    if (next.length > 0) handlers.onBatch(next, sent);
    if (sent < entries.length) {
      setTimeout(tick, 8);
    } else {
      handlers.onDone(sent, Math.round(performance.now() - started));
    }
  };
  setTimeout(tick, 4);

  return {
    cancel: () => {
      cancelled = true;
    },
  };
}

function listing(path: string): Entry[] {
  const seed = hash(path);
  const random = mulberry32(seed);
  const now = Date.now();

  const children = CHILDREN[path];
  const entries: Entry[] = [];
  // A real directory cannot hold two entries with the same path, and the row
  // components key on it. The generator can produce duplicates, so it dedupes.
  const taken = new Set<string>();

  if (children) {
    for (const [name, kind] of children) {
      const unique = uniqueName(name, taken);
      entries.push(makeEntry(path, unique, kind, random, now));
    }
  }

  const filler = FILLERS[path];
  if (filler) {
    const { dirs, files, pools } = filler;
    for (let i = 0; i < dirs; i += 1) {
      const name = uniqueName(
        `${capitalise(pick(DIR_WORDS, random))} ${2019 + Math.floor(random() * 6)}`,
        taken,
      );
      entries.push(makeEntry(path, name, "folder", random, now));
    }
    for (let i = 0; i < files; i += 1) {
      const pool = pick(pools, random);
      const base = pick(pool, random);
      const suffix = random() < 0.45 ? ` ${String(Math.floor(random() * 90) + 1).padStart(2, "0")}` : "";
      const ext = pick(EXTENSIONS, random);
      const name = uniqueName(`${base}${suffix}.${ext}`, taken);
      entries.push(makeEntry(path, name, null, random, now));
    }
  }

  return entries;
}

function uniqueName(name: string, taken: Set<string>): string {
  if (!taken.has(name)) {
    taken.add(name);
    return name;
  }
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  for (let n = 2; ; n += 1) {
    const candidate = `${stem} (${n})${ext}`;
    if (!taken.has(candidate)) {
      taken.add(candidate);
      return candidate;
    }
  }
}

/**
 * A stand-in for the system attribute the desktop uses to keep its own entries
 * out of a listing. Unix has no such mark, so these names exist only to make the
 * browser show the state the packaged app shows on Windows.
 */
const PROTECTED = new Set(["lost+found", "snap"]);

/** Whether a row belongs in the list at the current settings. */
function visible(entry: Entry, showHidden: boolean, showProtected: boolean): boolean {
  return (showHidden || !entry.hidden) && (showProtected || !entry.protected);
}

function makeEntry(
  parent: string,
  name: string,
  kind: EntryKind | "folder" | null,
  random: () => number,
  now: number,
): Entry {
  const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
  const isDir = kind === "folder" || ext === "";
  const resolved: EntryKind = isDir ? "folder" : extKind(ext);
  // Spread dates across the last two years, weighted toward recent.
  const age = Math.pow(random(), 2.2) * 730;
  const modifiedMs = now - age * 86_400_000 - Math.floor(random() * 86_400_000);
  return {
    name,
    path: parent === "/" ? `/${name}` : `${parent}/${name}`,
    isDir,
    isSymlink: false,
    size: isDir ? 0 : Math.floor(Math.pow(random(), 3) * 240_000_000) + 512,
    modifiedMs,
    kind: resolved,
    ext,
    hidden: name.startsWith("."),
    protected: PROTECTED.has(name),
    readable: true,
  };
}

const CHILDREN: Record<string, [string, EntryKind | "folder"][]> = {
  "/": [
    ["lost+found", "folder"],
    ["snap", "folder"],
    ["home", "folder"],
    ["media", "folder"],
    ["usr", "folder"],
    ["var", "folder"],
    ["etc", "folder"],
    ["opt", "folder"],
  ],
  "/home/you": [
    ["Desktop", "folder"],
    ["Documents", "folder"],
    ["Downloads", "folder"],
    ["Music", "folder"],
    ["Pictures", "folder"],
    ["Videos", "folder"],
    [".config", "folder"],
    [".ssh", "folder"],
    [".bashrc", "text"],
    ["notes.md", "text"],
    ["todo.txt", "text"],
  ],
  "/home/you/Desktop": [
    ["Lyre", "folder"],
    ["Screenshots", "folder"],
    ["project-brief.pdf", "pdf"],
    ["invoice-2026-014.pdf", "pdf"],
    ["logo-final-v3.svg", "image"],
    ["shortcut.desktop", "shortcut"],
    ["scratch.txt", "text"],
  ],
  "/home/you/Documents": [
    ["Lyre", "folder"],
    ["Contracts", "folder"],
    ["Invoices", "folder"],
    ["Design", "folder"],
    ["Q4 planning.docx", "document"],
    ["Budget 2026.xlsx", "spreadsheet"],
    ["Roadmap.pptx", "presentation"],
    ["README.md", "text"],
    ["archive.zip", "archive"],
  ],
  "/home/you/Documents/Lyre": [
    ["src", "folder"],
    ["src-tauri", "folder"],
    ["public", "folder"],
    ["package.json", "code"],
    ["Cargo.toml", "code"],
    ["README.md", "text"],
    ["LICENSE", "text"],
    ["vite.config.ts", "code"],
    ["tsconfig.json", "code"],
    ["index.html", "code"],
    [".gitignore", "code"],
  ],
  "/home/you/Downloads": [
    ["installers", "folder"],
    ["rustup-init.sh", "code"],
    ["ubuntu-26.04-desktop-amd64.iso", "archive"],
    ["Figma-Setup.exe", "executable"],
    ["font-inter-4.1.zip", "archive"],
    ["receipt.pdf", "pdf"],
    ["screenshot 2026-10-21.png", "image"],
    ["data-export.csv", "spreadsheet"],
    ["podcast-ep-118.mp3", "audio"],
  ],
  "/home/you/Music": [
    ["Ambient", "folder"],
    ["Jazz", "folder"],
    ["field-recording-01.flac", "audio"],
    ["night-drive.mp3", "audio"],
    ["strings.aiff", "audio"],
  ],
  "/home/you/Videos": [
    ["Clips", "folder"],
    ["screen-recording-2026-10-14.mp4", "video"],
    ["studio-session.mkv", "video"],
    ["demo.mov", "video"],
  ],
};

const FILLERS: Record<
  string,
  { dirs: number; files: number; pools: string[][] }
> = {
  "/home/you/Pictures": {
    dirs: 24,
    files: 4200,
    pools: [
      ["IMG", "DSC", "PXL", "Screenshot", "Scan"],
      ["IMG_2026", "DSC_0", "PXL_2026", "Screenshot 2026", "Beach", "Studio", "Portrait", "Rooftop"],
    ],
  },
  "/home/you/Downloads/installers": {
    dirs: 2,
    files: 180,
    pools: [
      ["setup", "installer", "update", "driver"],
      ["Adobe", "Blender", "Unity", "Godot", "Obsidian", "Rust", "Node", "Firefox"],
    ],
  },
  "/home/you/Documents/Invoices": {
    dirs: 0,
    files: 420,
    pools: [["INV", "RCPT", "Credit-note"], []],
  },
  "/home/you/Documents/Design": {
    dirs: 12,
    files: 860,
    pools: [["mockup", "wireframe", "token-sheet", "icon-grid", "type-spec"], []],
  },
  "/home/you/Music/Ambient": {
    dirs: 0,
    files: 96,
    pools: [["drift", "aurora", "long-night", "signal"], []],
  },
  "/home/you/Music/Jazz": {
    dirs: 0,
    files: 74,
    pools: [["quartet", "blue-hour", "take-five", "session"], []],
  },
  "/home/you/Videos/Clips": {
    dirs: 0,
    files: 132,
    pools: [["clip", "take", "b-roll"], []],
  },
  "/home/you/Documents/Contracts": {
    dirs: 0,
    files: 64,
    pools: [["contract", "NDA", "SOW", "amendment"], []],
  },
  "/home/you/Desktop/Screenshots": {
    dirs: 0,
    files: 340,
    pools: [["Screenshot 2026"], []],
  },
  "/home/you/Documents/Lyre/src": {
    dirs: 9,
    files: 120,
    pools: [["index", "app", "main", "store", "router"], []],
  },
  "/home/you/Documents/Lyre/src-tauri": {
    dirs: 3,
    files: 40,
    pools: [["lib", "main", "scan", "places", "build"], []],
  },
  "/home/you/Documents/Lyre/public": {
    dirs: 2,
    files: 36,
    pools: [["icon", "logo", "hero"], []],
  },
};

const DIR_WORDS = [
  "Holiday",
  "Wedding",
  "Archive",
  "Screens",
  "Wallpapers",
  "Reference",
  "Unsorted",
  "Trip",
  "Family",
  "Phone backup",
];

const EXTENSIONS = [
  "jpg",
  "png",
  "heic",
  "tif",
  "webp",
  "mp4",
  "mov",
  "pdf",
  "txt",
  "csv",
  "zip",
  "dng",
];

function pick<T>(list: T[], random: () => number): T {
  return list[Math.floor(random() * list.length)]!;
}

function capitalise(word: string): string {
  return word.charAt(0).toUpperCase() + word.slice(1);
}

const KIND_MAP: Record<string, EntryKind> = {
  png: "image", jpg: "image", jpeg: "image", gif: "image", webp: "image",
  svg: "image", heic: "image", tif: "image", tiff: "image", raw: "image",
  mp4: "video", mkv: "video", mov: "video", webm: "video", avi: "video",
  mp3: "audio", flac: "audio", wav: "audio", aiff: "audio", m4a: "audio",
  zip: "archive", tar: "archive", gz: "archive", iso: "archive", dmg: "archive",
  "7z": "archive", rar: "archive",
  pdf: "pdf",
  docx: "document", doc: "document", odt: "document", rtf: "document",
  xlsx: "spreadsheet", xls: "spreadsheet", csv: "spreadsheet", tsv: "spreadsheet",
  pptx: "presentation", ppt: "presentation", odp: "presentation",
  txt: "text", md: "text", log: "text", rst: "text",
  ts: "code", tsx: "code", js: "code", jsx: "code", rs: "code", py: "code",
  json: "code", toml: "code", yaml: "code", yml: "code", html: "code",
  css: "code", sh: "code", c: "code", h: "code", cpp: "code", go: "code",
  sql: "code", vue: "code", svelte: "code", xml: "code",
  exe: "executable", msi: "executable", deb: "executable", appimage: "executable",
  ttf: "font", otf: "font", woff: "font", woff2: "font",
  db: "database", sqlite: "database", parquet: "database",
  so: "binary", dll: "binary", dylib: "binary", wasm: "binary", o: "binary",
  desktop: "shortcut", lnk: "shortcut", url: "shortcut", alias: "shortcut",
};

function extKind(ext: string): EntryKind {
  return KIND_MAP[ext] ?? "file";
}

function hash(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i += 1) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Browser-preview search.
 *
 * The packaged app scores with nucleo in Rust. This stands in for it so the
 * result list, the streaming behaviour and the highlighting can be exercised
 * without a Rust build; it is a simplified matcher, not a second definition of
 * the real ranking.
 */
export async function mockSearch(
  path: string,
  roots: string[],
  query: string,
  recursive: boolean,
  showHidden: boolean,
  showProtected: boolean,
  handlers: SearchHandlers,
): Promise<ScanHandle> {
  const started = performance.now();
  let cancelled = false;
  // A place with no folder of its own (This PC) starts straight at its roots.
  const start = path === "" ? (roots[0] ?? "/") : path;

  const local = path === "" ? [] : listing(path)
    .filter((entry) => visible(entry, showHidden, showProtected))
    .map((entry) => toHit(entry, query))
    .filter((hit): hit is Hit => hit !== null)
    .sort((a, b) => b.score - a.score);

  setTimeout(() => {
    if (cancelled) return;
    handlers.onLocal(local, local.length, Math.round(performance.now() - started));
    if (!recursive) {
      handlers.onDone(local.length, 0, Math.round(performance.now() - started), false);
      return;
    }

    // Walk the synthetic tree's known folders, a few results per tick, so the
    // interface sees the same two-wave arrival as the real engine.
    const base = start.endsWith("/") ? start : `${start}/`;
    const prefix = base;
    const deeper: Hit[] = [];
    // Phase one already reported this folder's own entries; repeating them
    // here would give two rows the same key.
    const seen = new Set(local.map((hit) => hit.path));
    const push = (hit: Hit | null) => {
      if (!hit || seen.has(hit.path)) return;
      seen.add(hit.path);
      deeper.push(hit);
    };
    for (const key of Object.keys(CHILDREN)) {
      if (!key.startsWith(prefix) || key === path) continue;
      const dirName = key.split("/").filter(Boolean).pop() ?? "";
      const dirEntry = makeEntry(key.slice(0, key.lastIndexOf("/")) || "/", dirName, "folder", mulberry32(hash(key)), Date.now());
      const dirHit = toHit({ ...dirEntry, path: key }, query);
      push(dirHit);
      for (const [name, kind] of CHILDREN[key]!) {
        const entry = makeEntry(key, name, kind, mulberry32(hash(key + name)), Date.now());
        if (visible(entry, showHidden, showProtected)) push(toHit(entry, query));
      }
    }

    let sent = 0;
    const tick = () => {
      if (cancelled) return;
      const batch = deeper.slice(sent, sent + 6);
      sent += batch.length;
      if (batch.length > 0) handlers.onBatch(batch, sent);
      if (sent < deeper.length) {
        setTimeout(tick, 24);
      } else {
        handlers.onDone(
          local.length + deeper.length,
          deeper.length,
          Math.round(performance.now() - started),
          false,
        );
      }
    };
    setTimeout(tick, 16);
  }, 2);

  return {
    cancel: () => {
      cancelled = true;
    },
  };
}

/** Subsequence match with a contiguity and word-start bonus. */
function toHit(entry: Entry, query: string): Hit | null {
  const haystack = entry.name.toLowerCase();
  const needle = query.trim().toLowerCase();
  if (!needle) return null;

  const indices: number[] = [];
  let score = 0;
  let run = 0;
  let cursor = 0;

  for (const char of needle) {
    const found = haystack.indexOf(char, cursor);
    if (found === -1) return null;
    if (run > 0 && found === cursor) {
      run += 1;
      score += 8 + run * 2;
    } else {
      run = 1;
      score += 4;
      if (found === 0) score += 12;
      else if (haystack[found - 1] === " " || haystack[found - 1] === "-") score += 6;
    }
    indices.push(found);
    cursor = found + 1;
  }

  score += Math.max(0, 24 - entry.name.length);
  return { ...entry, score, indices };
}
