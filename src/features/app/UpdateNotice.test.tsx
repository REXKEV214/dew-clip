// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test } from "vitest";
import { translations, type TranslationKey } from "../../locales";
import type { Locale } from "../../shared/types";
import { UpdateBanner, UpdateDialog, UpdateSettings } from "./UpdateNotice";
import { useUpdateDialogTarget, type DownloadStatus, type InstallError, type InstallPreview, type UpdateSnapshot } from "./useUpdateStatus";

const latest = {
  version: "1.2.0",
  releaseUrl: "https://github.com/REXKEV214/dew-clip/releases/tag/v1.2.0",
  notesPreview: "New release\n<img src=invalid>",
};
const status: UpdateSnapshot = {
  currentVersion: "1.1.3", checking: false, status: "available", latest,
  lastCheckedAt: 1, skippedVersion: null, available: true, bannerDismissed: false,
  errorKind: null, retryAfterSecs: null,
  ready: null, download: null, incompleteInstall: false,
  noticeVersion: latest.version,
};
const downloadable = { ...latest, asset: { size: 4_683_663 } };
const downloading: DownloadStatus = { state: "downloading", version: latest.version, received: 50, total: 100 };
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

test.each(["en", "zh"] as const)("the banner can be viewed or dismissed in %s and stays quiet on the first download", (language) => {
  const t = localized(language);
  const actions: string[] = [];
  const render = (snapshot: UpdateSnapshot, showSettings = false) => act(() => root.render(
    <UpdateBanner t={t} status={snapshot} showSettings={showSettings}
      onView={(release) => actions.push(release.releaseUrl)}
      onDismiss={(version) => {
        actions.push(version);
        render({ ...snapshot, bannerDismissed: true });
      }} />
  ));
  render(status);
  expect(host.textContent).toContain(t("update_available").replace("{version}", latest.version));
  act(() => button(t("update_view"))!.click());
  act(() => host.querySelector<HTMLButtonElement>(`[aria-label="${t("update_dismiss")}"]`)!.click());
  expect(actions).toEqual([latest.releaseUrl, latest.version]);
  expect(host.querySelector(".update-banner")).toBeNull();
  render({ ...status, available: false, skippedVersion: latest.version });
  expect(host.querySelector(".update-banner")).toBeNull();
  render(status, true);
  expect(host.querySelector(".update-banner")).toBeNull();
  render({ ...status, available: false, latest: downloadable, download: downloading });
  expect(host.querySelector(".update-banner")).toBeNull();
  render({ ...status, ready: downloadable });
  expect(host.textContent).toContain(language === "en"
    ? "DewClip 1.2.0 is downloaded and ready to install"
    : "DewClip 1.2.0 已下载，可以安装");
  const newer = { ...downloadable, version: "1.3.0" };
  render({ ...status, latest: newer, ready: downloadable, noticeVersion: newer.version,
    download: { state: "deferred", version: newer.version, reason: "slow" } });
  expect(host.textContent).toContain(t("update_available").replace("{version}", newer.version));
});

test.each(["en", "zh"] as const)("the dialog offers the release page when there is no asset, with plain-text notes in %s", async (language) => {
  const actions: string[] = [];
  const t = localized(language);
  act(() => root.render(
    <UpdateDialog t={t} currentVersion={status.currentVersion} release={latest} ready={false} download={null} incomplete={false} preview={null}
      onLater={() => actions.push("later")} onSkip={async () => { actions.push("skip"); }}
      onOpenReleasePage={(page) => actions.push(page)} onDownload={noAction} onInstall={async () => null} />
  ));
  const dialog = document.querySelector<HTMLElement>('[aria-modal="true"]')!;
  expect(dialog.textContent).toContain(t("update_versions").replace("{current}", "1.1.3").replace("{latest}", "1.2.0"));
  expect(dialog.textContent).not.toContain(t("update_install_regrant"));
  expect(dialog.textContent).toContain(t("update_install_reminder"));
  expect(button(t("update_download_now"), dialog)).toBeUndefined();
  expect(dialog.querySelector(".update-notes")?.textContent).toContain("<img src=invalid>");
  expect(dialog.querySelector("img")).toBeNull();
  await act(async () => {
    button(t("update_download"), dialog)!.click();
    button(t("update_skip"), dialog)!.click();
    button(t("update_later"), dialog)!.click();
  });
  expect(actions).toEqual([latest.releaseUrl, "skip", "later"]);
});

test.each(["en", "zh"] as const)("the dialog offers install, retry and manual instructions for the corresponding states in %s", async (language) => {
  const t = localized(language);
  const downloads: string[] = [];
  const installs: Array<[string, boolean | null]> = [];
  const render = (ready: boolean, preview: InstallPreview | InstallError | null, incomplete = false, download: DownloadStatus | null = null) => {
    act(() => root.render(
      <UpdateDialog t={t} currentVersion={status.currentVersion} release={downloadable} ready={ready}
        preview={preview} incomplete={incomplete} download={download} onLater={noAction} onSkip={async () => {}}
        onDownload={(version) => downloads.push(version)} onOpenReleasePage={noAction}
        onInstall={async (version, automatic) => { installs.push([version, automatic]); return null; }} />
    ));
    return document.querySelector<HTMLElement>('[aria-modal="true"]')!;
  };
  let dialog = render(false, null);
  act(() => button(t("update_download_now"), dialog)!.click());
  expect(downloads).toEqual([latest.version]);
  expect(button(t("update_download"), dialog)).toBeDefined();
  dialog = render(false, null, false, downloading);
  expect(dialog.textContent).toContain("50%");
  expect(button(t("update_download_now"), dialog)).toBeUndefined();
  expect(dialog.textContent).not.toContain(t("update_install_reminder"));
  dialog = render(true, null, true);
  expect(button(t("update_install"), dialog)?.disabled).toBe(true);
  expect(dialog.textContent).toContain(t("update_install_incomplete"));
  dialog = render(true, { requirementMatches: true, automatic: true });
  expect(dialog.textContent).not.toContain(t("update_install_incomplete"));
  expect(dialog.textContent).not.toContain(t("update_install_regrant"));
  expect(dialog.textContent).not.toContain(t("update_install_manual"));
  expect(dialog.textContent).toContain(t("update_install_keep_accessibility"));
  dialog = render(true, { requirementMatches: true, automatic: false });
  expect(dialog.textContent).not.toContain(t("update_install_regrant"));
  expect(dialog.textContent).toContain(t("update_install_manual"));
  dialog = render(true, { requirementMatches: false, automatic: false });
  expect(dialog.textContent).toContain(t("update_install_regrant"));
  dialog = render(true, "failed");
  expect(dialog.querySelector('[role="alert"]')?.textContent).toBe(t("update_install_failed"));
  dialog = render(false, "missing");
  expect(dialog.querySelector('[role="alert"]')?.textContent).toBe(t("update_install_missing"));
  dialog = render(true, { requirementMatches: true, automatic: true });
  await act(async () => button(t("update_install"), dialog)!.click());
  expect(installs).toEqual([[latest.version, true]]);
});

test.each([
  ["en", ["Drag DewClip.app into Applications", "Open Anyway in Privacy & Security", "grant Accessibility permission again"]],
  ["zh", ["将 DewClip.app 拖入「应用程序」", "前往「隐私与安全性」选择「仍要打开」", "重新授予辅助功能权限"]],
] as const)("manual installation instructions cover deferred and failed downloads in %s", (language, instructions) => {
  for (const reason of ["slow", "failed"] as const) {
    act(() => root.render(
      <UpdateDialog t={localized(language)} currentVersion={status.currentVersion} release={downloadable}
        ready={false} download={{ state: "deferred", version: latest.version, reason }}
        incomplete={false} preview={null} onLater={noAction} onSkip={async () => {}}
        onOpenReleasePage={noAction} onDownload={noAction} onInstall={async () => null} />
    ));
    const dialog = document.querySelector<HTMLElement>('[aria-modal="true"]')!;
    for (const instruction of instructions) expect(dialog.textContent).toContain(instruction);
    expect(button(localized(language)("update_download"), dialog)).toBeDefined();
  }
});

const DialogWithLiveTarget = ({ snapshot }: { snapshot: UpdateSnapshot }) => {
  const { release, select } = useUpdateDialogTarget(snapshot);
  return (
    <>
      <button onClick={() => { if (snapshot.latest) select({ version: snapshot.latest.version }); }}>View</button>
      {release && <UpdateDialog key={release.version} t={localized("en")} currentVersion={snapshot.currentVersion}
        release={release} ready={release.version === snapshot.ready?.version} download={snapshot.download}
        incomplete={false} preview={null} onLater={() => select(null)} onSkip={async () => {}}
        onOpenReleasePage={noAction} onDownload={noAction} onInstall={async () => null} />}
    </>
  );
};

test.each([true, false])("an open dialog closes when its version is replaced (new version ready: %s)", (newReady) => {
  const old = { ...status, latest: downloadable, ready: downloadable };
  const newer = { ...downloadable, version: "1.3.0" };
  const render = (snapshot: UpdateSnapshot) => act(() => root.render(<DialogWithLiveTarget snapshot={snapshot} />));
  const dialog = () => document.querySelector<HTMLElement>('[aria-modal="true"]');
  render(old);
  act(() => button("View")!.click());
  expect(dialog()?.textContent).toContain("DewClip 1.2.0 is downloaded");
  render({ ...old, latest: newer });
  expect(dialog()?.textContent).toContain("DewClip 1.2.0 is downloaded");
  render({ ...old, latest: newer, ready: newReady ? newer : null, noticeVersion: newer.version });
  expect(dialog()).toBeNull();
  render(old);
  expect(dialog()).toBeNull();
  render({ ...old, latest: newer, ready: newReady ? newer : null, noticeVersion: newer.version });
  act(() => button("View")!.click());
  expect(dialog()?.textContent).toContain("1.3.0");
  expect(dialog()?.textContent).not.toContain("1.2.0");
});

test.each(["en", "zh"] as const)("settings show manual results and the backend version in %s", (language) => {
  const t = localized(language);
  const viewed: string[] = [];
  const render = (result: UpdateSnapshot | "failed" | null, checking = false) => act(() => root.render(
    <UpdateSettings t={t} status={result && result !== "failed" ? result : { ...status, available: false, skippedVersion: latest.version }}
      checking={checking} result={result} onCheck={noAction} onView={(release) => viewed.push(release.version)}
      onOpenReleasePage={noAction} onDownload={noAction} />
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
  render({ ...status, status: "error", errorKind: "rate_limited", retryAfterSecs: 60 });
  expect(host.textContent).toContain(language === "en" ? "Try again in about 1 minute." : "约 1 分钟后可重试。");
  render("failed");
  expect(host.textContent).toContain(t("update_failed"));
});

test.each(["en", "zh"] as const)("the footer shows progress, a ready installer and neutral download reasons in %s", (language) => {
  const t = localized(language);
  const actions: string[] = [];
  const render = (snapshot: UpdateSnapshot) => act(() => root.render(
    <UpdateSettings t={t} status={snapshot} result={null} checking={false} onCheck={noAction} onView={noAction}
      onDownload={(version) => actions.push(version)} onOpenReleasePage={(url) => actions.push(url)} />
  ));
  render({ ...status, latest: downloadable, download: downloading, available: false });
  expect(host.textContent).toContain(t("update_downloading").replace("{version}", latest.version).replace("{percent}", "50"));
  expect(button(t("update_view_details"))).toBeUndefined();
  render({ ...status, latest: downloadable, ready: downloadable });
  expect(host.textContent).toContain(t("update_ready").replace("{version}", latest.version));
  for (const reason of ["slow", "restricted", "space", "storage", "failed"] as const) {
    render({ ...status, latest: downloadable, download: { state: "deferred", version: latest.version, reason } });
    expect(host.textContent).toContain(t(`update_download_${reason}`));
    expect(button(t("update_download_now"))).toBeDefined();
    expect(button(t("update_download"))).toBeDefined();
  }
  act(() => button(t("update_download_now"))!.click());
  act(() => button(t("update_download"))!.click());
  expect(actions).toEqual([latest.version, latest.releaseUrl]);
});
