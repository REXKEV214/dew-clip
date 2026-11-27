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

test.each([
  ["en", "Close window", "Back"],
  ["zh", "关闭窗口", "返回"],
] as const)("close is reachable and never starts a drag in %s", (language, label, back) => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const noAction = () => {};
  let closed = 0;
  let mouseDowns = 0;
  const render = (showSettings: boolean, updateAvailable: boolean) => flushSync(() => root.render(
    <div onMouseDown={() => { mouseDowns++; }}>
      <AppHeader t={(key) => translations[language][key as TranslationKey]} showSettings={showSettings}
        isWindowPinned={false} pinMode="normal" favoritesOpen={false} sequentialActive={false}
        sequentialCount={0} sequentialHotkey="" updateAvailable={updateAvailable}
        onBack={noAction} onToggleFavorites={noAction} onToggleSequential={noAction} onTogglePinned={noAction}
        onSelectPinMode={noAction} onCloseWindow={() => { closed++; }} onClearHistory={noAction} onOpenSettings={noAction} />
    </div>
  ));
  try {
    render(true, false);
    const button = host.querySelector<HTMLButtonElement>("header .window-close")!;
    expect(button).toBe(host.querySelector("header")?.firstElementChild);
    expect(button.getAttribute("aria-label")).toBe(label);
    expect(button.title).toBe(`${label} · Esc / ⌘W`);
    expect(button.tabIndex).toBe(0);
    expect(button.querySelector("svg")).not.toBeNull();
    expect(host.querySelector(".back-button")?.getAttribute("aria-label")).toBe(back);
    button.focus();
    expect(document.activeElement).toBe(button);
    const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    button.dispatchEvent(press);
    expect(press.defaultPrevented).toBe(true);
    expect(mouseDowns).toBe(0);
    button.click();
    expect(closed).toBe(1);
    expect(document.activeElement).not.toBe(button);

    render(false, true);
    expect(host.querySelector(".settings-button .settings-update-dot")).not.toBeNull();
    render(false, false);
    expect(host.querySelector(".settings-button .settings-update-dot")).toBeNull();
  } finally {
    flushSync(() => root.unmount());
    host.remove();
  }
});
