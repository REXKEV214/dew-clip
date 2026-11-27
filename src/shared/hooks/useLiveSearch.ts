import { useCallback, useEffect, useRef, useState } from "react";

export const useLiveSearch = (delay: number) => {
  const [draft, setDraft] = useState("");
  const [search, setSearch] = useState("");
  const timer = useRef<number | undefined>(undefined);
  const composing = useRef(false);

  const cancelTimer = useCallback(() => window.clearTimeout(timer.current), []);
  useEffect(() => cancelTimer, [cancelTimer]);

  const flush = (value: string) => {
    if (composing.current) return;
    cancelTimer();
    setSearch(value.trim());
  };

  const change = (value: string) => {
    cancelTimer();
    setDraft(value);
    if (composing.current) return;
    const term = value.trim();
    if (!term || delay === 0) setSearch(term);
    else if (term !== search) timer.current = window.setTimeout(() => setSearch(term), delay);
  };

  const clear = useCallback(() => {
    cancelTimer();
    composing.current = false;
    setDraft("");
    setSearch("");
  }, [cancelTimer]);

  return {
    draft,
    search,
    change,
    clear,
    flush,
    isComposing: () => composing.current,
    startComposition: () => {
      composing.current = true;
      cancelTimer();
    },
    endComposition: (value: string) => {
      composing.current = false;
      setDraft(value);
      flush(value);
    },
  };
};
