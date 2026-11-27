import { expect, test } from "vitest";
import { hasEditedClipboardText } from "./EditPanel";

test("clipboard edits detect changes against the loaded full text", () => {
  expect(hasEditedClipboardText(null, null)).toBe(false);
  expect(hasEditedClipboardText("Full text\nSecond line", "Full text\nSecond line")).toBe(false);
  expect(hasEditedClipboardText("Full text", "Full text\nSecond line")).toBe(true);
  expect(hasEditedClipboardText("", "Full text\nSecond line")).toBe(true);
  expect(hasEditedClipboardText("Full text\nSecond line ", "Full text\nSecond line")).toBe(true);
});
