// @vitest-environment happy-dom
import { act, createElement, Fragment } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { useSettings } from "../../../app/useSettings";
import { useOverlays } from "../../../app/useOverlays";
import { ConfirmDialog } from "../../../../shared/components/Overlays";
import { StorageSettings } from "./StorageSettings";

const SettingsWithConfirmation = () => {
  const settings = useSettings();
  const { t } = settings;
  const { confirmDialog, openConfirm, closeConfirm, pushToast } = useOverlays(t);
  return createElement(Fragment, null,
    createElement(StorageSettings, { settings, pushToast, openConfirm, closeConfirm }),
    createElement(ConfirmDialog, {
      state: confirmDialog,
      confirmLabel: t("confirm"),
      cancelLabel: t("cancel"),
      onClose: closeConfirm,
    }),
  );
};

it("asks before tightening either rule and preserves both settings when cancelled", async () => {
  const environment = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };
  const previousActEnvironment = environment.IS_REACT_ACT_ENVIRONMENT;
  environment.IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  const button = (role: string, text: string) => {
    const element = [...document.querySelectorAll<HTMLButtonElement>(role)].find((item) => item.textContent === text);
    if (!element) throw new Error(`Missing button: ${text}`);
    return element;
  };
  try {
    await act(async () => root.render(createElement(SettingsWithConfirmation)));
    for (const option of ["7 days", "500 MB"]) {
      await act(async () => button('button[role="radio"]', option).click());
      const dialog = document.querySelector('[role="dialog"]');
      expect(dialog?.textContent).toContain("Apply Cleanup Rule");
      expect(dialog?.textContent).toContain("Pinned items, tagged items, and favorites");
      await act(async () => button('[role="dialog"] button', "Cancel").click());
      expect(document.querySelector('[role="dialog"]')).toBeNull();
      expect(button('button[role="radio"]', "Forever").getAttribute("aria-checked")).toBe("true");
      expect(button('button[role="radio"]', "Unlimited").getAttribute("aria-checked")).toBe("true");
    }
  } finally {
    await act(async () => root.unmount());
    container.remove();
    environment.IS_REACT_ACT_ENVIRONMENT = previousActEnvironment;
  }
});
