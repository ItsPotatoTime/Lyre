import type { SortKey, SortState, ViewMode } from "@/types";
import type { Mode } from "@/lib/modes";
import type { ThemePref } from "@/lib/theme";

import { Icon, type IconName } from "./Icons";
import { Menu, type MenuEntry } from "./Menu";

function CmdButton({
  icon,
  label,
  onClick,
  title,
  disabled = false,
  chevron = false,
  active = false,
}: {
  icon: IconName;
  label: string;
  onClick?: () => void;
  title: string;
  disabled?: boolean;
  chevron?: boolean;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      className="cmd-btn"
      data-reveal="true"
      data-disabled={disabled ? "true" : "false"}
      data-active={active ? "true" : "false"}
      aria-disabled={disabled}
      title={title}
      onClick={onClick}
    >
      <Icon name={icon} size={18} />
      <span className="cmd-label">
        {label}
        {chevron ? (
          <Icon
            name="chevronDown"
            size={8}
            strokeWidth={2}
            className="cmd-chevron"
          />
        ) : null}
      </span>
    </button>
  );
}

const DEFERRED = "Arrives with file operations";

export function CommandBar({
  view,
  onView,
  sort,
  onSort,
  navOpen,
  onToggleNav,
  showHidden,
  onToggleHidden,
  selectedCount,
  onSelectAll,
  onCopyPath,
  onRefresh,
  mode,
  onMode,
  theme,
  onTheme,
  notice,
}: {
  view: ViewMode;
  onView: (next: ViewMode) => void;
  sort: SortState;
  onSort: (next: SortState) => void;
  navOpen: boolean;
  onToggleNav: () => void;
  showHidden: boolean;
  onToggleHidden: () => void;
  selectedCount: number;
  onSelectAll: () => void;
  onCopyPath: () => void;
  onRefresh: () => void;
  mode: Mode;
  onMode: (next: Mode) => void;
  theme: ThemePref;
  onTheme: (next: ThemePref) => void;
  notice: (text: string) => void;
}) {
  const sortItems: MenuEntry[] = (
    [
      ["name", "Name"],
      ["modified", "Date modified"],
      ["type", "Type"],
      ["size", "Size"],
    ] as [SortKey, string][]
  ).map(([key, label]) => ({
    kind: "item",
    id: key,
    label,
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
      id: "hidden",
      label: "Show hidden items",
      checked: showHidden,
      onSelect: onToggleHidden,
    },
  ];

  const moreItems: MenuEntry[] = [
    { kind: "item", id: "refresh", label: "Refresh", icon: "refresh", hint: "F5", onSelect: onRefresh },
    { kind: "item", id: "selectall", label: "Select all", hint: "Ctrl+A", onSelect: onSelectAll },
    {
      kind: "item",
      id: "copypath",
      label: "Copy path",
      disabled: selectedCount === 0,
      onSelect: onCopyPath,
    },
    { kind: "sep" },
    {
      kind: "item",
      id: "nav",
      label: "Navigation pane",
      icon: "sidebar",
      checked: navOpen,
      hint: "Ctrl+B",
      onSelect: onToggleNav,
    },
    { kind: "sep" },
    { kind: "label", text: "Layout" },
    {
      kind: "item",
      id: "mode-explorer",
      label: "Explorer",
      checked: mode === "explorer",
      onSelect: () => onMode("explorer"),
    },
    {
      kind: "item",
      id: "mode-compact",
      label: "Compact",
      checked: mode === "compact",
      onSelect: () => onMode("compact"),
    },
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

  return (
    <div className="commandbar">
      <CmdButton
        icon="sidebar"
        label="Pane"
        title="Navigation pane (Ctrl+B)"
        onClick={onToggleNav}
        active={navOpen}
      />

      <div className="cmd-sep" />

      <div className="cmd-group">
        <Menu
          entries={[
            { kind: "item", id: "folder", label: "Folder", disabled: true, onSelect: () => {} },
            { kind: "item", id: "shortcut", label: "Shortcut", disabled: true, onSelect: () => {} },
          ]}
          trigger={({ toggle }) => (
            <CmdButton
              icon="folderPlus"
              label="New"
              chevron
              title={DEFERRED}
              onClick={() => {
                notice("Creating folders and shortcuts arrives with file operations.");
                toggle();
              }}
            />
          )}
        />
        <div className="cmd-sep" />

        <CmdButton icon="cut" label="Cut" title={DEFERRED} onClick={() => notice("Cut arrives with file operations.")} />
        <CmdButton icon="copy" label="Copy" title={DEFERRED} onClick={() => notice("Copy arrives with file operations.")} />
        <CmdButton icon="paste" label="Paste" title={DEFERRED} onClick={() => notice("Paste arrives with file operations.")} />
        <CmdButton
          icon="rename"
          label="Rename"
          title={DEFERRED}
          onClick={() => notice("Renaming arrives with file operations.")}
        />
        <CmdButton icon="share" label="Share" title={DEFERRED} onClick={() => notice("Sharing arrives with file operations.")} />
        <CmdButton icon="trash" label="Delete" title={DEFERRED} onClick={() => notice("Deleting arrives with file operations.")} />
      </div>

      <div className="cmd-sep" />

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
        ]}
        trigger={({ toggle }) => (
          <CmdButton
            icon="sort"
            label="Sort"
            chevron
            title="Sort by"
            onClick={toggle}
          />
        )}
      />
      <Menu
        align="start"
        entries={viewItems}
        trigger={({ toggle }) => (
          <CmdButton
            icon={view === "details" ? "viewDetails" : "viewGrid"}
            label="View"
            chevron
            title="Change your view"
            onClick={toggle}
          />
        )}
      />

      <div className="cmd-spacer" />

      <Menu
        align="end"
        entries={moreItems}
        trigger={({ toggle }) => (
          <button
            type="button"
            className="cmd-btn"
            data-compact="true"
            data-reveal="true"
            aria-label="More"
            title="More"
            onClick={toggle}
          >
            <Icon name="more" size={18} />
          </button>
        )}
      />
    </div>
  );
}
