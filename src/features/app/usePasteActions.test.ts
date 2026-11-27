import { expect, test } from "vitest";
import { translations, type TranslationKey } from "../../locales";
import { CONTENT_UNAVAILABLE, stoppedPasteMessage } from "./usePasteActions";

test("a paste stopped before anything was deleted says why", () => {
  expect(stoppedPasteMessage(new Error(`${CONTENT_UNAVAILABLE}: [internal] decrypt failed`))).toBe(
    "content_load_failed"
  );
  expect(stoppedPasteMessage("[validation] paste_target_unavailable")).toBe("paste_target_unavailable");
  expect(stoppedPasteMessage("[internal] paste_keystroke_failed")).toBe("paste_keystroke_failed");
  expect(stoppedPasteMessage("[validation] clipboard_snapshot_failed")).toBe("clipboard_snapshot_failed");
  expect(stoppedPasteMessage("[validation] clipboard_changed_during_paste")).toBe("clipboard_changed_during_paste");
  expect(stoppedPasteMessage("[io] disk full")).toBeNull();
});

test("clipboard errors use shared messages for batch and sequential paste", () => {
  const snapshot = stoppedPasteMessage("[validation] clipboard_snapshot_failed") as TranslationKey;
  const changed = stoppedPasteMessage("[validation] clipboard_changed_during_paste") as TranslationKey;
  expect(translations.en[snapshot]).toBe("Couldn't save the current clipboard for restoration. Paste cancelled.");
  expect(translations.en[changed]).toBe("The clipboard changed during paste. Pasting stopped.");
  expect(translations.zh[snapshot]).toBe("无法保存剪贴板当前内容以便恢复，已取消粘贴。");
  expect(translations.zh[changed]).toBe("粘贴期间剪贴板有了新内容，已停止粘贴。");
});
