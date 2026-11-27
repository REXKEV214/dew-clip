// @vitest-environment happy-dom
import { act, useRef, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { translations, type TranslationKey } from "../../locales";
import { installBrowserAcceleratorBlocker } from "../../shared/lib/blockBrowserAccelerators";
import type { ClipboardEntry, Locale } from "../../shared/types";
import ClipboardCard from "../clipboard/ClipboardCard";
import SearchBox, { type SearchBoxHandle } from "./SearchBox";
import { useKeyboard } from "./useKeyboard";
import { useHistorySelection } from "./useHistorySelection";
import { useLiveSearch } from "../../shared/hooks/useLiveSearch";
import { useShortcutBadges } from "./useShortcutBadges";
import { useWindowClose } from "./useWindowClose";
import { useOverlays } from "./useOverlays";
import { ConfirmDialog } from "../../shared/components/Overlays";
import { useSpacePreview } from "../preview/useSpacePreview";

const items: ClipboardEntry[] = [
  { id: 1, content_type: "text", content: "First", source_app: "Notes", timestamp: 1, preview: "", is_pinned: false },
  { id: 2, content_type: "text", content: "Second", source_app: "Notes", timestamp: 2, preview: "", is_pinned: false },
];
const noAction = () => {};
let host: HTMLDivElement;
let root: Root;
let uninstall: () => void;
let used: string[];

const Harness = ({
  blocked = false, batch = false, settled = true, language = "en", rows = items,
  unsaved = false, closingBlocked = false,
}: {
  blocked?: boolean; batch?: boolean; settled?: boolean; language?: Locale; rows?: ClipboardEntry[];
  unsaved?: boolean; closingBlocked?: boolean;
}) => {
  const t = (key: string) => translations[language][key as TranslationKey];
  const { confirmDialog, closeConfirm, openConfirm } = useOverlays(t);
  const search = useLiveSearch(150);
  const field = useRef<SearchBoxHandle>(null);
  const pending = search.draft.trim() !== search.search || !settled;
  const badges = useShortcutBadges(!batch && !blocked);
  const { selectedIndex, setSelectedIndex, clearSelection } = useHistorySelection(
    rows, search.search, null, Boolean(search.search) && !pending && !batch
  );
  const clear = () => {
    search.clear();
    clearSelection();
  };
  const cancelPreview = useSpacePreview({
    enabled: true,
    items: rows,
    selectedIndex,
    onOpen: () => used.push("preview opened"),
    onClose: () => used.push("panel restored"),
  });
  const windowClose = useWindowClose({
    t, openConfirm, closeConfirm,
    blocked: () => closingBlocked,
    hasUnsavedChanges: () => unsaved,
    hideWindow: async () => {
      cancelPreview.current();
      used.push("window hidden");
    },
  });
  useKeyboard({
    items: rows,
    enabled: !batch && !blocked,
    selectedIndex,
    setSelectedIndex,
    typeFilter: null,
    setTypeFilter: noAction,
    paste: (entry) => used.push(`paste ${entry.id}`),
    pastePlain: (entry) => used.push(`plain ${entry.id}`),
    copy: (entry) => used.push(`copy ${entry.id}`),
    remove: noAction,
    focusSearch: () => field.current?.focus(),
    searchPending: pending,
    flushSearch: () => search.flush(search.draft),
    isSearchComposing: search.isComposing,
    ...windowClose,
    dismiss: () => {
      clear();
      return true;
    },
    panel: batch ? "batch" : null,
    blocked,
    batchCount: 0,
    canSelectAll: settled,
    startBatchSelectAll: noAction,
    toggleSelectAll: noAction,
    requestBatchDelete: noAction,
  });
  return (
    <>
      <SearchBox
        ref={field}
        t={(key) => translations[language][key as TranslationKey]}
        value={search.draft}
        onChange={search.change}
        onSubmit={search.flush}
        onClear={clear}
        onCompositionStart={search.startComposition}
        onCompositionEnd={search.endComposition}
        pending={pending}
      />
      <output data-badges={badges} data-selected={selectedIndex ?? undefined}>{search.search}</output>
      <p>Preview text</p>
      <textarea aria-label="Editor" />
      <ConfirmDialog state={confirmDialog} confirmLabel={confirmDialog.confirmLabel ?? t("confirm")}
        cancelLabel={t("cancel")} onClose={closeConfirm} />
    </>
  );
};

const mount = (options: ComponentProps<typeof Harness> = {}) => act(() => root.render(<Harness {...options} />));
const input = () => host.querySelector("input")!;
const applied = () => host.querySelector("output")!.textContent;
const badges = () => host.querySelector("output")!.dataset.badges === "true";
const advance = (ms: number) => act(() => vi.advanceTimersByTime(ms));
const edit = (value: string) => act(() => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), value);
  input().dispatchEvent(new Event("input", { bubbles: true }));
});
const key = (name: string, options: KeyboardEventInit = {}, target: EventTarget = window, type = "keydown") => {
  const event = new KeyboardEvent(type, { key: name, bubbles: true, cancelable: true, ...options });
  act(() => { target.dispatchEvent(event); });
  return event;
};

beforeEach(() => {
  vi.useFakeTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  used = [];
  // main.tsx installs the blocker before React mounts the app's listener.
  uninstall = installBrowserAcceleratorBlocker({ allowDevReload: false });
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  uninstall();
  host.remove();
  window.getSelection()?.removeAllRanges();
  vi.useRealTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: false });
});

describe("the WebView keyboard listeners", () => {
  test("Command+F focuses and selects the search term while WebView accelerators stay blocked", () => {
    mount();
    edit("existing");
    for (const name of ["r", "g", "F3"]) {
      expect(key(name, { metaKey: name !== "F3" }).defaultPrevented).toBe(true);
      expect(document.activeElement).not.toBe(input());
    }
    for (const mods of [{ shiftKey: true }, { altKey: true }, { isComposing: true }]) {
      expect(key("f", { metaKey: true, ...mods }).defaultPrevented).toBe(false);
      expect(document.activeElement).not.toBe(input());
    }
    expect(key("f", { metaKey: true }).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(input());
    expect([input().selectionStart, input().selectionEnd]).toEqual([0, 8]);
    act(() => input().blur());
    mount({ batch: true });
    expect(key("f", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(input());
  });

  test("settings and an open dialog keep focus while the native Find bar stays blocked", () => {
    mount({ blocked: true });
    expect(key("f", { metaKey: true }).defaultPrevented).toBe(true);
    expect(document.activeElement).not.toBe(input());
    mount();
    const dialog = document.createElement("div");
    dialog.setAttribute("aria-modal", "true");
    host.append(dialog);
    expect(key("f", { metaKey: true }).defaultPrevented).toBe(true);
    expect(document.activeElement).not.toBe(input());
  });

  test("Command+W reaches the close entry through the capture listeners and ignores repeat", () => {
    mount({ blocked: true });
    act(() => input().focus());
    expect(key("W", { metaKey: true, repeat: true }, input()).defaultPrevented).toBe(true);
    expect(used).toEqual([]);
    expect(key("W", { metaKey: true }, input()).defaultPrevented).toBe(true);
    expect(used).toEqual(["window hidden"]);
    expect(document.activeElement).not.toBe(input());
  });

  test("Command+W leaves composing in the search or another editor and hotkey recording alone", () => {
    mount();
    for (const field of [input(), host.querySelector("textarea")!]) {
      act(() => {
        field.focus();
        field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
      });
      expect(key("w", { metaKey: true }, field).defaultPrevented).toBe(false);
      expect(used).toEqual([]);
      act(() => field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
    }
    const legacyComposition = new KeyboardEvent("keydown", { key: "w", metaKey: true, bubbles: true, cancelable: true });
    Object.defineProperty(legacyComposition, "keyCode", { value: 229 });
    act(() => input().dispatchEvent(legacyComposition));
    expect(legacyComposition.defaultPrevented).toBe(false);
    const recorder = document.createElement("button");
    recorder.dataset.hotkeyRecording = "true";
    recorder.addEventListener("keydown", () => used.push("recorded"));
    host.append(recorder);
    expect(key("w", { metaKey: true }, recorder).defaultPrevented).toBe(false);
    expect(used).toEqual(["recorded"]);
    recorder.remove();
    expect(key("w", { metaKey: true }, input()).defaultPrevented).toBe(true);
    expect(used).toEqual(["recorded", "window hidden"]);
  });

  test("discard confirmation starts on Cancel, preserves focus on cancel and hides on confirm", async () => {
    mount({ unsaved: true });
    const editor = host.querySelector("textarea")!;
    act(() => editor.focus());
    expect(key("w", { metaKey: true }, editor).defaultPrevented).toBe(true);
    const dialog = () => document.body.querySelector<HTMLElement>('[aria-modal="true"]');
    expect(dialog()?.getAttribute("aria-label")).toBe("Discard unsaved changes?");
    const button = (label: string) => [...dialog()!.querySelectorAll("button")].find((item) => item.textContent === label)!;
    advance(32);
    expect(document.activeElement).toBe(button("Cancel"));
    expect(key("w", { metaKey: true }, button("Cancel")).defaultPrevented).toBe(true);
    expect(used).toEqual([]);
    act(() => button("Cancel").click());
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(editor);
    key("w", { metaKey: true }, editor);
    await act(async () => button("Discard and Close").click());
    expect(used).toEqual(["window hidden"]);
    expect(dialog()).toBeNull();
  });

  test("in-flight work consumes Command+W without confirming or hiding", () => {
    mount({ closingBlocked: true, unsaved: true });
    expect(key("w", { metaKey: true }).defaultPrevented).toBe(true);
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(used).toEqual([]);
  });

  test("closing while Space is held disarms preview before blur can restore another panel", () => {
    mount();
    key("ArrowDown");
    key(" ", { code: "Space" });
    advance(250);
    expect(used).toEqual(["preview opened"]);
    key("w", { metaKey: true });
    act(() => window.dispatchEvent(new Event("blur")));
    key(" ", { code: "Space" }, window, "keyup");
    expect(used).toEqual(["preview opened", "window hidden"]);
  });

  test("a pending search flushes once then stays safe until the results arrive", () => {
    mount();
    edit("new");
    expect(key("2", { metaKey: true }, input()).defaultPrevented).toBe(true);
    expect(applied()).toBe("new");
    expect(used).toEqual([]);

    mount({ settled: false });
    key("Enter", { shiftKey: true }, input());
    key("c", { metaKey: true }, input());
    expect(used).toEqual([]);

    mount();
    key("Enter", {}, input());
    key("Enter", { shiftKey: true }, input());
    key("c", { metaKey: true }, input());
    key("2", { metaKey: true }, input());
    expect(used).toEqual(["paste 1", "plain 1", "copy 1", "paste 2"]);
  });

  test("settled search picks the first result, keeps an arrow-key pick and clears both with Escape", () => {
    const selected = () => host.querySelector("output")!.dataset.selected;
    mount({ settled: false });
    edit("results");
    advance(150);
    expect(selected()).toBeUndefined();
    mount();
    expect(selected()).toBe("0");
    key("ArrowDown", {}, input());
    expect(selected()).toBe("1");
    mount({ rows: [items[1], items[0]] });
    expect(selected()).toBe("0");
    key("Enter", {}, input());
    expect(used).toEqual(["paste 2"]);
    key("Escape", {}, input());
    expect([input().value, applied(), selected()]).toEqual(["", "", undefined]);
  });

  test("Command+C keeps native copying for both input and page selections", () => {
    mount();
    edit("search term");
    act(() => input().select());
    expect(key("c", { metaKey: true }, input()).defaultPrevented).toBe(false);
    expect(applied()).toBe("");
    act(() => input().blur());
    const preview = host.querySelector("p")!;
    const range = document.createRange();
    range.selectNodeContents(preview);
    window.getSelection()!.addRange(range);
    expect(key("c", { metaKey: true }, preview).defaultPrevented).toBe(false);
    expect(used).toEqual([]);
  });
});

test("badges stay hidden outside the main search and disappear when a dialog opens", async () => {
  mount();
  const editor = host.querySelector("textarea")!;
  act(() => editor.focus());
  key("Meta", { metaKey: true }, editor);
  advance(300);
  expect(badges()).toBe(false);
  act(() => input().focus());
  key("Meta", { metaKey: true }, input());
  advance(300);
  expect(badges()).toBe(true);
  act(() => editor.focus());
  expect(badges()).toBe(false);
  act(() => input().focus());
  key("Meta", { metaKey: true }, input());
  advance(300);
  const dialog = document.createElement("div");
  dialog.setAttribute("aria-modal", "true");
  await act(async () => {
    host.append(dialog);
    await vi.advanceTimersByTimeAsync(0);
  });
  expect(badges()).toBe(false);
  key("Meta", { metaKey: true }, input());
  advance(300);
  expect(badges()).toBe(false);
});

describe("live search", () => {
  test("debounces input by 150 ms and clears immediately", () => {
    mount();
    edit("a");
    advance(100);
    edit("ab");
    advance(149);
    expect(applied()).toBe("");
    advance(1);
    expect(applied()).toBe("ab");
    edit("abc");
    edit("");
    expect(applied()).toBe("");
    advance(150);
    expect(applied()).toBe("");
  });

  test("the button flushes a new term and collapses the settled search", () => {
    mount();
    edit("notes");
    const submit = host.querySelector<HTMLButtonElement>(".searchbox-button")!;
    expect(submit.getAttribute("aria-label")).toBe("Search");
    act(() => submit.form!.requestSubmit(submit));
    expect(applied()).toBe("notes");
    expect(submit.getAttribute("aria-label")).toBe("Close search");
    act(() => submit.click());
    expect([input().value, applied()]).toEqual(["", ""]);
  });

  test("waits through composition and immediately searches the text committed by the IME", () => {
    mount({ language: "zh" });
    expect(input().getAttribute("aria-label")).toBe("搜索");
    expect(Array.from(host.querySelectorAll(".searchbox-letters span"), (span) => span.textContent)).toEqual(["搜", "索"]);
    edit("before");
    act(() => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    edit("zhong");
    expect(key("Enter", { isComposing: true }, input()).defaultPrevented).toBe(false);
    expect(key("Enter", {}, input()).defaultPrevented).toBe(false);
    advance(500);
    expect(applied()).toBe("");
    act(() => {
      input().value = "中";
      input().dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "中" }));
    });
    expect(input().value).toBe("中");
    expect(applied()).toBe("中");
    expect(used).toEqual([]);
  });
});

test("badges require a 300 ms hold of Command alone and hide on other keys, release, blur or batch mode", () => {
  mount();
  key("Meta", { metaKey: true });
  advance(299);
  expect(badges()).toBe(false);
  advance(1);
  expect(badges()).toBe(true);
  key("x", { metaKey: true });
  expect(badges()).toBe(false);
  key("Meta", { metaKey: true, repeat: true });
  advance(300);
  expect(badges()).toBe(false);
  key("Meta", {}, window, "keyup");

  for (const hide of [
    () => key("Meta", {}, window, "keyup"),
    () => act(() => window.dispatchEvent(new Event("blur"))),
    () => mount({ batch: true }),
  ]) {
    key("Meta", { metaKey: true });
    advance(300);
    expect(badges()).toBe(true);
    hide();
    expect(badges()).toBe(false);
  }
  key("Meta", { metaKey: true });
  advance(300);
  expect(badges()).toBe(false);
});

test("a video card highlights its matching basename and renders its current shortcut number", () => {
  const video = { ...items[0], content_type: "video", content: "/parent-only/录音 Report.MOV" };
  const renderCard = (searchTerm: string, shortcutNumber?: number) => act(() => root.render(
    <ClipboardCard
      item={video}
      t={(name) => translations.en[name as TranslationKey]}
      selected={false}
      batchMode={false}
      batchChecked={false}
      shortcutNumber={shortcutNumber}
      onPaste={noAction}
      onToggleBatch={noAction}
      onContextMenu={noAction}
      onEdit={noAction}
      onTogglePin={noAction}
      onDelete={noAction}
      onPasteAsText={noAction}
      searchTerm={searchTerm}
    />
  ));
  renderCard("report", 3);
  expect(host.querySelector(".card-file-name mark")?.textContent).toBe("Report");
  expect(host.querySelector(".card.is-search-hit")).not.toBeNull();
  expect(host.querySelector(".card-shortcut")?.textContent).toBe("⌘3");
  renderCard("parent-only");
  expect(host.querySelector(".card-file-name")?.textContent).toBe("录音 Report.MOV");
  expect(host.querySelector("mark, .is-search-hit, .card-shortcut")).toBeNull();
});
