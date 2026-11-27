// @vitest-environment happy-dom
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { expect, test } from "vitest";
import { translations, type TranslationKey } from "../../locales";
import type { EntryKey } from "../../shared/lib/clipboardTypes";
import type { ClipboardEntry } from "../../shared/types";
import BatchPanel from "./BatchPanel";

const t = (key: string) => translations.en[key as TranslationKey];

test("Paste One by One submits the selected identities in their displayed order", async () => {
  const items: ClipboardEntry[] = [7, 2].map((id) => ({
    id, content_type: "text", content: String(id), preview: String(id), source_app: "Notes",
    timestamp: id + 10, created_at: id + 10, is_pinned: false,
  }));
  const container = document.createElement("div");
  const root = createRoot(container);
  const errors: unknown[] = [];
  let submitted: EntryKey[] = [];
  const render = (selection: ClipboardEntry[]) => flushSync(() => root.render(
    <BatchPanel
      t={t} items={selection} allVisibleSelected={false} canSelectAll={false} hasMore={false} deleting={false}
      onToggleSelectAll={() => {}} onDeleteSelected={() => {}} onReorder={() => {}} onRemove={() => {}}
      deleteAfterPaste={false} moveToTopAfterPaste={false} onPasteFailed={(error) => errors.push(error)}
      onStartSequential={async (entries) => { submitted = entries; }} onClose={() => {}}
    />
  ));

  try {
    render([]);
    const sequential = container.querySelector<HTMLButtonElement>(".batch-sequential-button")!;
    expect(sequential.textContent).toBe("Paste One by One");
    expect(sequential.disabled).toBe(true);
    render(items);
    await expect.poll(() => sequential.disabled).toBe(false);
    flushSync(() => sequential.click());
    await Promise.resolve();
    expect(submitted).toEqual([{ id: 7, createdAt: 17 }, { id: 2, createdAt: 12 }]);
    expect(errors).toEqual([]);
  } finally {
    flushSync(() => root.unmount());
  }
});
