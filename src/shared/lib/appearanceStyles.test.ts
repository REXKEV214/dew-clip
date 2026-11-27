// @vitest-environment happy-dom
import { readFileSync } from "node:fs";
import type { Window } from "happy-dom";
import { afterEach, expect, it } from "vitest";
import { applyAccent, ACCENTS } from "./accent";
import { applyEffectiveTheme } from "./appearance";

const root = document.documentElement;
const device = (window as unknown as Window).happyDOM.settings.device;
const initialScheme = device.prefersColorScheme;

afterEach(() => {
  document.head.replaceChildren();
  document.body.replaceChildren();
  delete root.dataset.theme;
  root.removeAttribute("style");
  device.prefersColorScheme = initialScheme;
});

const loadStyles = (relativePath: string) => {
  const css = readFileSync(new URL(relativePath, import.meta.url), "utf-8");
  document.head.appendChild(document.createElement("style")).textContent = css;
};

const addElement = (classes: string, parent = document.body) => {
  const element = document.createElement("div");
  element.className = classes;
  parent.appendChild(element);
  return element;
};

it.each(["../../styles/glass.css", "../../capsule/capsule.css"])(
  "uses the native dark scheme before the script sets an explicit theme in %s",
  (relativePath) => {
    loadStyles(relativePath);
    device.prefersColorScheme = "dark";
    expect(getComputedStyle(root).colorScheme).toBe("dark");
    root.dataset.theme = "light";
    expect(getComputedStyle(root).colorScheme).toBe("light");
  },
);

it("recolors release surfaces and shared controls when the window appearance changes", () => {
  loadStyles("../../styles/glass.css");
  const pause = addElement("recording-pause-banner");
  const shortcut = addElement("card-shortcut");
  const queueNext = addElement("queue-next");
  const favoriteSearch = addElement("expandable-search");
  const favoriteActions = addElement("favorite-actions");
  const storageAndIgnoredApps = addElement("setting-row");
  const pickedAppearance = addElement("segment is-active");
  const appPicker = addElement("modal-panel-app-selector");
  const toast = addElement("toast glass-popover");
  const permission = addElement("permission-icon");

  applyAccent("purple");
  const colors = () => [
    getComputedStyle(pause).color,
    getComputedStyle(shortcut).color,
    getComputedStyle(queueNext).color,
    getComputedStyle(favoriteSearch).backgroundColor,
    getComputedStyle(favoriteActions).backgroundColor,
    getComputedStyle(storageAndIgnoredApps).borderBottomColor,
    getComputedStyle(pickedAppearance).backgroundColor,
    getComputedStyle(appPicker).backgroundColor,
    getComputedStyle(toast).backgroundColor,
    getComputedStyle(permission).color,
  ];
  applyEffectiveTheme("light");
  const light = colors();
  applyEffectiveTheme("dark");
  const dark = colors();
  for (const [index, color] of dark.entries()) {
    expect(color).toBeTruthy();
    expect(color).not.toBe(light[index]);
  }
  expect(getComputedStyle(queueNext).color).toBe(ACCENTS.purple.darkInk);
  applyEffectiveTheme("light");
  expect(colors()).toEqual(light);
});

it("changes glass capsule ink while both black styles keep the same colors", () => {
  loadStyles("../../capsule/capsule.css");
  applyAccent("green");
  const glass = addElement("capsule is-glass");
  const glassBadge = addElement("capsule-count", glass);
  // Both black capsule styles use the same front-end class.
  const black = addElement("capsule is-island");
  const blackBadge = addElement("capsule-count", black);
  const blackColors = () => [getComputedStyle(black).color, getComputedStyle(blackBadge).color];

  applyEffectiveTheme("light");
  const lightBlackColors = blackColors();
  const lightGlassInk = getComputedStyle(glass).color;
  expect(getComputedStyle(glassBadge).color).toBe(ACCENTS.green.strong);
  applyEffectiveTheme("dark");
  expect(getComputedStyle(glass).color).not.toBe(lightGlassInk);
  expect(getComputedStyle(glassBadge).color).toBe(ACCENTS.green.darkInk);
  expect(blackColors()).toEqual(lightBlackColors);
});
