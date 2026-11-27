import type { Translate } from "../../../shared/types";

export const RETENTION_OPTIONS = [
  ["7", "retention_7d"], ["30", "retention_30d"], ["90", "retention_90d"],
  ["365", "retention_365d"], ["0", "retention_forever"],
] as const;

export const SIZE_OPTIONS = [
  ["0", "storage_limit_unlimited"], ["500", "storage_limit_500mb"], ["1024", "storage_limit_1gb"],
  ["2048", "storage_limit_2gb"], ["5120", "storage_limit_5gb"],
] as const;

export const tightensStorageRule = (previous: string, next: string) =>
  Number(next) > 0 && (Number(previous) === 0 || Number(next) < Number(previous));

export interface StorageUsage {
  databaseBytes: number;
  historyAttachmentsBytes: number;
  favoritesMediaBytes: number;
  thumbnailCacheBytes: number;
  totalBytes: number;
  protectedOverLimit: boolean;
}

export const storageCleanupMessage = (count: number, t: Translate) =>
  t(count === 1 ? "storage_cleaned_toast_one" : "storage_cleaned_toast").replace("{count}", String(count));

export const formatStorageBytes = (bytes: number, locale: string) => {
  const kb = 1024;
  const mb = 1024 ** 2;
  const gb = 1024 ** 3;
  const [divisor, unit] = bytes >= gb ? [gb, "gigabyte"] : bytes >= mb ? [mb, "megabyte"] : [kb, "kilobyte"];
  return new Intl.NumberFormat(locale, { style: "unit", unit, maximumFractionDigits: 1 })
    .format(bytes / divisor).replace("kB", "KB");
};
