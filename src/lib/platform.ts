/**
 * What the landing page is called.
 *
 * It is the Quick access tile grid plus the Favorites shelf, so it is named for
 * what it contains rather than borrowed from Explorer, where the same page is
 * called "Home". One name on every platform: a Linux user reads "Home" as their
 * own folder, and having the word mean two different things depending on the
 * operating system is worse than having a slightly less familiar label.
 *
 * "Home" is therefore the home directory, everywhere, and the rail always has a
 * row for it.
 */
export const LANDING_LABEL = "Quick access";

/** What the home directory is called, in the rail, a tab and a breadcrumb. */
export const HOME_FOLDER_LABEL = "Home";

let homePath: string | null = null;

/** Record where the home directory is, so it can be named rather than spelled. */
export function setHomePath(path: string): void {
  homePath = path;
}

/**
 * The name to show for a folder in a tab or a breadcrumb: "Home" rather than
 * the account name, so the word means the same thing everywhere in the window.
 */
export function folderLabel(path: string, fallback: string): string {
  return homePath !== null && path === homePath ? HOME_FOLDER_LABEL : fallback;
}
