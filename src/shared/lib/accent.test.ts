// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { ACCENTS, ACCENT_NAMES, applyAccent, DEFAULT_ACCENT, normalizeAccent } from "./accent";

describe("accent", () => {
  it("offers six colours, blue first and default", () => {
    expect(ACCENT_NAMES).toHaveLength(6);
    expect(ACCENT_NAMES[0]).toBe("blue");
    expect(DEFAULT_ACCENT).toBe("blue");
  });

  it("falls back to blue for unknown or missing values", () => {
    expect(normalizeAccent(undefined)).toBe("blue");
    expect(normalizeAccent("pink")).toBe("blue");
    expect(normalizeAccent("toString")).toBe("blue");
    expect(normalizeAccent("green")).toBe("green");
  });

  it("writes the CSS variables, with rgb triplets", () => {
    const root = document.createElement("div");
    applyAccent("blue", root);
    expect(root.style.getPropertyValue("--accent")).toBe("#0a84ff");
    expect(root.style.getPropertyValue("--accent-rgb")).toBe("10, 132, 255");
    expect(root.style.getPropertyValue("--accent-strong-rgb")).toBe("0, 102, 224");
  });

  it.each(ACCENT_NAMES)("writes readable light and dark inks for %s", (name) => {
    const root = document.createElement("div");
    applyAccent(name, root);
    expect(root.style.getPropertyValue("--accent-strong")).toBe(ACCENTS[name].strong);
    expect(root.style.getPropertyValue("--accent-ink-dark")).toBe(ACCENTS[name].darkInk);
    expect(ACCENTS[name].darkInk).toMatch(/^#[\da-f]{6}$/);
    expect(ACCENTS[name].darkInk).not.toBe(ACCENTS[name].strong);
  });
});
