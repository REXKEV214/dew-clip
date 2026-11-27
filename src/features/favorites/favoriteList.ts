import type { FavoriteItem } from "../../shared/types";
import { isEditableEntry } from "../../shared/lib/clipboardTypes";
import { fileName } from "../clipboard/searchHighlight";

/** 常用语 holds text; 图片与文件 holds everything else (image, video, file). */
export type FavoriteTab = "text" | "media";
export const FAVORITE_TABS: FavoriteTab[] = ["text", "media"];

export const inTab = (item: FavoriteItem, tab: FavoriteTab) =>
  tab === "media" ? !isEditableEntry(item.content_type) : isEditableEntry(item.content_type);

export const matchesFavoriteSearch = (item: FavoriteItem, query: string): boolean => {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  const matches = (value: string) => value.toLowerCase().includes(needle);
  if (matches(item.title ?? "") || (item.tags ?? []).some(matches)) return true;
  return isEditableEntry(item.content_type)
    ? matches(item.content) || matches(item.preview)
    : item.paths.some((path) => matches(fileName(path)));
};

export const matchesVisible = (item: FavoriteItem, tab: FavoriteTab, tag: string | null, query = "") =>
  inTab(item, tab) && (tag === null || (item.tags ?? []).includes(tag)) && matchesFavoriteSearch(item, query);

/**
 * Put a reordered visible list back into the full one. The visible items
 * trade the slots they already held; every other item keeps its place.
 */
export const mergeOrder = (
  all: FavoriteItem[],
  tab: FavoriteTab,
  tag: string | null,
  reordered: FavoriteItem[]
) => {
  let next = 0;
  return all.map((item) => (matchesVisible(item, tab, tag) ? reordered[next++] ?? item : item));
};
