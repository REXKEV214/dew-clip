import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ClipboardEntry } from "../../shared/types";
import { entryKey, type EntryKey, type TypeFilter } from "../../shared/lib/clipboardTypes";
import { indexOfEntry } from "../batch/batchSelection";

export const useHistorySelection = (
  items: ClipboardEntry[],
  search: string,
  typeFilter: TypeFilter | null,
  selectFirst: boolean
) => {
  // Keep the entry picked when copies shift its position in the list.
  const [selectedKey, setSelectedKey] = useState<EntryKey | null>(null);
  const selectedIndex = useMemo(() => indexOfEntry(items, selectedKey), [items, selectedKey]);
  const itemsRef = useRef(items);
  itemsRef.current = items;

  const setSelectedIndex = useCallback((index: number | null) => {
    const entry = index === null ? undefined : itemsRef.current[index];
    setSelectedKey(entry ? entryKey(entry) : null);
  }, []);
  const clearSelection = useCallback(() => setSelectedKey(null), []);

  useLayoutEffect(clearSelection, [clearSelection, search, typeFilter]);
  useLayoutEffect(() => {
    if (selectedIndex !== null) return;
    const first = selectFirst ? items[0] : undefined;
    if (first || selectedKey) setSelectedKey(first ? entryKey(first) : null);
  }, [items, selectFirst, selectedIndex, selectedKey]);

  return { selectedIndex, setSelectedIndex, clearSelection };
};
