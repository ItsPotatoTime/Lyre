/**
 * Which desktop this window is running on, for the few places where Windows and
 * the rest of the world genuinely disagree about what a thing is called.
 *
 * Set once at startup from the real platform rather than sniffed from the user
 * agent, and read synchronously afterwards so the first paint is already right.
 */
let windows = false;

export function setPlatform(os: string): void {
  windows = os === "windows";
}

/**
 * The landing page: Quick access and Favorites.
 *
 * On Windows this is Explorer's "Home". Calling it that on Linux is where the
 * confusion started: a Linux user reads "Home" as their own folder, and the
 * page they got instead was a list of shortcuts with an empty favourites shelf.
 * So on Linux the page is named for what it contains and "Home" means the home
 * directory, which is what it means everywhere else on that system.
 */
export function landingLabel(): string {
  return windows ? "Home" : "Quick access";
}

/** What the home directory is called. It is Home everywhere, on purpose. */
export const HOME_FOLDER_LABEL = "Home";

let homePath: string | null = null;

/** Record where the home directory is, so it can be named rather than spelled. */
export function setHomePath(path: string): void {
  homePath = path;
}

/**
 * The name to show for a folder in a tab or a breadcrumb.
 *
 * The home directory is shown as "Home" rather than as the account name, which
 * is what every other Linux file manager does and what the rail row says.
 */
export function folderLabel(path: string, fallback: string): string {
  return homePath !== null && path === homePath ? HOME_FOLDER_LABEL : fallback;
}
