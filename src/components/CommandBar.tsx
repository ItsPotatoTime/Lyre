import type { KindFilter, SortKey, SortState, ViewMode } from "@/types";
import type { Mode } from "@/lib/modes";
import type { ThemePref } from "@/lib/theme";
import { Icon, type IconName } from "./Icons";
import { Menu, type MenuEntry } from "./Menu";

const DEFERRED = "Arrives with file operations";

const FILTERS: { id: KindFilter; label: string }[] = [
  { id: "all", label: "All items" },
  { id: "folder", label: "Folders" },
  { id: "document", label: "Documents" },
  { id: "image", label: "Images" },
  { id: "video", label: "Video" },
  { id: "audio", label: "Audio" },
  { id: "archive", label: "Compressed archives" },
  { id: "code", label: "Code and programs" },
];

export function CommandBar({
  view,
  onView,
  mode,
  onMode,
  sort,
  onSort,
  filter,
  onFilter,
  groupByDate,
  onGroupByDate,
  navOpen,
  onToggleNav,
  showHidden,
  onToggleHidden,
  selectedCount,
  onSelectAll,
  onCopyPath,
  onRefresh,
  theme,
  onTheme,
  isDefaultManager,
  managerUnsupported,
  promptDefaultManager,
  onSetDefaultManager,
  onTogglePromptManager,
  onMeasureFrameRate,
  canRestoreFileManager,
  onRestoreFileManager,
  reducedMemory,
  onToggleReducedMemory,
  canRestorePins,
  onRestorePins,
  notice,
}: {
  view: ViewMode;
  onView: (next: ViewMode) => void;
  mode: Mode;
  onMode: (next: Mode) => void;
  sort: SortState;
  onSort: (next: SortState) => void;
  filter: KindFilter;
  onFilter: (next: KindFilter) => void;
  groupByDate: boolean;
  onGroupByDate: (next: boolean) => void;
  navOpen: boolean;
  onToggleNav: () => void;
  showHidden: boolean;
  onToggleHidden: () => void;
  selectedCount: number;
  onSelectAll: () => void;
  onCopyPath: () => void;
  onRefresh: () => void;
  theme: ThemePref;
  onTheme: (next: ThemePref) => void;
  isDefaultManager: boolean;
  managerUnsupported: boolean;
  promptDefaultManager: boolean;
  onSetDefaultManager: () => void;
  onTogglePromptManager: () => void;
  onMeasureFrameRate: () => void;
  canRestoreFileManager: boolean;
  onRestoreFileManager: () => void;
  reducedMemory: boolean;
  onToggleReducedMemory: () => void;
  canRestorePins: boolean;
  onRestorePins: () => void;
  notice: (text: string) => void;
}) {
  const hasSelection = selectedCount > 0;

  const sortItems: MenuEntry[] = (
    [
      ["name", "Name"],
      ["modified", "Date modified"],
      ["type", "Type"],
      ["size", "Size"],
    ] as [SortKey, string][]
  ).map(([key, labelText]) => ({
    kind: "item",
    id: key,
    label: labelText,
    checked: sort.key === key,
    onSelect: () => onSort({ key, dir: sort.key === key ? sort.dir : 1 }),
  }));

  const viewItems: MenuEntry[] = [
    {
      kind: "item",
      id: "details",
      label: "Details",
      icon: "viewDetails",
      checked: view === "details",
      hint: "Ctrl+Shift+1",
      onSelect: () => onView("details"),
    },
    {
      kind: "item",
      id: "grid",
      label: "Large icons",
      icon: "viewGrid",
      checked: view === "grid",
      hint: "Ctrl+Shift+2",
      onSelect: () => onView("grid"),
    },
    { kind: "sep" },
    {
      kind: "item",
      id: "compact",
      label: "Compact view",
      checked: mode === "compact",
      onSelect: () => onMode(mode === "compact" ? "explorer" : "compact"),
    },
    { kind: "sep" },
    {
      kind: "item",
      id: "hidden",
      label: "Show hidden items",
      checked: showHidden,
      onSelect: onToggleHidden,
    },
    {
      kind: "item",
      id: "nav",
      label: "Navigation pane",
      checked: navOpen,
      hint: "Ctrl+B",
      onSelect: onToggleNav,
    },
  ];

  const moreItems: MenuEntry[] = [
    {
      kind: "item",
      id: "refresh",
      label: "Refresh",
      icon: "refresh",
      hint: "F5",
      onSelect: onRefresh,
    },
    {
      kind: "item",
      id: "selectall",
      label: "Select all",
      hint: "Ctrl+A",
      onSelect: onSelectAll,
    },
    {
      kind: "item",
      id: "copypath",
      label: "Copy path",
      disabled: !hasSelection,
      onSelect: onCopyPath,
    },
    { kind: "sep" },
    {
      kind: "item",
      id: "default-manager",
      label: isDefaultManager
        ? "Folders open with Lyre"
        : "Open folders with Lyre…",
      disabled: isDefaultManager || managerUnsupported,
      onSelect: onSetDefaultManager,
    },
    ...(canRestoreFileManager
      ? ([
          {
            kind: "item" as const,
            id: "restore-manager",
            label: "Stop opening folders with Lyre",
            onSelect: onRestoreFileManager,
          },
        ])
      : []),
    {
      kind: "item",
      id: "prompt-manager",
      label: "Offer this at startup",
      checked: promptDefaultManager,
      onSelect: onTogglePromptManager,
    },
    {
      kind: "item",
      id: "reduced-memory",
      label: "Reduce graphics memory (restart)",
      checked: reducedMemory,
      onSelect: onToggleReducedMemory,
    },
    {
      kind: "item",
      id: "frame-rate",
      label: "Measure frame rate",
      onSelect: onMeasureFrameRate,
    },
    ...(canRestorePins
      ? ([
          {
            kind: "item" as const,
            id: "restore-pins",
            label: "Restore sidebar folders",
            onSelect: onRestorePins,
          },
        ])
      : []),
    { kind: "sep" },
    { kind: "label", text: "Theme" },
    {
      kind: "item",
      id: "theme-system",
      label: "Use system setting",
      checked: theme === "system",
      onSelect: () => onTheme("system"),
    },
    {
      kind: "item",
      id: "theme-light",
      label: "Light",
      checked: theme === "light",
      onSelect: () => onTheme("light"),
    },
    {
      kind: "item",
      id: "theme-dark",
      label: "Dark",
      checked: theme === "dark",
      onSelect: () => onTheme("dark"),
    },
  ];

  const op = (
    icon: IconName,
    label: string,
    message: string,
  ) => (
    <button
      type="button"
      className="cmd-icon"
      data-disabled={hasSelection ? "false" : "true"}
      aria-label={label}
      title={`${label}: ${DEFERRED}`}
      onClick={() => notice(message)}
    >
      <Icon name={icon} size={16} />
    </button>
  );

  return (
    <div className="commandbar">
      <Menu
        entries={[
          {
            kind: "item",
            id: "folder",
            label: "Folder",
            disabled: true,
            onSelect: () => {},
          },
          {
            kind: "item",
            id: "shortcut",
            label: "Shortcut",
            disabled: true,
            onSelect: () => {},
          },
        ]}
        trigger={({ toggle }) => (
          <button
            type="button"
            className="cmd"
            data-reveal="true"
            title={`New: ${DEFERRED}`}
            onClick={() => {
              notice("Creating folders and shortcuts arrives with file operations.");
              toggle();
            }}
          >
            <Icon name="folderPlus" size={16} />
            <span>New</span>
            <Icon name="chevronDown" size={9} className="cmd-chevron" />
          </button>
        )}
      />

      <div className="cmd-gap" />

      <div className="cmd-group">
        {op("cut", "Cut", "Cut arrives with file operations.")}
        {op("copy", "Copy", "Copy arrives with file operations.")}
        {op("paste", "Paste", "Paste arrives with file operations.")}
        {op("rename", "Rename", "Renaming arrives with file operations.")}
        {op("share", "Share", "Sharing arrives with file operations.")}
        {op("trash", "Delete", "Deleting arrives with file operations.")}
      </div>

      <div className="cmd-gap" />

      <Menu
        align="start"
        entries={[
          ...sortItems,
          { kind: "sep" },
          {
            kind: "item",
            id: "asc",
            label: "Ascending",
            checked: sort.dir === 1,
            onSelect: () => onSort({ key: sort.key, dir: 1 }),
          },
          {
            kind: "item",
            id: "desc",
            label: "Descending",
            checked: sort.dir === -1,
            onSelect: () => onSort({ key: sort.key, dir: -1 }),
          },
          { kind: "sep" },
          {
            kind: "item",
            id: "group",
            label: "Group by date",
            checked: groupByDate,
            onSelect: () => onGroupByDate(!groupByDate),
          },
        ]}
        trigger={({ toggle }) => (
          <button
            type="button"
            className="cmd"
            data-reveal="true"
            title="Sort by"
            onClick={toggle}
          >
            <Icon name="sort" size={16} />
            <span>Sort</span>
            <Icon name="chevronDown" size={9} className="cmd-chevron" />
          </button>
        )}
      />

      <Menu
        align="start"
        entries={viewItems}
        trigger={({ toggle }) => (
          <button
            type="button"
            className="cmd"
            data-reveal="true"
            title="Change your view"
            onClick={toggle}
          >
            <Icon name="viewDetails" size={16} />
            <span>View</span>
            <Icon name="chevronDown" size={9} className="cmd-chevron" />
          </button>
        )}
      />

      <Menu
        align="start"
        entries={FILTERS.map((entry) => ({
          kind: "item" as const,
          id: entry.id,
          label: entry.label,
          checked: filter === entry.id,
          onSelect: () => onFilter(entry.id),
        }))}
        trigger={({ toggle }) => (
          <button
            type="button"
            className="cmd"
            data-reveal="true"
            data-active={filter !== "all" ? "true" : "false"}
            title="Filter by type"
            onClick={toggle}
          >
            <Icon name="filter" size={16} />
            <span>Filter</span>
            <Icon name="chevronDown" size={9} className="cmd-chevron" />
          </button>
        )}
      />

      <Menu
        align="start"
        entries={moreItems}
        trigger={({ toggle }) => (
          <button
            type="button"
            className="cmd-icon"
            aria-label="More"
            title="More"
            onClick={toggle}
          >
            <Icon name="more" size={16} />
          </button>
        )}
      />

      <div className="cmd-spacer" />

      <Menu
        align="end"
        entries={[
          {
            kind: "item",
            id: "details",
            label: "Details",
            checked: view === "details",
            onSelect: () => onView("details"),
          },
          {
            kind: "item",
            id: "grid",
            label: "Large icons",
            checked: view === "grid",
            onSelect: () => onView("grid"),
          },
          { kind: "sep" },
          {
            kind: "item",
            id: "compact",
            label: "Compact view",
            checked: mode === "compact",
            onSelect: () => onMode(mode === "compact" ? "explorer" : "compact"),
          },
        ]}
        trigger={({ toggle }) => (
          <button
            type="button"
            className="cmd"
            data-reveal="true"
            title="Change your view"
            onClick={toggle}
          >
            <Icon name="viewDetails" size={14} />
            <span>{view === "details" ? "Details" : "Large icons"}</span>
          </button>
        )}
      />
    </div>
  );
}
