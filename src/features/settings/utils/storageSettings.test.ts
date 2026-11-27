import { expect, it } from "vitest";
import { translations, type TranslationKey } from "../../../locales";
import { formatStorageBytes, storageCleanupMessage, tightensStorageRule } from "./storageSettings";

it("requires confirmation when switching from unlimited or reducing either limit", () => {
  expect(tightensStorageRule("0", "7")).toBe(true);
  expect(tightensStorageRule("365", "7")).toBe(true);
  expect(tightensStorageRule("0", "1024")).toBe(true);
  expect(tightensStorageRule("5120", "500")).toBe(true);
  expect(tightensStorageRule("7", "30")).toBe(false);
  expect(tightensStorageRule("500", "0")).toBe(false);
  expect(tightensStorageRule("500", "500")).toBe(false);
});

it("formats storage amounts with readable units", () => {
  expect(formatStorageBytes(0, "en")).toBe("0 KB");
  expect(formatStorageBytes(128 * 1024, "en")).toBe("128 KB");
  expect(formatStorageBytes(1.5 * 1024 ** 2, "en")).toBe("1.5 MB");
  expect(formatStorageBytes(1024 ** 3, "en")).toBe("1 GB");
});

it("uses the singular cleanup message for one entry", () => {
  const en = (key: string) => translations.en[key as TranslationKey];
  const zh = (key: string) => translations.zh[key as TranslationKey];
  expect(storageCleanupMessage(0, en)).toBe("Removed 0 older items");
  expect(storageCleanupMessage(1, en)).toBe("Removed 1 older item");
  expect(storageCleanupMessage(2, en)).toBe("Removed 2 older items");
  expect(storageCleanupMessage(1, zh)).toBe("已清理 1 条旧记录");
});
