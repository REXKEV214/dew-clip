import { Check, X } from "lucide-react";
import { useState } from "react";
import AppModal from "../../shared/components/AppModal";
import { GITHUB_URL } from "../../shared/config";
import type { Translate } from "../../shared/types";
import { installError, noticedRelease, type DownloadStatus, type InstallError, type InstallPreview, type Release, type UpdateSnapshot } from "./useUpdateStatus";

const releaseTitle = (t: Translate, version: string, ready: boolean) =>
  t(ready ? "update_ready" : "update_available").replace("{version}", version);

const downloadProgress = (t: Translate, download: Extract<DownloadStatus, { state: "downloading" }>) =>
  t("update_downloading").replace("{version}", download.version)
    .replace("{percent}", String(Math.floor(100 * download.received / download.total)));

export const UpdateBanner = ({ t, status, showSettings, onView, onDismiss }: {
  t: Translate;
  status: UpdateSnapshot | null;
  showSettings: boolean;
  onView: (release: Release) => void;
  onDismiss: (version: string) => void;
}) => {
  const release = noticedRelease(status);
  if (showSettings || status?.bannerDismissed || !release) return null;
  const { version } = release;
  return (
    <div className="update-banner" role="status">
      <span>{releaseTitle(t, version, status?.ready?.version === version)}</span>
      <button type="button" className="link-button" onClick={() => onView(release)}>{t("update_view")}</button>
      <button type="button" className="icon-button" aria-label={t("update_dismiss")} onClick={() => onDismiss(version)}>
        <X size={13} aria-hidden="true" />
      </button>
    </div>
  );
};

export const UpdateDialog = ({
  t, currentVersion, release, ready, download, incomplete, preview, onLater, onSkip, onOpenReleasePage, onDownload, onInstall,
}: {
  t: Translate;
  currentVersion: string;
  release: Release;
  ready: boolean;
  download: DownloadStatus | null;
  incomplete: boolean;
  preview: InstallPreview | InstallError | null;
  onLater: () => void;
  onSkip: () => Promise<void>;
  onOpenReleasePage: (url: string) => void;
  onDownload: (version: string) => void;
  onInstall: (version: string, confirmedAutomatic: boolean | null) => Promise<InstallPreview | null>;
}) => {
  const { version } = release;
  const [failure, setFailure] = useState<InstallError | null>(null);
  const [installing, setInstalling] = useState(false);
  const install = async () => {
    setInstalling(true);
    setFailure(null);
    try {
      const revised = await onInstall(version, verified?.automatic ?? null);
      if (revised) setInstalling(false);
    } catch (error) {
      setFailure(installError(error));
      setInstalling(false);
    }
  };
  const title = releaseTitle(t, version, ready);
  const openRelease = () => onOpenReleasePage(release.releaseUrl);
  const progress = download?.state === "downloading" && download.version === version ? download : null;
  const deferred = download?.state === "deferred" && download.version === version ? download : null;
  const previewError = typeof preview === "string" ? preview : null;
  const verified = ready && preview && typeof preview !== "string" ? preview : null;
  const problem = verified && (failure === "missing" || failure === "invalid") ? null : failure ?? previewError;
  return (
    <AppModal open onClose={() => { if (!installing) onLater(); }} panelClassName="modal-panel glass-popover update-dialog" ariaLabel={title}>
      <h3 className="modal-title">{title}</h3>
      <p className="modal-message">
        {t("update_versions").replace("{current}", currentVersion).replace("{latest}", release.version)}
      </p>
      {release.notesPreview && <p className="update-notes">{release.notesPreview}</p>}
      <button type="button" className="link-button" onClick={openRelease}>{t("update_release_notes")}</button>
      {incomplete && <p className="update-install-reminder">{t("update_install_incomplete")}</p>}
      {ready && !preview && <p role="status">{t("update_install_checking")}</p>}
      {verified && !verified.automatic && <p className="update-install-reminder">{t("update_install_manual")}</p>}
      {verified?.automatic && <p className="update-install-reminder">{t("update_install_keep_accessibility")}</p>}
      {verified && !verified.requirementMatches && <p className="update-install-reminder">{t("update_install_regrant")}</p>}
      {problem && <p className="update-install-reminder" role="alert">
        {t(problem === "failed" ? "update_install_failed" : "update_install_missing")}
      </p>}
      {!ready && !progress && <p className="update-install-reminder">{t("update_install_reminder")}</p>}
      {progress && <p role="status">{downloadProgress(t, progress)}</p>}
      {deferred && <p role="status">{t(`update_download_${deferred.reason}`)}</p>}
      <div className="modal-actions">
        <button type="button" className="glass-button is-compact" data-autofocus disabled={installing} onClick={onLater}>
          {t("update_later")}
        </button>
        <button type="button" className="glass-button is-compact" disabled={installing} onClick={() => void onSkip()}>{t("update_skip")}</button>
        {ready ? (
          <button type="button" className="glass-button is-compact is-primary" disabled={installing || (!verified && !previewError)}
            onClick={() => void install()}>{t(installing ? "update_installing" : "update_install")}</button>
        ) : (
          <>
            {release.asset && !progress && <button type="button" className="glass-button is-compact is-primary"
              onClick={() => onDownload(version)}>{t("update_download_now")}</button>}
            <button type="button" className={`glass-button is-compact${release.asset ? "" : " is-primary"}`} onClick={openRelease}>
              {t("update_download")}
            </button>
          </>
        )}
        {ready && previewError === "failed" && <button type="button" className="glass-button is-compact" onClick={openRelease}>
          {t("update_download")}
        </button>}
      </div>
    </AppModal>
  );
};

export const UpdateSettings = ({ t, status, checking, result, onCheck, onView, onOpenReleasePage, onDownload }: {
  t: Translate;
  status: UpdateSnapshot | null;
  checking: boolean;
  result: UpdateSnapshot | "failed" | null;
  onCheck: () => void;
  onView: (release: Release) => void;
  onOpenReleasePage: (url: string) => void;
  onDownload: (version: string) => void;
}) => {
  const report = result === "failed" ? null : result;
  const error = result === "failed" ? "failed" : report?.errorKind;
  const latest = status?.ready ?? (error ? null : noticedRelease(status)
    ?? (report?.latest?.version === status?.latest?.version ? status?.latest : null));
  const download = status?.download;
  const deferredRelease = download?.state === "deferred" && status?.latest?.version === download.version ? status.latest : null;
  const retryMinutes = Math.ceil((report?.retryAfterSecs ?? 0) / 60);
  return (
    <>
      {status && <div className="brand-version">{status.currentVersion}</div>}
      <button type="button" className="glass-button is-compact" disabled={!status || checking} onClick={onCheck}>
        {t(checking ? "update_checking" : "update_check")}
      </button>
      <div className="update-settings-result" role="status">
        {error && (
          <>
            <span>{t(`update_${error}`)}</span>
            {error === "rate_limited" && retryMinutes > 0 && (
              <span>{t(retryMinutes === 1 ? "update_retry_minute" : "update_retry_minutes")
                .replace("{minutes}", String(retryMinutes))}</span>
            )}
            <button type="button" className="link-button" onClick={() => onOpenReleasePage(`${GITHUB_URL}/releases`)}>
              {t("update_open_releases")}
            </button>
          </>
        )}
        {latest && (download?.version !== latest.version || download.state !== "downloading") && (
          <>
            <span>{releaseTitle(t, latest.version, latest.version === status?.ready?.version)}</span>
            <button type="button" className="link-button" onClick={() => onView(latest)}>{t("update_view_details")}</button>
          </>
        )}
        {download?.state === "downloading" && <span>{downloadProgress(t, download)}</span>}
        {deferredRelease && download?.state === "deferred" && (
          <>
            <span>{t(`update_download_${download.reason}`)}</span>
            <button type="button" className="link-button" onClick={() => onDownload(deferredRelease.version)}>
              {t("update_download_now")}
            </button>
            <button type="button" className="link-button" onClick={() => onOpenReleasePage(deferredRelease.releaseUrl)}>
              {t("update_download")}
            </button>
          </>
        )}
        {report?.status === "up_to_date" && !latest && !download && (
          <span className="update-up-to-date">
            <Check size={13} aria-hidden="true" />
            {t("update_up_to_date").replace("{version}", report.currentVersion)}
          </span>
        )}
      </div>
    </>
  );
};
