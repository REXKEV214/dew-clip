import { describe, expect, test } from "vitest";
import { resolveKey, type KeyContext, type KeyInput } from "./keyboardRules";

const key = (name: string, mods: Partial<KeyInput> = {}): KeyInput => ({
  key: name,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  isComposing: false,
  ...mods,
});
const cmd = (name: string) => key(name, { metaKey: true });

const list: KeyContext = {
  dialogOpen: false,
  inSearch: false,
  inField: false,
  panel: null,
  blocked: false,
  listKeys: true,
  itemCount: 5,
  selectedIndex: 1,
  batchCount: 0,
  canSelectAll: true,
  searchPending: false,
  hasTextSelection: false,
};
const batch: KeyContext = { ...list, panel: "batch", listKeys: false, selectedIndex: null, batchCount: 3 };
const search: KeyContext = { ...list, inField: true, inSearch: true };

const type = (input: KeyInput, ctx: KeyContext) => resolveKey(input, ctx).type;

describe("Cmd+A", () => {
  test("on the list: batch mode with everything ticked", () => {
    expect(type(cmd("a"), list)).toBe("startBatchSelectAll");
    expect(type(cmd("A"), list)).toBe("startBatchSelectAll");
  });

  test("an empty or still-loading list only swallows it", () => {
    expect(type(cmd("a"), { ...list, itemCount: 0 })).toBe("consume");
    expect(type(cmd("a"), { ...list, canSelectAll: false })).toBe("consume");
    expect(type(cmd("a"), { ...batch, canSelectAll: false })).toBe("consume");
  });

  test("in batch mode: toggles select all", () => {
    expect(type(cmd("a"), batch)).toBe("toggleSelectAll");
  });

  test("a text field keeps its own select-all", () => {
    expect(type(cmd("a"), { ...list, inField: true, inSearch: true })).toBe("ignore");
    expect(type(cmd("a"), { ...batch, inField: true, inSearch: true })).toBe("ignore");
  });

  test("only Cmd+A exactly: Ctrl+A and Cmd+Shift+A are left alone", () => {
    expect(type(key("a", { ctrlKey: true }), list)).toBe("ignore");
    expect(type(key("a", { metaKey: true, shiftKey: true }), list)).toBe("ignore");
  });

  test("other panels, settings and fixed-position setup ignore it", () => {
    for (const panel of ["edit", "translate", "preview", "favorites"] as const) {
      expect(type(cmd("a"), { ...list, panel })).toBe("ignore");
    }
    expect(type(cmd("a"), { ...list, blocked: true })).toBe("ignore");
  });
});

describe("guards", () => {
  test("IME composition and open dialogs win over every shortcut", () => {
    for (const ctx of [list, batch]) {
      expect(type(key("a", { metaKey: true, isComposing: true }), ctx)).toBe("ignore");
      expect(type(key("Process"), ctx)).toBe("ignore");
      expect(type(cmd("a"), { ...ctx, dialogOpen: true })).toBe("ignore");
      expect(type(key("Delete"), { ...ctx, dialogOpen: true })).toBe("ignore");
      expect(type(key("Escape"), { ...ctx, dialogOpen: true })).toBe("ignore");
    }
  });

  test("Escape always reaches dismiss", () => {
    expect(type(key("Escape"), batch)).toBe("escape");
    expect(type(key("Escape"), { ...list, listKeys: false, panel: "edit" })).toBe("escape");
    expect(type(key("Escape"), { ...list, inField: true })).toBe("escape");
  });
});

describe("batch mode", () => {
  test("Delete and Cmd+Backspace ask to delete the ticked entries", () => {
    expect(type(key("Delete"), batch)).toBe("batchDelete");
    expect(type(cmd("Backspace"), batch)).toBe("batchDelete");
    expect(type(key("Delete"), { ...batch, batchCount: 0 })).toBe("ignore");
  });

  test("in the search box Delete edits the text", () => {
    expect(type(cmd("Backspace"), { ...batch, inField: true, inSearch: true })).toBe("ignore");
    expect(type(key("Delete"), { ...batch, inField: true })).toBe("ignore");
  });

  test("Cmd+F still finds", () => {
    expect(type(cmd("f"), batch)).toBe("focusSearch");
  });

  test("no single-entry paste, delete, arrows or tab switch", () => {
    // A stale keyboard pick must not act either.
    const picked = { ...batch, selectedIndex: 0 };
    for (const name of ["Enter", "ArrowDown", "ArrowUp", "Tab", "Backspace"]) {
      expect(type(key(name), picked)).toBe("ignore");
    }
  });
});

describe("the list", () => {
  test("arrows pick, Return pastes and Delete removes the picked entry", () => {
    expect(resolveKey(key("ArrowDown"), list)).toEqual({ type: "select", index: 2 });
    expect(resolveKey(key("ArrowUp"), { ...list, selectedIndex: null })).toEqual({ type: "select", index: 0 });
    expect(resolveKey(key("ArrowDown"), { ...list, selectedIndex: 4 })).toEqual({ type: "select", index: 4 });
    expect(type(key("ArrowDown"), { ...list, itemCount: 0 })).toBe("consume");
    expect(type(key("Enter"), list)).toBe("paste");
    expect(type(key("Delete"), list)).toBe("remove");
    expect(type(cmd("Backspace"), list)).toBe("remove");
  });

  test("the search box: Return pastes the pick, Delete edits the text", () => {
    const search = { ...list, inField: true, inSearch: true };
    expect(type(key("Enter"), search)).toBe("paste");
    expect(type(key("Delete"), search)).toBe("ignore");
  });

  test("other text fields keep every key", () => {
    const field = { ...list, inField: true };
    expect(type(key("Enter"), field)).toBe("ignore");
    expect(type(key("ArrowDown"), field)).toBe("ignore");
    expect(type(key("Tab"), field)).toBe("ignore");
  });

  test("nothing picked, or a pick past the end: Return and Delete do nothing", () => {
    expect(type(key("Enter"), { ...list, selectedIndex: null })).toBe("ignore");
    expect(type(key("Delete"), { ...list, selectedIndex: 9 })).toBe("ignore");
  });

  test("Tab cycles the type tabs", () => {
    expect(resolveKey(key("Tab"), list)).toEqual({ type: "cycleType", step: 1 });
    expect(resolveKey(key("Tab", { shiftKey: true }), list)).toEqual({ type: "cycleType", step: -1 });
  });

  test("editing panels and settings turn the list keys off", () => {
    for (const ctx of [
      { ...list, panel: "edit" as const, listKeys: false },
      { ...list, blocked: true, listKeys: false },
    ]) {
      expect(type(key("Enter"), ctx)).toBe("ignore");
      expect(type(key("Delete"), ctx)).toBe("ignore");
      expect(type(cmd("f"), ctx)).toBe("ignore");
    }
  });

  test("preview and favorites keep the list keys", () => {
    for (const panel of ["preview", "favorites"] as const) {
      expect(type(key("Enter"), { ...list, panel })).toBe("paste");
    }
  });
});

describe("find", () => {
  test("accepts Command or Control with no Shift or Option", () => {
    expect(type(cmd("F"), list)).toBe("focusSearch");
    expect(type(key("f", { ctrlKey: true }), batch)).toBe("focusSearch");
    expect(type(cmd("f"), { ...list, panel: "favorites", inField: true, listKeys: false })).toBe("focusSearch");
    expect(type(cmd("f"), { ...list, panel: "translate", listKeys: false })).toBe("ignore");
    for (const mods of [{ shiftKey: true }, { altKey: true }, { isComposing: true }]) {
      expect(type(key("f", { metaKey: true, ...mods }), list)).toBe("ignore");
    }
    expect(type(cmd("g"), list)).toBe("ignore");
    expect(type(key("F3"), list)).toBe("ignore");
  });
});

describe("paste and copy shortcuts", () => {
  const shiftedReturn = key("Enter", { shiftKey: true });

  test("Return pastes the first settled result and Shift+Return uses plain text", () => {
    const first = { ...search, selectedIndex: 0 };
    expect(type(key("Enter"), first)).toBe("paste");
    expect(type(shiftedReturn, first)).toBe("pastePlain");
    for (const mods of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }, { isComposing: true }]) {
      expect(type(key("Enter", { ...mods, shiftKey: true }), first)).toBe("ignore");
    }
    expect(type(shiftedReturn, { ...first, selectedIndex: null })).toBe("ignore");
  });

  test("Command digits use the current list order even while typing in search", () => {
    expect(resolveKey(cmd("1"), { ...list, selectedIndex: null })).toEqual({ type: "pasteIndex", index: 0 });
    expect(resolveKey(cmd("9"), { ...search, itemCount: 9 })).toEqual({ type: "pasteIndex", index: 8 });
    expect(resolveKey(key("&", { code: "Digit2", metaKey: true }), list)).toEqual({ type: "pasteIndex", index: 1 });
    expect(type(cmd("6"), list)).toBe("consume");
    expect(type(cmd("1"), { ...list, itemCount: 0 })).toBe("consume");
    expect(type(cmd("0"), list)).toBe("ignore");
    for (const mods of [{ shiftKey: true }, { altKey: true }, { ctrlKey: true }]) {
      expect(type(key("3", { metaKey: true, code: "Digit3", ...mods }), list)).toBe("ignore");
    }
  });

  test("Command+C copies a card only when the user has no text selection", () => {
    expect(type(cmd("C"), list)).toBe("copy");
    expect(type(cmd("c"), search)).toBe("copy");
    expect(type(cmd("c"), { ...list, selectedIndex: null })).toBe("ignore");
    expect(type(cmd("c"), { ...search, hasTextSelection: true })).toBe("ignore");
    expect(type(cmd("c"), { ...list, panel: "preview", hasTextSelection: true })).toBe("ignore");
    for (const mods of [{ shiftKey: true }, { altKey: true }, { ctrlKey: true }]) {
      expect(type(key("c", { metaKey: true, ...mods }), list)).toBe("ignore");
    }
  });

  test("pending search consumes every paste and copy shortcut and flushes the query", () => {
    for (const ctx of [list, search, { ...search, itemCount: 0, selectedIndex: null }]) {
      for (const input of [key("Enter"), shiftedReturn, cmd("9"), cmd("c")]) {
        expect(type(input, { ...ctx, searchPending: true })).toBe("flushSearch");
      }
    }
    expect(type(cmd("c"), { ...search, searchPending: true, hasTextSelection: true })).toBe("ignore");
  });

  test("editors, batch mode, settings and dialogs disable single-entry shortcuts", () => {
    for (const ctx of [
      { ...list, inField: true },
      { ...list, panel: "edit" as const, listKeys: false },
      { ...list, panel: "translate" as const, listKeys: false },
      { ...list, panel: "favorites" as const, listKeys: false },
      { ...list, blocked: true },
      { ...list, dialogOpen: true },
      { ...batch, selectedIndex: 0 },
    ]) {
      for (const input of [key("Enter"), shiftedReturn, cmd("1"), cmd("c")]) {
        expect(type(input, { ...ctx, searchPending: true })).toBe("ignore");
      }
    }
  });
});
