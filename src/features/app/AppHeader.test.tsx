// @vitest-environment happy-dom
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { expect, test } from "vitest";
import { translations, type TranslationKey } from "../../locales";
import AppHeader from "./AppHeader";

test("the active sequential button uses singular and plural labels", () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  const render = (count: number, hotkey: string) => flushSync(() => root.render(
    <AppHeader t={(key) => translations.en[key as TranslationKey]} showSettings={false} isWindowPinned={false}
      pinMode="normal" favoritesOpen={false} sequentialActive sequentialCount={count} sequentialHotkey={hotkey}
      onBack={() => {}} onToggleFavorites={() => {}} onToggleSequential={() => {}} onTogglePinned={() => {}}
      onSelectPinMode={() => {}} onCloseWindow={() => {}} onClearHistory={() => {}} onOpenSettings={() => {}} />
  ));
  const label = () => container.querySelector(".sequential-button")?.getAttribute("aria-label");
  try {
    render(1, "⌥V");
    expect(label()).toBe("Sequential Paste active (1 item) · Press ⌥V");
    render(1, "");
    expect(label()).toBe("Sequential Paste active (1 item)");
    render(2, "⌥V");
    expect(label()).toBe("Sequential Paste active (2 items) · Press ⌥V");
  } finally {
    flushSync(() => root.unmount());
  }
});
