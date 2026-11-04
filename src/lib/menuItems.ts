import type { ContextItem } from "@/components/ContextMenu";
import type { ShellMenuEntry } from "./ipc";

/**
 * Every item Lyre's own context menus can show, in the order they are offered.
 *
 * The menus themselves are built where the actions live, because an item needs
 * the current folder to do anything. This is the catalogue the customise list
 * reads: what exists, what it is called when it is not being described by the
 * selection, and which menu it belongs to.
 *
 * `id` is the contract between the two. It is stored when someone turns an item
 * off, so it must never be renamed to reflect a label.
 */
export type MenuScope = "item" | "background" | "place";

export interface MenuSpec {
  id: string;
  label: string;
  scope: MenuScope;
  group: string;
  /** Shown dimmed beside the name: which targets the item appears for. */
  detail?: string;
}

/** The menu a right-click opens on a file or a folder. */
const ITEM: MenuSpec[] = [
  { id: "open", label: "Open", scope: "item", group: "Open" },
  {
    id: "open-tab",
    label: "Open in new tab",
    scope: "item",
    group: "Open",
    detail: "Folders",
  },
  {
    id: "open-with",
    label: "Open with…",
    scope: "item",
    group: "Open",
    detail: "Files",
  },
  { id: "favorite", label: "Add to favorites", scope: "item", group: "Organise" },
  { id: "pin", label: "Pin to sidebar", scope: "item", group: "Organise" },
  { id: "copy-path", label: "Copy path", scope: "item", group: "Organise" },
  { id: "rename", label: "Rename", scope: "item", group: "Organise" },
  { id: "cut", label: "Cut", scope: "item", group: "Organise" },
  { id: "copy", label: "Copy", scope: "item", group: "Organise" },
  { id: "delete", label: "Delete", scope: "item", group: "Organise" },
  { id: "properties", label: "Properties", scope: "item", group: "Inspect" },
];

/** The menu a right-click opens on the empty space of a folder. */
const BACKGROUND: MenuSpec[] = [
  { id: "new-folder", label: "New folder", scope: "background", group: "Create" },
  { id: "paste", label: "Paste", scope: "background", group: "Create" },
  { id: "sort-name", label: "Sort by name", scope: "background", group: "Sort by" },
  {
    id: "sort-modified",
    label: "Sort by date modified",
    scope: "background",
    group: "Sort by",
  },
  { id: "sort-type", label: "Sort by type", scope: "background", group: "Sort by" },
  { id: "sort-size", label: "Sort by size", scope: "background", group: "Sort by" },
  {
    id: "view-details",
    label: "Details view",
    scope: "background",
    group: "View",
  },
  {
    id: "view-icons",
    label: "Large icons view",
    scope: "background",
    group: "View",
  },
  { id: "refresh", label: "Refresh", scope: "background", group: "Page" },
  { id: "select-all", label: "Select all", scope: "background", group: "Page" },
];

/** The menu a right-click opens on a place: a rail row, a tile, a drive card. */
const PLACE: MenuSpec[] = [
  { id: "place-open", label: "Open", scope: "place", group: "Open" },
  {
    id: "place-favorite",
    label: "Add to favorites",
    scope: "place",
    group: "Organise",
    detail: "Folders",
  },
  {
    id: "place-pin",
    label: "Pin to sidebar",
    scope: "place",
    group: "Organise",
    detail: "Folders",
  },
  {
    id: "place-eject",
    label: "Eject",
    scope: "place",
    group: "Organise",
    detail: "Removable drives",
  },
  { id: "place-copy", label: "Copy path", scope: "place", group: "Organise" },
];

export const MENU_ITEMS: MenuSpec[] = [...ITEM, ...BACKGROUND, ...PLACE];

/** The items of one menu, in the order that menu draws them. */
export function menuSpecs(scope: MenuScope): MenuSpec[] {
  return MENU_ITEMS.filter((spec) => spec.scope === scope);
}

/**
 * Whether an item shows, given what the person has turned off and on.
 *
 * Two lists rather than one because an item can start either way: the menu's own
 * items start on, and the ones Lyre already offers a better version of start
 * off. One list of exceptions covers both without a tri-state per item.
 */
export function menuItemShown(
  id: string,
  defaultOn: boolean,
  hidden: ReadonlySet<string>,
  shown: ReadonlySet<string>,
): boolean {
  if (hidden.has(id)) return false;
  if (shown.has(id)) return true;
  return defaultOn;
}

/**
 * Drop what has been turned off, then tidy the result.
 *
 * The menus are written as blocks between separators, so removing an item can
 * leave an empty block, a separator at the top, or a heading like "Sort by" with
 * nothing under it. All three are removed here rather than at every call site.
 */
export function filterMenu(
  items: ContextItem[],
  isShown: (id: string, defaultOn: boolean) => boolean,
): ContextItem[] {
  const blocks: ContextItem[][] = [[]];
  for (const item of items) {
    if (item.kind === "sep") blocks.push([]);
    else blocks[blocks.length - 1]!.push(item);
  }

  const kept = blocks
    .map((block) =>
      block.filter((item) =>
        item.kind === "item" ? isShown(item.id, item.defaultOn ?? true) : true,
      ),
    )
    // A block of nothing but headings describes items that are no longer there.
    .filter((block) => block.some((item) => item.kind === "item"));

  const out: ContextItem[] = [];
  for (const block of kept) {
    if (out.length > 0) out.push({ kind: "sep" });
    out.push(...block);
  }
  return out;
}

/**
 * The system's own items, as menu rows.
 *
 * Only the top level is filtered: a program that adds a group adds it whole, and
 * the choice on offer is whether Lyre shows that group, not which of its members
 * survive.
 */
export function shellMenuItems(
  entries: ShellMenuEntry[],
  isShown: (id: string, defaultOn: boolean) => boolean,
  run: (entry: ShellMenuEntry) => void,
): ContextItem[] {
  const out: ContextItem[] = [];
  for (const entry of entries) {
    if (!isShown(entry.id, entry.defaultOn)) continue;
    if (entry.children.length > 0) {
      const children = shellMenuItems(entry.children, () => true, run);
      if (children.length === 0) continue;
      out.push({
        kind: "item",
        id: entry.id,
        label: entry.label || entry.source,
        defaultOn: entry.defaultOn,
        children,
      });
    } else {
      out.push({
        kind: "item",
        id: entry.id,
        label: entry.label || entry.source,
        defaultOn: entry.defaultOn,
        onSelect: () => run(entry),
      });
    }
  }
  return out;
}

/**
 * Put a block of items before a named one, keeping exactly one separator
 * between it and whatever came before.
 */
export function insertMenuBlock(
  items: ContextItem[],
  beforeId: string | null,
  block: ContextItem[],
): ContextItem[] {
  if (block.length === 0) return items;
  const index =
    beforeId === null
      ? items.length
      : items.findIndex((item) => item.kind === "item" && item.id === beforeId);
  if (index === -1) return [...items, ...block];

  const head = items.slice(0, index);
  while (head.length > 0 && head[head.length - 1]!.kind === "sep") head.pop();
  return [...head, { kind: "sep" }, ...block, ...items.slice(index)];
}
