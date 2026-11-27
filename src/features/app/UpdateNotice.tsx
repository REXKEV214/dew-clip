import { Check, X } from "lucide-react";
import AppModal from "../../shared/components/AppModal";
import { GITHUB_URL } from "../../shared/config";
import type { Translate } from "../../shared/types";
import type { Release, UpdateSnapshot } from "./useUpdateStatus";

const releaseTitle = (t: Translate, version: string) => t("update_available").replace("{version}", version);

export const UpdateBanner = ({ t, status, showSettings, onView, onDismiss }: {
  t: Translate;
  status: UpdateSnapshot | null;
  showSettings: boolean;
  onView: (release: Release) => void;
  onDismiss: (version: string) => void;
}) => {
  if (showSettings || !status?.available || status.bannerDismissed || !status.latest) return null;
  const release = status.latest;
  const { version } = release;
  return (
    <div className="update-banner" role="status">
      <span>{releaseTitle(t, version)}</span>
      <button type="button" className="link-button" onClick={() => onView(release)}>{t("update_view")}</button>
      <button type="button" className="icon-button" aria-label={t("update_dismiss")} onClick={() => onDismiss(version)}>
        <X size={13} aria-hidden="true" />
      </button>
    </div>
  );
};

export const UpdateDialog = ({ t, currentVersion, release, onLater, onSkip, onOpenReleasePage }: {
  t: Translate;
  currentVersion: string;
  release: Release;
  onLater: () => void;
  onSkip: () => Promise<void>;
  onOpenReleasePage: (url: string) => void;
}) => {
  const title = releaseTitle(t, release.version);
  const openRelease = () => onOpenReleasePage(release.releaseUrl);
  return (
    <AppModal open onClose={onLater} panelClassName="modal-panel glass-popover update-dialog" ariaLabel={title}>
      <h3 className="modal-title">{title}</h3>
      <p className="modal-message">
        {t("update_versions").replace("{current}", currentVersion).replace("{latest}", release.version)}
      </p>
      {release.notesPreview && <p className="update-notes">{release.notesPreview}</p>}
      <button type="button" className="link-button" onClick={openRelease}>{t("update_release_notes")}</button>
      <p className="update-install-reminder">{t("update_install_reminder")}</p>
      <div className="modal-actions">
        <button type="button" className="glass-button is-compact" data-autofocus onClick={onLater}>
          {t("update_later")}
        </button>
        <button type="button" className="glass-button is-compact" onClick={() => void onSkip()}>{t("update_skip")}</button>
        <button type="button" className="glass-button is-compact is-primary" onClick={openRelease}>
          {t("update_download")}
        </button>
      </div>
    </AppModal>
  );
};

export const UpdateSettings = ({ t, status, checking, result, onCheck, onView, onOpenReleasePage }: {
  t: Translate;
  status: UpdateSnapshot | null;
  checking: boolean;
  result: UpdateSnapshot | "failed" | null;
  onCheck: () => void;
  onView: (release: Release) => void;
  onOpenReleasePage: (url: string) => void;
}) => {
  const report = result === "failed" ? null : result;
  const error = result === "failed" ? "failed" : report?.errorKind;
  const latest = error ? null : report ? report.latest : status?.available ? status.latest : null;
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
        {latest && (
          <>
            <span>{releaseTitle(t, latest.version)}</span>
            <button type="button" className="link-button" onClick={() => onView(latest)}>{t("update_view_details")}</button>
          </>
        )}
        {report?.status === "up_to_date" && (
          <span className="update-up-to-date">
            <Check size={13} aria-hidden="true" />
            {t("update_up_to_date").replace("{version}", report.currentVersion)}
          </span>
        )}
      </div>
    </>
  );
};
