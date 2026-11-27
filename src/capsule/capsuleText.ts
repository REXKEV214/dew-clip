import { translations, type TranslationKey } from "../locales";
import { getHotkeyDisplayTokens } from "../shared/lib/hotkeyDisplay";

export type CapsuleLocale = "zh" | "en";

type SequentialNotice =
  | { kind: "queued"; position: number; newRound: boolean }
  | { kind: "pasted"; current: number; total: number }
  | { kind: "done" | "round_done"; total: number }
  | { kind: "skipped"; remaining: number; hotkey: string | null; roundEnded: boolean }
  | { kind: "empty" }
  | { kind: "failed"; reason: "permission" | "modifiers" | "paste" };

export interface CapsulePayload {
  kind: string;
  preview: string;
  appName: string;
  appIcon: string | null;
  imagePath: string | null;
  fileCount: number;
  sequential?: SequentialNotice | null;
  style: "glass" | "island" | "black";
  language: string;
  count: number;
  fresh: boolean;
  seq: number;
  width: number;
  height: number;
  topInset: number;
}

const TITLES: Record<CapsuleLocale, Record<string, string>> = {
  zh: {
    text: "已复制文本",
    rich_text: "已复制文本",
    code: "已复制代码",
    url: "已复制链接",
    image: "已复制图片",
    video: "已复制视频",
    file: "已复制文件",
    files: "已复制 {n} 个文件",
    sample: "复制提示弹窗",
    other: "已复制",
  },
  en: {
    text: "Text copied",
    rich_text: "Text copied",
    code: "Code copied",
    url: "Link copied",
    image: "Image copied",
    video: "Video copied",
    file: "File copied",
    files: "{n} files copied",
    sample: "Copy Notice",
    other: "Copied",
  },
};

const SAMPLE_SUBTITLE: Record<CapsuleLocale, string> = {
  zh: "每次复制，都会在这里出现",
  en: "Shows up here every time you copy",
};

const FROM: Record<CapsuleLocale, string> = { zh: "来自 {app}", en: "From {app}" };

export const capsuleLocale = (language: string): CapsuleLocale => {
  if (language === "zh" || language === "en") return language;
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
};

export const capsuleTitle = (payload: Pick<CapsulePayload, "kind" | "fileCount" | "sequential">, locale: CapsuleLocale) => {
  const t = translations[locale];
  const notice = payload.sequential;
  switch (notice?.kind) {
    case "queued":
      return notice.newRound ? t.sequential_new_round : t.sequential_queued.replace("{position}", String(notice.position));
    case "pasted":
      return t.sequential_progress.replace("{current}", String(notice.current)).replace("{total}", String(notice.total));
    case "done": return t.sequential_done;
    case "round_done": return t.sequential_round_done.replace("{total}", String(notice.total));
    case "skipped": return t.sequential_skipped;
    case "empty": return t.sequential_empty;
    case "failed": return t.sequential_paste_failed;
  }
  const titles = TITLES[locale];
  if (payload.kind === "file" && payload.fileCount > 1) {
    return titles.files.replace("{n}", String(payload.fileCount));
  }
  return titles[payload.kind] ?? titles.other;
};

/** Generic names the capture falls back to when it can't tell the app. */
const UNKNOWN_APPS = new Set(["", "unknown", "macos app"]);

export const knownAppName = (name: string) => (UNKNOWN_APPS.has(name.trim().toLowerCase()) ? "" : name.trim());

export const capsuleShowsCheck = ({ sequential }: Pick<CapsulePayload, "sequential">) =>
  sequential?.kind !== "failed" && sequential?.kind !== "skipped" && sequential?.kind !== "empty";

export const capsuleSubtitle = (
  payload: Pick<CapsulePayload, "kind" | "preview" | "appName" | "sequential">,
  locale: CapsuleLocale
) => {
  const t = translations[locale];
  const notice = payload.sequential;
  switch (notice?.kind) {
    case "pasted": {
      const next = payload.preview.trim() || t[`type_${payload.kind}` as TranslationKey];
      return next ? t.sequential_next_preview.replace("{preview}", next) : "";
    }
    case "done": return notice.total === 1 ? t.sequential_done_one : t.sequential_done_total.replace("{total}", String(notice.total));
    case "round_done": return t.sequential_round_again;
    case "skipped": {
      if (notice.remaining === 0) return notice.roundEnded ? t.sequential_inactive : t.sequential_empty;
      const hotkey = getHotkeyDisplayTokens(notice.hotkey ?? undefined, { preferMacSymbols: true })
        .map(({ label }) => label).join("");
      return hotkey
        ? t.sequential_skip_retry.replace("{hotkey}", hotkey)
        : t.sequential_status_count.replace("{count}", String(notice.remaining));
    }
    case "failed": {
      if (notice.reason === "modifiers") return t.sequential_release_modifiers;
      if (notice.reason === "permission") return t.sequential_permission_hint;
      return "";
    }
  }
  if (payload.kind === "sample") return SAMPLE_SUBTITLE[locale];
  if (payload.preview.trim()) return payload.preview.trim();
  const app = knownAppName(payload.appName);
  return app ? FROM[locale].replace("{app}", app) : "";
};
