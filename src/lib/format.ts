/** Formatting that matches how Explorer prints the same values. */

const UNITS = ["KB", "MB", "GB", "TB", "PB"] as const;

/**
 * Explorer reports sizes in KB at minimum (a folder of tiddlers reads
 * "1 KB" and an empty file reads "0 KB"), then steps up a unit per thousand.
 */
export function formatSize(bytes: number, isDir: boolean): string {
  if (isDir) return "";
  if (bytes <= 0) return "0 KB";
  let value = bytes / 1024;
  let unit = 0;
  if (value < 1) return "1 KB";
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value >= 100 ? 0 : value >= 10 ? 1 : 2;
  const text = value.toFixed(digits).replace(/\.?0+$/, "");
  return `${text} ${UNITS[unit]}`;
}

const dateFormat = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

export function formatDate(ms: number | null): string {
  if (ms === null) return "";
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return "";
  return dateFormat.format(date);
}

const countFormat = new Intl.NumberFormat();

export function formatCount(n: number): string {
  return countFormat.format(n);
}

/** The Type column: Explorer's own human names for generic file families. */
export function typeLabel(entry: { kind: string; ext: string; isDir: boolean }): string {
  if (entry.isDir) return "File folder";
  switch (entry.kind) {
    case "image":
      return `${entry.ext.toUpperCase()} image`.replace(/^ /, "");
    case "video":
      return `${entry.ext.toUpperCase()} video`.trim();
    case "audio":
      return `${entry.ext.toUpperCase()} audio`.trim();
    case "pdf":
      return "PDF document";
    case "document":
      return "Document";
    case "spreadsheet":
      return "Spreadsheet";
    case "presentation":
      return "Presentation";
    case "text":
      return "Text document";
    case "code":
      return "Source file";
    case "archive":
      return "Compressed archive";
    case "executable":
      return "Application";
    case "font":
      return "Font file";
    case "database":
      return "Database";
    case "binary":
      return "Binary file";
    case "shortcut":
      return "Shortcut";
    default:
      return entry.ext ? `${entry.ext.toUpperCase()} file` : "File";
  }
}

export function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many;
}
