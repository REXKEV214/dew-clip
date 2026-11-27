import { forwardRef, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { ChevronLeft, Search, X } from "lucide-react";
import type { Translate } from "../../shared/types";

/** Where the letters rest as a placeholder, from the box's left edge. */
const LETTERS_REST_X = 38;
const BUTTON_PADDING = 22;
const BUTTON_INSET = 3;
/** Extra button width for the ‹ in front of the letters. */
const ARROW_ROOM = 12;

export interface SearchBoxHandle {
  focus: () => void;
}

interface SearchBoxProps {
  t: Translate;
  value: string;
  onChange: (value: string) => void;
  onSubmit: (value: string) => void;
  onClear: () => void;
  onCompositionStart: () => void;
  onCompositionEnd: (value: string) => void;
  pending: boolean;
}

/**
 * On focus the input shortens, the label slides right one letter at a time,
 * and a blue button grows to catch it.
 */
const SearchBox = forwardRef<SearchBoxHandle, SearchBoxProps>(({
  t, value, onChange, onSubmit, onClear, onCompositionStart, onCompositionEnd, pending,
}, ref) => {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const formRef = useRef<HTMLFormElement | null>(null);
  const lettersRef = useRef<HTMLSpanElement | null>(null);
  const [focused, setFocused] = useState(false);
  const [{ boxWidth, lettersWidth }, setWidths] = useState({ boxWidth: 0, lettersWidth: 0 });
  const active = focused || value.length > 0;
  const label = t("search_placeholder");
  const letters = Array.from(label);

  useImperativeHandle(ref, () => ({
    focus: () => {
      inputRef.current?.focus();
      inputRef.current?.select();
    },
  }), []);

  useLayoutEffect(() => {
    const form = formRef.current;
    const letters = lettersRef.current;
    if (!form || !letters) return;
    const measure = () => setWidths({ boxWidth: form.clientWidth, lettersWidth: letters.offsetWidth });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(form);
    observer.observe(letters);
    return () => observer.disconnect();
  }, [label]);

  // Nothing new to search for: the button gets a ‹ and takes the box back to rest.
  const canCollapse = active && !pending;
  const collapse = () => {
    onClear();
    inputRef.current?.blur();
  };

  const buttonWidth = lettersWidth + BUTTON_PADDING + (canCollapse ? ARROW_ROOM : 0);
  const lettersLeft = boxWidth - BUTTON_INSET - lettersWidth - BUTTON_PADDING / 2;
  const shift = Math.max(0, lettersLeft - LETTERS_REST_X);

  return (
    <form
      ref={formRef}
      className={`searchbox${active ? " is-active" : ""}${canCollapse ? " can-collapse" : ""}`}
      style={{ "--search-shift": `${shift}px`, "--search-button-width": `${buttonWidth}px` } as CSSProperties}
      onSubmit={(event) => {
        event.preventDefault();
        if (pending) onSubmit(value);
      }}
    >
      <span className="searchbox-icon" aria-hidden="true">
        <Search size={15} />
      </span>
      <input
        ref={inputRef}
        className="searchbox-input"
        type="text"
        aria-label={label}
        autoComplete="off"
        autoCorrect="off"
        spellCheck={false}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onCompositionStart={onCompositionStart}
        onCompositionEnd={(event) => onCompositionEnd(event.currentTarget.value)}
      />
      {value && (
        <button
          type="button"
          className="searchbox-clear"
          title={t("search_clear")}
          aria-label={t("search_clear")}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            onClear();
            inputRef.current?.focus();
          }}
        >
          <X size={12} />
        </button>
      )}
      <button
        type={canCollapse ? "button" : "submit"}
        className="searchbox-button"
        tabIndex={active ? 0 : -1}
        aria-label={canCollapse ? t("search_close") : label}
        title={canCollapse ? t("search_close") : undefined}
        // Keeps the input focused, so the box doesn't start closing mid-click.
        onMouseDown={(event) => event.preventDefault()}
        onClick={canCollapse ? collapse : undefined}
      >
        <ChevronLeft className="searchbox-arrow" size={13} strokeWidth={2.6} aria-hidden="true" />
      </button>
      <span ref={lettersRef} className="searchbox-letters" aria-hidden="true">
        {letters.map((letter, index) => (
          <span
            key={index}
            style={{ transitionDelay: `${(active ? letters.length - 1 - index : index) * 45}ms` }}
          >
            {letter}
          </span>
        ))}
      </span>
    </form>
  );
});

SearchBox.displayName = "SearchBox";

export default SearchBox;
