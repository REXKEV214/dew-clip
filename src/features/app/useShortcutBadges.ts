import { useEffect, useState } from "react";

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
    const keyDown = (event: KeyboardEvent) => {
      if (event.key === "Meta" && !event.ctrlKey && !event.altKey && !event.shiftKey) {
        if (!event.repeat && timer === undefined) {
          timer = window.setTimeout(() => setVisible(true), COMMAND_HOLD_MS);
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
    return () => {
      hide();
      window.removeEventListener("keydown", keyDown, true);
      window.removeEventListener("keyup", keyUp, true);
      window.removeEventListener("blur", hide);
    };
  }, [enabled]);

  return enabled && visible;
};
