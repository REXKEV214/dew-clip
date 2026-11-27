// @vitest-environment happy-dom
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { expect, test } from "vitest";
import { translations, type TranslationKey } from "../../locales";
import { useKeyboard } from "../app/useKeyboard";
import QueuePanel from "./QueuePanel";
import type { SequentialItem } from "./useSequentialQueue";

const t = (key: string) => translations.en[key as TranslationKey];
const items: SequentialItem[] = ["Name", "Address"].map((preview, index) => ({
  queueId: index + 10,
  identity: { id: index + 1, createdAt: 100 + index },
  contentType: "text",
  preview,
  mediaPath: null,
  timestamp: 100 + index,
}));

const frame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

test("renders the next item and commits a reorder and removal by queue row", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let ordered = items;
  let removed: number | undefined;
  const render = () => flushSync(() => root.render(
    <QueuePanel t={t} items={ordered} hotkey="⌥V"
      onReorder={(ids) => { ordered = ids.map((id) => items.find((item) => item.queueId === id)!); render(); }}
      onRemove={(id) => { removed = id; ordered = ordered.filter((item) => item.queueId !== id); render(); }}
      onClear={() => { ordered = []; render(); }} onEnd={() => {}} onPaste={() => {}}
    />
  ));
  const pointer = (type: string, clientY: number) => {
    const event = new PointerEvent(type, {
      bubbles: true, clientY, pointerId: 1, pointerType: "mouse", isPrimary: true,
      buttons: type === "pointerup" ? 0 : 1,
    });
    // happy-dom doesn't populate the document-relative coordinates Motion uses.
    return Object.assign(event, { pageX: 0, pageY: clientY });
  };

  render();
  try {
    expect(container.querySelector(".queue-status")?.textContent).toBe("2 queued · Press ⌥V");
    expect(container.querySelector(".is-next")?.textContent).toBe("1NameNext");

    await frame();
    container.querySelector("button[aria-label='Drag to reorder']")!.dispatchEvent(pointer("pointerdown", 5));
    for (const y of [20, 45, 80]) {
      window.dispatchEvent(pointer("pointermove", y));
      await frame();
    }
    window.dispatchEvent(pointer("pointerup", 80));
    await frame();
    await frame();
    expect(ordered.map((item) => item.queueId)).toEqual([11, 10]);
    expect(container.querySelector(".is-next")?.textContent).toBe("1AddressNext");

    container.querySelector<HTMLButtonElement>(".is-next button[aria-label='Remove']")!.click();
    expect(removed).toBe(11);
    expect(container.querySelector(".is-next")?.textContent).toBe("1NameNext");
    container.querySelector<HTMLButtonElement>(".panel-title button")!.click();
    expect(container.querySelector(".queue-status")?.textContent).toBe(t("sequential_empty_hint"));
    expect(container.querySelector<HTMLButtonElement>(".panel-actions button.is-primary")?.disabled).toBe(true);
  } finally {
    flushSync(() => root.unmount());
    container.remove();
  }
});

test("the queue grip handles arrows before they can change the history selection", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const historySelections: (number | null)[] = [];
  const reorders: number[][] = [];
  function HistoryWithQueue() {
    useKeyboard({
      items: items.map((item) => ({
        id: item.identity.id, content_type: item.contentType, content: item.preview,
        source_app: "Notes", timestamp: item.timestamp, preview: item.preview, is_pinned: false,
      })),
      enabled: true, selectedIndex: 0, setSelectedIndex: (index) => historySelections.push(index),
      typeFilter: null, setTypeFilter: () => {}, paste: () => {}, remove: () => {}, focusSearch: () => {},
      dismiss: () => false, panel: "queue", blocked: false, batchCount: 0, canSelectAll: true,
      startBatchSelectAll: () => {}, toggleSelectAll: () => {}, requestBatchDelete: () => {},
    });
    return <QueuePanel t={t} items={items} hotkey="⌥V" onReorder={(ids) => reorders.push(ids)}
      onRemove={() => {}} onClear={() => {}} onEnd={() => {}} onPaste={() => {}} />;
  }
  try {
    flushSync(() => root.render(<HistoryWithQueue />));
    const arrowsDown = () => new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true });
    const handle = container.querySelector<HTMLButtonElement>("button[data-queue-reorder]")!;
    handle.focus();
    handle.dispatchEvent(arrowsDown());
    expect(reorders).toEqual([[11, 10]]);
    expect(historySelections).toEqual([]);
    handle.blur();
    container.dispatchEvent(arrowsDown());
    expect(historySelections).toEqual([1]);
  } finally {
    flushSync(() => root.unmount());
    container.remove();
  }
});
