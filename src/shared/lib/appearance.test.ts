// @vitest-environment happy-dom
import type { Window } from "happy-dom";
import { afterEach, describe, expect, it } from "vitest";
import {
  applyEffectiveTheme, mirrorAppearance, mirroredAppearance, normalizeAppearance, resolveInitialTheme,
} from "./appearance";

const device = (window as unknown as Window).happyDOM.settings.device;
const initialScheme = device.prefersColorScheme;

afterEach(() => {
  window.localStorage.clear();
  device.prefersColorScheme = initialScheme;
  delete document.documentElement.dataset.theme;
  document.documentElement.style.removeProperty("color-scheme");
});

describe("appearance", () => {
  it("normalizes missing and unknown preferences to System", () => {
    for (const value of [undefined, null, "", "  ", " dark ", "Dark", "other", "system"]) {
      expect(normalizeAppearance(value)).toBe("system");
    }
    expect(normalizeAppearance("light")).toBe("light");
    expect(normalizeAppearance("dark")).toBe("dark");
  });

  it.each(["light", "dark"] as const)("uses the WebView's %s scheme for System or a missing mirror", (scheme) => {
    device.prefersColorScheme = scheme;
    expect(mirroredAppearance()).toBe("system");
    expect(resolveInitialTheme(mirroredAppearance())).toBe(scheme);
    mirrorAppearance("system");
    expect(resolveInitialTheme(mirroredAppearance())).toBe(scheme);
  });

  it.each(["light", "dark"] as const)("restores a cached %s override before the backend responds", (preference) => {
    device.prefersColorScheme = preference === "light" ? "dark" : "light";
    mirrorAppearance(preference);
    expect(mirroredAppearance()).toBe(preference);
    expect(resolveInitialTheme(mirroredAppearance())).toBe(preference);
  });

  it("updates both CSS theme controls in either direction", () => {
    const root = document.documentElement;
    for (const theme of ["dark", "light"] as const) {
      applyEffectiveTheme(theme);
      expect(root.dataset.theme).toBe(theme);
      expect(root.style.colorScheme).toBe(theme);
    }
  });

  it("uses the system scheme after a reset discards an explicit override", () => {
    device.prefersColorScheme = "light";
    mirrorAppearance("dark");
    expect(resolveInitialTheme(mirroredAppearance())).toBe("dark");
    mirrorAppearance("system");
    expect(resolveInitialTheme(mirroredAppearance())).toBe("light");
  });
});
