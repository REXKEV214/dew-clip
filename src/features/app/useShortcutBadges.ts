import { useEffect, useState } from "react";
import { hasOpenDialog, isField } from "./useKeyboard";

const COMMAND_HOLD_MS = 300;

export const useShortcutBadges = (enabled: boolean) => {
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    if (!enabled) return;
    let timer: number | undefined;
    const hide = () => {
      window.clearTimeout(timer);
      timer = undefined;
      setVisible(false);
    };
    const blocked = () => {
      const focused = document.activeElement;
      return hasOpenDialog() || (isField(focused) && !focused?.classList.contains("searchbox-input"));
    };
    const hideIfBlocked = () => {
      if (timer !== undefined && blocked()) hide();
    };
    const keyDown = (event: KeyboardEvent) => {
      if (event.key === "Meta" && !event.ctrlKey && !event.altKey && !event.shiftKey && !blocked()) {
        if (!event.repeat && timer === undefined) {
          timer = window.setTimeout(() => {
            if (blocked()) hide();
            else setVisible(true);
          }, COMMAND_HOLD_MS);
        }
      } else {
        hide();
      }
    };
    const keyUp = (event: KeyboardEvent) => {
      if (event.key === "Meta") hide();
    };
    window.addEventListener("keydown", keyDown, true);
    window.addEventListener("keyup", keyUp, true);
    window.addEventListener("blur", hide);
    window.addEventListener("focusin", hideIfBlocked);
    const dialogs = new MutationObserver(hideIfBlocked);
    dialogs.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["aria-modal"] });
    return () => {
      hide();
      window.removeEventListener("keydown", keyDown, true);
      window.removeEventListener("keyup", keyUp, true);
      window.removeEventListener("blur", hide);
      window.removeEventListener("focusin", hideIfBlocked);
      dialogs.disconnect();
    };
  }, [enabled]);

  return enabled && visible;
};
