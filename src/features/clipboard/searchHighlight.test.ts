import { describe, expect, it } from "vitest";
import { findMatchedFilePath, hasMatch, matchedExcerpt, splitMatches } from "./searchHighlight";

describe("splitMatches", () => {
  it("marks every occurrence", () => {
    expect(splitMatches("固定在这里，再固定", "固定")).toEqual([
      { text: "固定", match: true },
      { text: "在这里，再", match: false },
      { text: "固定", match: true },
    ]);
  });

  it("ignores case and keeps the original letters", () => {
    expect(splitMatches("Open the PDF", "pdf")).toEqual([
      { text: "Open the ", match: false },
      { text: "PDF", match: true },
    ]);
  });

  it("treats the term literally", () => {
    expect(hasMatch(splitMatches("a+b=c", "a+b"))).toBe(true);
    expect(hasMatch(splitMatches("aab", "a+b"))).toBe(false);
  });

  it("leaves text alone without a term or a match", () => {
    expect(splitMatches("固定", "  ")).toEqual([{ text: "固定", match: false }]);
    expect(hasMatch(splitMatches("固定", "移动"))).toBe(false);
  });
});

describe("findMatchedFilePath", () => {
  const paths = ["/folder-only/other.txt", "C:\\parent\\报告[FINAL].pdf", "/reports/报告[final].mov"];

  it("shows the first matching basename and marks the original text", () => {
    expect(findMatchedFilePath(paths, " 报告[final] ")).toEqual({
      path: paths[1],
      parts: [{ text: "报告[FINAL]", match: true }, { text: ".pdf", match: false }],
    });
  });

  it("ignores directories and shows the original first file when no name matches", () => {
    for (const term of ["folder-only", "parent", "reports", " "]) {
      expect(findMatchedFilePath(paths, term)).toEqual({ path: paths[0], parts: null });
    }
  });
});

describe("matchedExcerpt", () => {
  it("brings a match from later in the body into the visible preview", () => {
    const body = `${"earlier text\n".repeat(20)}Before the 报告 [Final] after it`;
    expect(matchedExcerpt(body, "报告 [final]")).toEqual([
      { text: "Before the ", match: false },
      { text: "报告 [Final]", match: true },
      { text: " after it", match: false },
    ]);
    expect(matchedExcerpt(body, "missing")).toBeNull();
  });
});
