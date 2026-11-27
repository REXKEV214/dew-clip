/** A run of card text, marked when it is the searched word. */
export interface TextPart {
  text: string;
  match: boolean;
}

const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * Splits `text` around every occurrence of `term`, ignoring case, the way the
 * search itself matches (SQL LIKE). An empty term gives the text unmarked.
 */
export const splitMatches = (text: string, term: string): TextPart[] => {
  const needle = term.trim();
  if (!needle || !text) return [{ text, match: false }];
  const parts: TextPart[] = [];
  const pattern = new RegExp(escapeRegExp(needle), "gi");
  let last = 0;
  for (const found of text.matchAll(pattern)) {
    const start = found.index ?? 0;
    if (start > last) parts.push({ text: text.slice(last, start), match: false });
    parts.push({ text: found[0], match: true });
    last = start + found[0].length;
  }
  if (last < text.length) parts.push({ text: text.slice(last), match: false });
  return parts.length ? parts : [{ text, match: false }];
};

export const hasMatch = (parts: TextPart[]) => parts.some((part) => part.match);

/** Start close to the first hit so it fits in a short card preview. */
export const matchedExcerpt = (text: string, term: string): TextPart[] | null => {
  const needle = term.trim();
  if (!needle) return null;
  const index = text.search(new RegExp(escapeRegExp(needle), "i"));
  if (index < 0) return null;
  const lineStart = index === 0 ? 0 : text.lastIndexOf("\n", index - 1) + 1;
  const start = Math.max(lineStart, index - 24);
  return splitMatches(text.slice(start, index + needle.length + 160), needle);
};

export const fileName = (path: string) => path.trim().replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";

export const findMatchedFilePath = (paths: string[], term: string): { path: string; parts: TextPart[] | null } => {
  for (const path of paths) {
    const parts = splitMatches(fileName(path), term);
    if (hasMatch(parts)) return { path, parts };
  }
  return { path: paths[0] ?? "", parts: null };
};
