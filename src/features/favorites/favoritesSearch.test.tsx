// @vitest-environment happy-dom
import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Reorder } from "framer-motion";
import App from "../../App";
import { translations, type TranslationKey } from "../../locales";
import { installBrowserAcceleratorBlocker } from "../../shared/lib/blockBrowserAccelerators";
import type { FavoriteItem } from "../../shared/types";
import { FavoriteRow } from "./FavoritesPanel";
import type { UnsavedChangesHandle } from "../app/useWindowClose";

let host: HTMLDivElement;
let root: Root;
let uninstall: () => void;

const mainSearch = () => host.querySelector<HTMLInputElement>(".searchbox-input")!;
const favoriteSearch = () => host.querySelector<HTMLInputElement>(".favorite-search input")!;
const searchEmpty = () => host.querySelector(".side-panel .empty-title")?.textContent;
const press = (name: string, modifiers: KeyboardEventInit = {}) => {
  const event = new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...modifiers });
  act(() => document.activeElement!.dispatchEvent(event));
  return event;
};
const edit = (input: HTMLInputElement | HTMLTextAreaElement, value: string) => act(() => {
  Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
});

beforeEach(() => {
  vi.useFakeTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  uninstall = installBrowserAcceleratorBlocker({ allowDevReload: false });
  root = createRoot(host);
  act(() => root.render(<App />));
  act(() => host.querySelector<HTMLButtonElement>(".favorites-button")!.click());
});

afterEach(() => {
  act(() => root.unmount());
  uninstall();
  host.remove();
  vi.useRealTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: false });
});

test("Command+F chooses the search in the focused panel and selects its term", () => {
  const tab = host.querySelector<HTMLButtonElement>(".favorite-tabs button")!;
  edit(favoriteSearch(), "Report");
  act(() => tab.focus());
  expect(press("f", { metaKey: true }).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(favoriteSearch());
  expect([favoriteSearch().selectionStart, favoriteSearch().selectionEnd]).toEqual([0, 6]);

  const mainButton = host.querySelector<HTMLButtonElement>(".type-tabs button")!;
  act(() => mainButton.focus());
  expect(press("f", { metaKey: true }).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(mainSearch());
});

test("Escape closes the editor, clears favorites search, then closes the panel", () => {
  edit(favoriteSearch(), "Report");
  act(() => host.querySelector<HTMLButtonElement>(".side-panel .panel-title button")!.click());
  expect(host.querySelector(".favorite-editor")).not.toBeNull();
  expect(document.activeElement).toBe(host.querySelector(".favorite-textarea"));
  expect(press("f", { metaKey: true }).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(favoriteSearch());
  press("Escape");
  expect(host.querySelector(".favorite-editor")).toBeNull();
  expect(favoriteSearch().value).toBe("Report");
  press("Escape");
  expect(favoriteSearch().value).toBe("");
  expect(document.activeElement).toBe(favoriteSearch());
  press("Escape");
  expect(host.querySelector(".side-panel")).toBeNull();
});

test("Escape in the history search clears that field independently of favorites", () => {
  edit(favoriteSearch(), "Report");
  act(() => mainSearch().focus());
  edit(mainSearch(), "history");
  press("Escape");
  expect(mainSearch().value).toBe("");
  expect(favoriteSearch().value).toBe("Report");
});

test("a new favorite asks before closing if it has a title, tag draft or text", () => {
  act(() => host.querySelector<HTMLButtonElement>(".side-panel .panel-title button")!.click());
  const title = host.querySelector<HTMLInputElement>(".favorite-title-input")!;
  const tag = host.querySelector<HTMLInputElement>(".favorite-tag-box input")!;
  const text = host.querySelector<HTMLTextAreaElement>(".favorite-textarea")!;
  const dialog = () => document.body.querySelector('[aria-modal="true"]');
  for (const [input, value] of [[title, "A phrase"], [tag, "Work"], [text, "Send tomorrow"]] as const) {
    act(() => input.focus());
    edit(input, value);
    expect(press("w", { metaKey: true }).defaultPrevented).toBe(true);
    expect(dialog()?.getAttribute("aria-label")).toBe("Discard unsaved changes?");
    expect(input.value).toBe(value);
    press("Escape");
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(input);
    edit(input, "");
  }
});

test.each(["text", "file"] as const)("existing favorite %s edits detect metadata and revert cleanly", (kind) => {
  const favorite: FavoriteItem = {
    id: 1, content_type: kind, content: "Original phrase",
    preview: "", source_app: "Notes", sort_order: 1, created_at: 1,
    paths: kind === "file" ? ["/documents/report.pdf"] : [],
    title: "Original title", tags: ["Work"],
  };
  const editor = createRef<UnsavedChangesHandle>();
  const noAction = () => {};
  act(() => root.render(
    <Reorder.Group axis="y" values={[favorite]} onReorder={noAction}>
      <FavoriteRow item={favorite} t={(key) => translations.en[key as TranslationKey]} query=""
        editing editorRef={editor} existingTags={[]} saving={false} pushToast={noAction}
        onPaste={noAction} onEdit={noAction} onSaveEdit={noAction} onCancelEdit={noAction}
        onDelete={noAction} onDragEnd={noAction} />
    </Reorder.Group>
  ));
  const dirty = () => editor.current?.hasUnsavedChanges();
  expect(dirty()).toBe(false);
  const title = host.querySelector<HTMLInputElement>(".favorite-title-input")!;
  edit(title, "  Original title  ");
  expect(dirty()).toBe(false);
  edit(title, "Different");
  expect(dirty()).toBe(true);
  edit(title, favorite.title!);
  expect(dirty()).toBe(false);
  const text = host.querySelector<HTMLTextAreaElement>(".favorite-textarea");
  if (kind === "text") {
    edit(text!, "");
    expect(dirty()).toBe(true);
    edit(text!, favorite.content);
    expect(dirty()).toBe(false);
  } else {
    expect(text).toBeNull();
  }
  act(() => host.querySelector<HTMLButtonElement>(".favorite-tag-chip-remove")!.click());
  expect(dirty()).toBe(true);
  const tag = host.querySelector<HTMLInputElement>(".favorite-tag-box input")!;
  edit(tag, "Work");
  act(() => tag.focus());
  press("Enter");
  expect(dirty()).toBe(false);
});

test("favorites filters immediately across tabs, waits for the IME, and clears with the button", () => {
  expect(favoriteSearch().getAttribute("aria-label")).toBe("Search favorites");
  expect(searchEmpty()).toBe("No saved phrases yet");
  act(() => favoriteSearch().focus());
  act(() => favoriteSearch().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  edit(favoriteSearch(), "baogao");
  expect(searchEmpty()).toBe("No saved phrases yet");
  expect(press("Escape").defaultPrevented).toBe(false);
  act(() => {
    favoriteSearch().value = "报告";
    favoriteSearch().dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "报告" }));
  });
  expect(searchEmpty()).toBe("No matching favorites");

  const mediaTab = host.querySelectorAll<HTMLButtonElement>(".favorite-tabs button")[1];
  act(() => mediaTab.click());
  expect(favoriteSearch().value).toBe("报告");
  expect(searchEmpty()).toBe("No matching favorites");
  act(() => host.querySelector<HTMLButtonElement>(".favorite-search button")!.click());
  expect(favoriteSearch().value).toBe("");
  expect(searchEmpty()).toBe("No saved images or files yet");
});

test("visible matches highlight title, content, tags and filenames while the search hides drag handles", () => {
  const phrase: FavoriteItem = {
    id: 1, content_type: "text", content: "First line\nSecond line\nThe Report is ready.",
    preview: "First line", source_app: "Notes", sort_order: 1, created_at: 1, paths: [],
    title: "Quarterly REPORT", tags: ["Work", "Quarterly", "Customer", "reports"],
  };
  const file: FavoriteItem = {
    ...phrase, id: 2, content_type: "file", title: "", tags: [],
    paths: ["/favorites/123/notes.txt", "/favorites/123/Report.pdf", "/favorites/123/report.docx"],
  };
  const rows = [phrase, file];
  const actions: number[] = [];
  const noAction = () => {};
  const renderRows = (query: string) => act(() => root.render(
    <Reorder.Group axis="y" values={rows} onReorder={noAction}>
      {rows.map((item) => (
        <FavoriteRow
          key={item.id}
          item={item}
          t={(key) => translations.en[key as TranslationKey]}
          query={query}
          editing={false}
          existingTags={[]}
          saving={false}
          pushToast={noAction}
          onPaste={() => actions.push(item.id)}
          onEdit={noAction}
          onSaveEdit={noAction}
          onCancelEdit={noAction}
          onDelete={noAction}
          onDragEnd={noAction}
        />
      ))}
    </Reorder.Group>
  ));
  renderRows("");
  expect(host.querySelectorAll(".favorite-grip")).toHaveLength(2);
  renderRows("report");
  expect(host.querySelectorAll(".favorite-grip")).toHaveLength(0);
  for (const [selector, text] of [
    [".favorite-item-title", "REPORT"],
    [".favorite-text", "Report"],
    [".favorite-card-tag", "report"],
    [".favorite-name", "Report"],
  ]) {
    expect(host.querySelector(`${selector} mark`)?.textContent).toBe(text);
  }
  const filename = host.querySelector<HTMLElement>(".favorite-name")!;
  expect(filename.textContent).toBe("Report.pdf");
  act(() => filename.click());
  expect(actions).toEqual([file.id]);
  renderRows("");
  expect(host.querySelector(".favorite-name")?.textContent).toBe("notes.txt");
});
