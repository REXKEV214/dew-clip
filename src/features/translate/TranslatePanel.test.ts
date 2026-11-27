import { expect, it } from "vitest";
import { hasEditedTranslation, loadTranslationSource } from "./TranslatePanel";

it("rejects translation source loading when the full-content command is unavailable", async () => {
  await expect(loadTranslationSource(1, 10)).rejects.toThrow();
});

it("only treats user changes to source or translation as unsaved", () => {
  expect(hasEditedTranslation(null, null, "", "", false)).toBe(false);
  expect(hasEditedTranslation("你好", "你好", "Hello", "Hello", false)).toBe(false);
  expect(hasEditedTranslation("您好", "你好", "Hello", "Hello", false)).toBe(true);
  expect(hasEditedTranslation("您好", "你好", "", "", true)).toBe(true);
  expect(hasEditedTranslation("你好", "你好", "Hi", "Hello", false)).toBe(true);
  expect(hasEditedTranslation("你好", "你好", "Hi", "Hello", true)).toBe(false);
  expect(hasEditedTranslation("你好", "你好", "Hello", "Hello", false)).toBe(false);
});
