import type { SidePanelKind } from "../../shared/types";

export interface KeyInput {
  key: string;
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
  repeat?: boolean;
}

export interface KeyContext {
  dialogOpen: boolean;
  recordingHotkey?: boolean;
  settingFixedPosition?: boolean;
  /** Focus is in the search box. */
  inSearch: boolean;
  /** Focus is in any text field, the search box included. */
  inField: boolean;
  panel: SidePanelKind | null;
  /** Settings, or setting the fixed position: the list is not in use. */
  blocked: boolean;
  /** The list's own keys (arrows, Return, Delete, Tab and paste shortcuts) apply. */
  listKeys: boolean;
  itemCount: number;
  selectedIndex: number | null;
  batchCount: number;
  /** The list shows the finished result of the current search and tab. */
  canSelectAll: boolean;
  searchPending: boolean;
  hasTextSelection: boolean;
}

export type KeyAction =
  /** Not ours: the key does what it normally does. */
  | { type: "ignore" }
  /** Swallowed, nothing else happens. */
  | { type: "consume" }
  | { type: "escape" }
  | { type: "closeWindow" }
  | { type: "focusSearch" }
  | { type: "cycleType"; step: 1 | -1 }
  | { type: "select"; index: number }
  | { type: "paste" }
  | { type: "pastePlain" }
  | { type: "pasteIndex"; index: number }
  | { type: "copy" }
  | { type: "flushSearch" }
  | { type: "remove" }
  | { type: "startBatchSelectAll" }
  | { type: "toggleSelectAll" }
  | { type: "batchDelete" };

const IGNORE: KeyAction = { type: "ignore" };
const CONSUME: KeyAction = { type: "consume" };

const isCommand = (input: KeyInput) => input.metaKey && !input.ctrlKey && !input.altKey && !input.shiftKey;

const isFind = (input: KeyInput) =>
  (input.metaKey || input.ctrlKey) && !input.altKey && !input.shiftKey && input.key.toLowerCase() === "f";

const shortcutIndex = (input: KeyInput) => {
  const digit = /^[1-9]$/.test(input.key) ? input.key : /^Digit([1-9])$/.exec(input.code ?? "")?.[1];
  return digit === undefined ? null : Number(digit) - 1;
};

const isDelete = (input: KeyInput) => input.key === "Delete" || (input.metaKey && input.key === "Backspace");

/** What a key press does in the main window. */
export const resolveKey = (input: KeyInput, ctx: KeyContext): KeyAction => {
  if (input.isComposing || input.key === "Process") return IGNORE;
  const command = isCommand(input);
  const key = input.key.toLowerCase();
  const close = command && key === "w";
  if (ctx.dialogOpen) return close ? CONSUME : IGNORE;
  if (close) {
    if (ctx.recordingHotkey) return IGNORE;
    if (ctx.settingFixedPosition || input.repeat) return CONSUME;
    return { type: "closeWindow" };
  }
  if (input.key === "Escape") return { type: "escape" };
  if (ctx.blocked) return IGNORE;

  if (command && key === "a") {
    // A text field keeps its own select-all.
    if (ctx.inField) return IGNORE;
    const ready = ctx.canSelectAll && ctx.itemCount > 0;
    if (ctx.panel === "batch") return ready ? { type: "toggleSelectAll" } : CONSUME;
    // Editing, translating, a preview or 常用: Cmd+A must not swap the panel out.
    if (ctx.panel !== null) return IGNORE;
    return ready ? { type: "startBatchSelectAll" } : CONSUME;
  }

  if (isFind(input)) {
    return ctx.panel === "edit" || ctx.panel === "translate" ? IGNORE : { type: "focusSearch" };
  }

  // Batch mode: no single-item paste or delete, so a stray Return or Delete
  // can't act on one entry.
  if (ctx.panel === "batch") {
    if (ctx.inField) return IGNORE;
    if (isDelete(input)) return ctx.batchCount > 0 ? { type: "batchDelete" } : IGNORE;
    return IGNORE;
  }

  if (!ctx.listKeys) return IGNORE;
  if (ctx.inField && !ctx.inSearch) return IGNORE;

  const selected = ctx.selectedIndex !== null && ctx.selectedIndex < ctx.itemCount;
  if (input.key === "Enter" && !input.metaKey && !input.ctrlKey && !input.altKey) {
    if (ctx.searchPending) return { type: "flushSearch" };
    if (!selected) return IGNORE;
    return input.shiftKey ? { type: "pastePlain" } : { type: "paste" };
  }
  if (command && key === "c") {
    if (ctx.hasTextSelection) return IGNORE;
    if (ctx.searchPending) return { type: "flushSearch" };
    return selected ? { type: "copy" } : IGNORE;
  }
  if (command) {
    const index = shortcutIndex(input);
    if (index !== null) {
      if (ctx.searchPending) return { type: "flushSearch" };
      return index < ctx.itemCount ? { type: "pasteIndex", index } : CONSUME;
    }
  }

  if (input.key === "Tab" && !input.metaKey && !input.ctrlKey && !input.altKey) {
    return { type: "cycleType", step: input.shiftKey ? -1 : 1 };
  }

  if (input.key === "ArrowDown" || input.key === "ArrowUp") {
    const last = ctx.itemCount - 1;
    if (last < 0) return CONSUME;
    const position = ctx.selectedIndex;
    const index =
      input.key === "ArrowDown"
        ? position === null ? 0 : Math.min(position + 1, last)
        : position === null ? 0 : Math.max(position - 1, 0);
    return { type: "select", index };
  }

  if (!selected) return IGNORE;
  if (isDelete(input) && !ctx.inSearch) return { type: "remove" };
  return IGNORE;
};
