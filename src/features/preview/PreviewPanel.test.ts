// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import type { ClipboardEntry } from "../../shared/types";
import PreviewPanel from "./PreviewPanel";
import { previewContentRequest } from "./previewContentRequest";

it("addresses the original entry even if its ID can be reused", () => {
  expect(previewContentRequest(1, 10)).toEqual(["get_clipboard_content", { id: 1, createdAt: 10 }]);
});

it("shows the unavailable message when the full-content command fails", async () => {
  const environment = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };
  const previousActEnvironment = environment.IS_REACT_ACT_ENVIRONMENT;
  environment.IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  const entry: ClipboardEntry = {
    id: 1,
    created_at: 10,
    timestamp: 10,
    content_type: "text",
    content: "truncated text",
    preview: "truncated text",
    source_app: "",
    is_pinned: false,
  };
  try {
    await act(async () => root.render(createElement(PreviewPanel, { t: (key) => key, entry })));
    expect(container.querySelector(".preview-message")?.textContent).toBe("preview_unavailable");
    expect(container.textContent).not.toContain(entry.preview);
  } finally {
    await act(async () => root.unmount());
    environment.IS_REACT_ACT_ENVIRONMENT = previousActEnvironment;
  }
});
