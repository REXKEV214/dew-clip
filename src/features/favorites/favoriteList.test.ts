import { describe, expect, it } from "vitest";
import type { FavoriteItem } from "../../shared/types";
import { inTab, matchesFavoriteSearch, matchesVisible, mergeOrder } from "./favoriteList";

const item = (id: number, content_type: string, tags: string[] = []): FavoriteItem => ({
  id, content_type, content: "", preview: "", source_app: "",
  sort_order: id, created_at: 0, paths: [], title: "", tags,
});

describe("favorite tabs", () => {
  it("puts every type in exactly one tab, video and file under media", () => {
    for (const type of ["text", "rich_text", "code", "url"]) {
      expect(inTab(item(1, type), "text")).toBe(true);
      expect(inTab(item(1, type), "media")).toBe(false);
    }
    for (const type of ["image", "video", "file"]) {
      expect(inTab(item(1, type), "media")).toBe(true);
      expect(inTab(item(1, type), "text")).toBe(false);
    }
  });

  it("filters by tag within the tab", () => {
    expect(matchesVisible(item(1, "text", ["a"]), "text", "a")).toBe(true);
    expect(matchesVisible(item(1, "text", ["b"]), "text", "a")).toBe(false);
    expect(matchesVisible(item(1, "image", ["a"]), "text", "a")).toBe(false);
    expect(matchesVisible(item(1, "text"), "text", null)).toBe(true);
  });
});

describe("favorite search", () => {
  it("matches the title, partial tags, preview and full text without regard to case", () => {
    const phrase = {
      ...item(1, "rich_text", ["Support", "售后"]),
      title: "Order update",
      preview: "An item shipped",
      content: "It is expected to arrive tomorrow.",
    };
    for (const query of [" ORDER ", "upp", "售后", "SHIPPED", "TOMORROW", " "]) {
      expect(matchesFavoriteSearch(phrase, query)).toBe(true);
    }
    expect(matchesFavoriteSearch(phrase, "missing")).toBe(false);
  });

  it("matches any file basename, including images and videos", () => {
    const files = {
      ...item(1, "file"),
      paths: ["/favorites/private-id/notes.txt", "/favorites/private-id/报告 [Final].pdf"],
    };
    expect(matchesFavoriteSearch(files, "NOTES.txt")).toBe(true);
    expect(matchesFavoriteSearch(files, "报告 [final]")).toBe(true);
    for (const [kind, path] of [["image", "/favorites/private-id/Logo.PNG"], ["video", "/favorites/private-id/Clip.MOV"]]) {
      const media = { ...item(1, kind, ["Media"]), title: "Reference", paths: [path] };
      expect(matchesFavoriteSearch(media, path.split("/").pop()!.toLowerCase())).toBe(true);
      expect(matchesFavoriteSearch(media, "REFERENCE")).toBe(true);
      expect(matchesFavoriteSearch(media, "media")).toBe(true);
    }
  });

  it("ignores storage directories and internal path text for media", () => {
    const media = {
      ...item(1, "file"),
      paths: ["/data/favorites/f32cc353-81ca-45a0-b82d-fb8cd3cd324c/agenda.pdf"],
      content: "f32cc353-81ca-45a0-b82d-fb8cd3cd324c/agenda.pdf",
      preview: "legacy-preview",
    };
    for (const query of ["favorites", "f32cc353", "legacy-preview"]) {
      expect(matchesFavoriteSearch(media, query)).toBe(false);
    }
  });

  it("combines the search with the current tab and tag", () => {
    const phrase = { ...item(1, "text", ["Work", "Urgent"]), title: "Report" };
    const file = { ...item(2, "file", ["Work"]), paths: ["/documents/report.pdf"] };
    expect([phrase, file].filter((entry) => matchesVisible(entry, "text", "Work", "report"))).toEqual([phrase]);
    expect([phrase, file].filter((entry) => matchesVisible(entry, "media", "Work", "report"))).toEqual([file]);
    expect(matchesVisible(file, "media", "Urgent", "report")).toBe(false);
    expect(matchesVisible(phrase, "text", "Work", "missing")).toBe(false);
  });
});

describe("mergeOrder", () => {
  const ids = (list: FavoriteItem[]) => list.map((favorite) => favorite.id);

  it("reorders the filtered items inside their own slots", () => {
    const all = [
      item(1, "text", ["p"]),
      item(2, "text"),
      item(3, "image", ["p"]),
      item(4, "text", ["p"]),
      item(5, "text"),
      item(6, "text", ["p"]),
    ];
    const [one, , , four, , six] = all;
    // Under "p" in 常用语, drag the last visible item to the top.
    const merged = mergeOrder(all, "text", "p", [six, one, four]);
    expect(ids(merged)).toEqual([6, 2, 3, 1, 5, 4]);
  });

  it("keeps the other tab's slots when nothing is filtered", () => {
    const all = [item(1, "text"), item(2, "video"), item(3, "text"), item(4, "file")];
    expect(ids(mergeOrder(all, "media", null, [all[3], all[1]]))).toEqual([1, 4, 3, 2]);
  });
});
