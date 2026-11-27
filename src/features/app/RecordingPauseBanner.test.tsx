// @vitest-environment happy-dom
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { expect, test } from "vitest";
import RecordingPauseBanner, { recordingPauseMessage } from "./RecordingPauseBanner";
import { translations, type TranslationKey } from "../../locales";

const t = (key: string) => translations.en[key as TranslationKey];

test("the pause message shows manual, timed and under-one-minute states", () => {
  expect(recordingPauseMessage(t, null, 0)).toBe("Clipboard recording paused");
  expect(recordingPauseMessage(t, 120_000, 0)).toBe("Recording paused · ~2 min left");
  expect(recordingPauseMessage(t, 120_000, 61_000)).toBe("Recording paused · <1 min left");
});

test("the banner exposes a working Resume button", () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  let resumed = false;
  flushSync(() => root.render(<RecordingPauseBanner t={t} resumeAt={null} onResume={() => { resumed = true; }} />));
  try {
    expect(container.querySelector("span")?.textContent).toBe("Clipboard recording paused");
    const button = container.querySelector("button");
    expect(button?.textContent).toBe("Resume");
    button?.click();
    expect(resumed).toBe(true);
  } finally {
    flushSync(() => root.unmount());
  }
});
