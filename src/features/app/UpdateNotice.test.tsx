// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test } from "vitest";
import { translations, type TranslationKey } from "../../locales";
import type { Locale } from "../../shared/types";
import { UpdateBanner, UpdateDialog, UpdateSettings } from "./UpdateNotice";
import type { UpdateSnapshot } from "./useUpdateStatus";

const latest = {
  version: "1.2.0",
  releaseUrl: "https://github.com/REXKEV214/dew-clip/releases/tag/v1.2.0",
  notesPreview: "New release\n<img src=invalid>",
};
const status: UpdateSnapshot = {
  currentVersion: "1.1.3", checking: false, status: "available", latest,
  lastCheckedAt: 1, skippedVersion: null, available: true, bannerDismissed: false,
  errorKind: null, retryAfterSecs: null,
};
const localized = (locale: Locale) => (key: string) => translations[locale][key as TranslationKey];
const noAction = () => {};
let host: HTMLDivElement;
let root: Root;

const button = (label: string, element: ParentNode = host) => Array.from(element.querySelectorAll("button"))
  .find((button) => button.textContent?.trim() === label);

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: false });
});

test("the banner can be viewed or dismissed and appears only on the main list for an available release", () => {
  const actions: string[] = [];
  const render = (snapshot: UpdateSnapshot, showSettings = false) => act(() => root.render(
    <UpdateBanner t={localized("en")} status={snapshot} showSettings={showSettings}
      onView={(release) => actions.push(release.releaseUrl)}
      onDismiss={(version) => {
        actions.push(version);
        render({ ...snapshot, bannerDismissed: true });
      }} />
  ));
  render(status);
  expect(host.textContent).toContain("Update available: v1.2.0");
  act(() => button("View")!.click());
  act(() => host.querySelector<HTMLButtonElement>('[aria-label="Dismiss update banner"]')!.click());
  expect(actions).toEqual([latest.releaseUrl, latest.version]);
  expect(host.querySelector(".update-banner")).toBeNull();
  render({ ...status, available: false, skippedVersion: latest.version });
  expect(host.querySelector(".update-banner")).toBeNull();
  render(status, true);
  expect(host.querySelector(".update-banner")).toBeNull();
});

test("the dialog shows the versions, plain text and reauthorization instructions, and offers download, skip and later", async () => {
  const actions: string[] = [];
  const t = localized("en");
  act(() => root.render(
    <UpdateDialog t={t} currentVersion={status.currentVersion} release={latest}
      onLater={() => actions.push("later")} onSkip={async () => { actions.push("skip"); }}
      onOpenReleasePage={(page) => actions.push(page)} />
  ));
  const dialog = document.querySelector<HTMLElement>('[aria-modal="true"]')!;
  expect(dialog.textContent).toContain("Current version v1.1.3 → Latest version v1.2.0");
  expect(dialog.textContent).toContain("grant access again");
  expect(dialog.querySelector(".update-notes")?.textContent).toContain("<img src=invalid>");
  expect(dialog.querySelector("img")).toBeNull();
  await act(async () => {
    button("Go to Download", dialog)!.click();
    button("Skip This Version", dialog)!.click();
    button("Later", dialog)!.click();
  });
  expect(actions).toEqual([latest.releaseUrl, "skip", "later"]);
});

test.each(["en", "zh"] as const)("settings show manual results and the backend version in %s", (language) => {
  const t = localized(language);
  const viewed: string[] = [];
  const render = (result: UpdateSnapshot | "failed" | null, checking = false) => act(() => root.render(
    <UpdateSettings t={t} status={{ ...status, available: false, skippedVersion: latest.version }}
      checking={checking} result={result} onCheck={noAction} onView={(release) => viewed.push(release.version)}
      onOpenReleasePage={noAction} />
  ));
  render(null, true);
  expect(host.querySelector(".brand-version")?.textContent).toBe("1.1.3");
  expect(button(t("update_checking"))?.disabled).toBe(true);
  render({ ...status, status: "up_to_date", latest: null, available: false });
  expect(host.textContent).toContain(t("update_up_to_date").replace("{version}", "1.1.3"));
  render({ ...status, available: false, skippedVersion: latest.version });
  expect(host.textContent).toContain(t("update_available").replace("{version}", latest.version));
  act(() => button(t("update_view_details"))!.click());
  expect(viewed).toEqual([latest.version]);
  for (const errorKind of ["offline", "timeout", "rate_limited", "failed"] as const) {
    render({ ...status, status: "error", errorKind, retryAfterSecs: errorKind === "rate_limited" ? 65 : null });
    expect(host.textContent).toContain(t(`update_${errorKind}`));
    expect(button(t("update_open_releases"))).toBeDefined();
    if (errorKind === "rate_limited") expect(host.textContent).toContain(t("update_retry_minutes").replace("{minutes}", "2"));
  }
  render("failed");
  expect(host.textContent).toContain(t("update_failed"));
});
