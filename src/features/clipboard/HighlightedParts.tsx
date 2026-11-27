import type { TextPart } from "./searchHighlight";

export const HighlightedParts = ({ parts }: { parts: TextPart[] }) => parts.map((part, index) =>
  part.match ? <mark key={index} className="search-hit">{part.text}</mark> : part.text
);
