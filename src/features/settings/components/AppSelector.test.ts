// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import AppSelector from "./AppSelector";

it("keeps every installed app available when macOS recommendations are unavailable", async () => {
  const environment = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };
  const previousActEnvironment = environment.IS_REACT_ACT_ENVIRONMENT;
  environment.IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  const props = {
    type: "image",
    installedApps: [
      { label: "Photo Booth", value: "/System/Applications/Photo Booth.app" },
      { label: "Terminal", value: "/System/Applications/Utilities/Terminal.app" },
    ],
    t: (key: string) => key,
    onSelect: () => {},
  };
  try {
    await act(async () => root.render(createElement(AppSelector, props)));
    expect(container.querySelector(".app-selector-section-label")?.textContent).toBe("all_apps");
    expect([...container.querySelectorAll('[role="option"]')].map((app) => app.textContent))
      .toEqual(["Photo Booth", "Terminal"]);

    await act(async () => root.render(createElement(AppSelector, {
      ...props,
      mode: "pick-app",
      excludePaths: ["/system/applications/photo booth.app/"],
    })));
    expect([...container.querySelectorAll('[role="option"]')].map((app) => app.textContent)).toEqual(["Terminal"]);
  } finally {
    await act(async () => root.unmount());
    environment.IS_REACT_ACT_ENVIRONMENT = previousActEnvironment;
  }
});
