import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import type { ConfirmDialogState, Translate } from "../../shared/types";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";
import { hasOpenDialog, isField, isRecordingHotkey } from "./useKeyboard";

export interface UnsavedChangesHandle {
  hasUnsavedChanges: () => boolean;
}

interface CloseOptions {
  t: Translate;
  blocked: () => boolean;
  hasUnsavedChanges: () => boolean;
  hideWindow: () => Promise<void>;
  openConfirm: (options: Omit<ConfirmDialogState, "show">) => void;
  closeConfirm: () => void;
}

export const useWindowClose = (options: CloseOptions) => {
  const current = useRef(options);
  current.current = options;
  const composing = useRef<EventTarget | null>(null);
  const closing = useRef(false);
  const isComposing = () => composing.current !== null;

  const requestCloseWindow = () => {
    const { t, blocked, hasUnsavedChanges, hideWindow, openConfirm, closeConfirm } = current.current;
    if (closing.current || isComposing() || hasOpenDialog() || isRecordingHotkey() || blocked()) return;
    const hide = () => {
      closing.current = true;
      composing.current = null;
      const focused = document.activeElement;
      if (isField(focused)) (focused as HTMLElement).blur();
      void hideWindow().catch(console.error).finally(() => { closing.current = false; });
    };
    if (hasUnsavedChanges()) {
      openConfirm({
        title: t("close_unsaved_title"),
        message: t("close_unsaved_message"),
        confirmLabel: t("close_unsaved_discard"),
        danger: true,
        onConfirm: () => {
          closeConfirm();
          hide();
        },
      });
    } else {
      hide();
    }
  };
  const requestRef = useRef(requestCloseWindow);
  requestRef.current = requestCloseWindow;

  useEffect(() => {
    const start = (event: CompositionEvent) => { composing.current = event.target; };
    const reset = () => { composing.current = null; };
    const focusIn = (event: FocusEvent) => {
      if (event.target !== composing.current) reset();
    };
    window.addEventListener("compositionstart", start, true);
    window.addEventListener("compositionend", reset, true);
    window.addEventListener("blur", reset, true);
    window.addEventListener("focusin", focusIn, true);
    const subscriptions = isTauriRuntime() ? [
      listen("window-close-requested", () => requestRef.current()),
      listen("clipboard-shown", reset),
    ] : [];
    return () => {
      window.removeEventListener("compositionstart", start, true);
      window.removeEventListener("compositionend", reset, true);
      window.removeEventListener("blur", reset, true);
      window.removeEventListener("focusin", focusIn, true);
      subscriptions.forEach((subscription) => void subscription.then((off) => off()));
    };
  }, []);

  return { isComposing, requestCloseWindow };
};
